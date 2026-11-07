#!/usr/bin/env python3
"""Isolated debug WASM / mock IPC. No relay, signer, wallet or publication.
Usage: python3 -B scripts/regtest-receipt-portability-ui.py /tmp/opencode/receipt-frontend
Requires Chromium and Python websockets.
"""
import asyncio
import base64
import functools
import http.server
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
import websockets

OUT = Path('/tmp/opencode')


class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_args):
        pass


async def verify(port, debug):
    with urllib.request.urlopen(f'http://127.0.0.1:{debug}/json') as response:
        page = next(p for p in json.load(response) if p.get('type') == 'page' and p.get('url') == 'about:blank')
    async with websockets.connect(page['webSocketDebuggerUrl'], max_size=20_000_000) as ws:
        seq = 0

        async def call(method, params=None):
            nonlocal seq
            seq += 1
            await ws.send(json.dumps(dict(id=seq, method=method, params=params or {})))
            while True:
                result = json.loads(await ws.recv())
                if result.get('id') == seq:
                    assert 'error' not in result, result
                    return result.get('result', {})

        async def js(expression):
            result = await call('Runtime.evaluate', dict(expression=expression, returnByValue=True, awaitPromise=True))
            assert 'exceptionDetails' not in result, result
            return result.get('result', {}).get('value')

        async def until(expression):
            for _ in range(1200):
                if await js(expression):
                    return
                await asyncio.sleep(.1)
            raise AssertionError((expression, await js('document.body.innerText')))

        mock = """window.calls=[];window.delayImport=false;window.report={chains:[
          {order_id:'paid-order',listing_coordinate:'30402:public:game',verification:'verified',reason:'Verified',tip_event_id:null},
          {order_id:'pending-order',listing_coordinate:'30402:public:other',verification:'indeterminate',reason:'Missing predecessor',tip_event_id:null},
          {order_id:'invalid-order',listing_coordinate:'30402:public:third',verification:'invalid',reason:'Conflicting chains',tip_event_id:null}
        ],rejected_events:0,relay_failures:1,incomplete:true,message:'Available evidence checked; relay history is not guaranteed complete.'};
        window.__TAURI__={core:{invoke:async(cmd,args)=>{
          window.calls.push({cmd,args});
          if(cmd==='sync_purchase_receipts')return structuredClone(window.report);
          if(cmd==='import_purchase_receipts'){
            if(window.delayImport)return await new Promise(resolve=>window.resolveOldImport=resolve);
            return structuredClone(window.report);
          }
          throw 'Forbidden mock command: '+cmd;
        }}};"""
        await call('Page.enable')
        await call('Runtime.enable')
        await call('Network.enable')
        await call('Network.setBlockedURLs', dict(urls=['https://*']))
        await call('Page.addScriptToEvaluateOnNewDocument', dict(source=mock))
        await call('Page.navigate', dict(url=f'http://127.0.0.1:{port}/?receipt-portability-preview=1'))
        await until("!!document.querySelector('[data-testid=receipt-sync-summary]')")
        assert await js("document.body.innerText.includes('VERIFIED: 1')&&document.body.innerText.includes('INDETERMINATE: 1')&&document.body.innerText.includes('INVALID chains: 1')")
        assert await js("document.body.innerText.includes('does not guarantee recovery of every historical receipt')&&document.body.innerText.includes('independent recovery mechanism')")
        assert await js("document.querySelector('[data-testid=receipt-import-submit]').disabled")
        await js("const field=document.querySelector('[data-testid=receipt-import-input]');field.value='[{\"kind\":1020}]';field.dispatchEvent(new Event('input',{bubbles:true}));")
        await until("!document.querySelector('[data-testid=receipt-import-submit]').disabled")
        await js("document.querySelector('[data-testid=receipt-import-submit]').click()")
        await until("window.calls.some(c=>c.cmd==='import_purchase_receipts')&&!document.querySelector('[data-testid=receipt-sync-status]').innerText.includes('Synchronizing')")
        assert await js("document.querySelector('[data-testid=receipt-import-input]').value===''")
        assert await js("window.calls.find(c=>c.cmd==='import_purchase_receipts').args.request.events_json==='[{\"kind\":1020}]'")
        await js("Array.from(document.querySelectorAll('button')).find(b=>b.innerText==='RESCAN AVAILABLE HISTORY').click()")
        await until("window.calls.some(c=>c.cmd==='sync_purchase_receipts'&&c.args.request.full_rescan)")
        await until("!document.querySelector('[data-testid=receipt-sync-status]').innerText.includes('Synchronizing')")
        await js("window.delayImport=true;{const nextField=document.querySelector('[data-testid=receipt-import-input]');nextField.value='[]';nextField.dispatchEvent(new Event('input',{bubbles:true}));}")
        await until("!document.querySelector('[data-testid=receipt-import-submit]').disabled")
        await js("document.querySelector('[data-testid=receipt-import-submit]').click()")
        await until("typeof window.resolveOldImport==='function'")
        await js("document.querySelector('[data-testid=mock-receipt-account-switch]').click()")
        await until("!document.querySelector('[data-testid=receipt-sync-summary]')")
        revision = await js("document.querySelector('[data-testid=receipt-parent-revision]').innerText")
        await js("window.resolveOldImport({...window.report,message:'STALE ACCOUNT RESULT'});")
        assert await js("!document.body.innerText.includes('STALE ACCOUNT RESULT')")
        assert await js("document.querySelector('[data-testid=receipt-parent-revision]').innerText") == revision
        for width, height, name in [(1920, 1080, 'desktop'), (390, 844, 'mobile')]:
            await call('Emulation.setDeviceMetricsOverride', dict(width=width, height=height, deviceScaleFactor=1, mobile=width < 500))
            assert await js(f'document.documentElement.scrollWidth<={width}')
            shot = await call('Page.captureScreenshot', dict(format='png', captureBeyondViewport=True))
            OUT.joinpath(f'receipt-portability-{name}.png').write_bytes(base64.b64decode(shot['data']))
        calls = await js('window.calls')
        assert all(c['cmd'] in ['sync_purchase_receipts', 'import_purchase_receipts'] for c in calls)
        OUT.joinpath('receipt-portability-ui.json').write_text(json.dumps(dict(result='PASS', mode='real WASM / mock IPC only', calls=calls, live_payments=0, receipt_publications=0), indent=2))
        print('PASS receipt statuses, bounded recovery disclosure, manual import, rescan, parent refresh, account fencing, desktop/mobile UI; mock IPC only')


if __name__ == '__main__':
    root = Path(sys.argv[1]).resolve()
    assert root.joinpath('index.html').is_file(), 'Build isolated debug WASM first'
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), functools.partial(Quiet, directory=str(root)))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix='receipt-chrome-', dir=str(OUT)) as profile:
        chrome = subprocess.Popen(['chromium', '--headless', '--no-sandbox', '--disable-gpu', '--remote-debugging-port=0', '--user-data-dir=' + profile, 'about:blank'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            active = Path(profile) / 'DevToolsActivePort'
            for _ in range(100):
                if active.exists():
                    break
                time.sleep(.1)
            asyncio.run(verify(server.server_port, int(active.read_text().splitlines()[0])))
        finally:
            chrome.terminate()
            chrome.wait(timeout=10)
            server.shutdown()
