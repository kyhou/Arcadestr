#!/usr/bin/env python3
"""Real debug WASM, allowlisted mock IPC. Never uses a live publisher service."""
import asyncio
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

OUT=Path('/tmp/opencode')
ROOT='ab'*32
class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self,*_args): pass

async def verify(port,debug):
    with urllib.request.urlopen(f'http://127.0.0.1:{debug}/json') as response:
        page=next(p for p in json.load(response) if p.get('type')=='page' and p.get('url')=='about:blank')
    async with websockets.connect(page['webSocketDebuggerUrl'],max_size=20_000_000) as ws:
        seq=0
        async def call(method,params=None):
            nonlocal seq
            seq+=1
            await ws.send(json.dumps(dict(id=seq,method=method,params=params or {})))
            while True:
                data=json.loads(await ws.recv())
                if data.get('id')==seq:
                    assert 'error' not in data,data
                    return data.get('result',{})
        async def js(expression):
            result=await call('Runtime.evaluate',dict(expression=expression,returnByValue=True,awaitPromise=True))
            assert 'exceptionDetails' not in result,result
            return result.get('result',{}).get('value')
        async def until(expression):
            for _ in range(200):
                if await js(expression):return
                await asyncio.sleep(.05)
            raise AssertionError((expression,await js('document.body.innerText'),await js('window.calls')))
        mock=r"""window.calls=[];window.__TAURI__={core:{invoke:async(command,args)=>{
            window.calls.push({command,args});
            if(command==='publisher_distribution_state')return [
              {server_url:'https://one.test',operation_id:'original',state:'registration_acknowledged',operator:'first-operator',capabilities:['upload-build','issue-receipt'],diagnostic:null},
              {server_url:'https://two.test',operation_id:'original',state:'unknown',operator:'second-operator',capabilities:['upload-build'],diagnostic:'transport interrupted; outcome unknown'}];
            if(command==='review_publisher_credential'){
              if(window.delayReview)return await new Promise(resolve=>window.releaseReview=resolve);
              return {root_event_id:args.rootEventId,tip_event_id:args.rootEventId,coordinate:args.coordinate,subject:args.action==='revoke_grant'?'Grant fixture · one recipient':'Operator fixture · upload-build',terminal:false};
            }
            if(command==='revoke_publisher_credential'){
              if(!args.request.confirm||args.request.root_event_id!=='ROOT')throw 'Wrong credential consent';
              if(window.delayRevoke)return await new Promise((resolve,reject)=>window.rejectRevoke=reject);
              throw 'Mock relay response unavailable; exact event remains recoverable';
            }
            throw 'Forbidden command '+command;
        }}};""".replace('ROOT',ROOT)
        await call('Page.enable');await call('Runtime.enable');await call('Network.enable')
        await call('Network.setBlockedURLs',dict(urls=['https://*','wss://*','ws://*']))
        await call('Page.addScriptToEvaluateOnNewDocument',dict(source=mock))
        await call('Page.navigate',dict(url=f'http://127.0.0.1:{port}/?publisher-management-preview=1'))
        await until("document.body.innerText.includes('Unknown remote outcome')")
        assert await js("window.calls.every(c=>c.command==='publisher_distribution_state')")
        assert await js("document.body.innerText.includes('Registration acknowledged (historical upload)')&&document.body.innerText.includes('not current download availability')")
        await js("window.button=text=>Array.from(document.querySelectorAll('button')).find(b=>b.innerText===text)")
        assert await js("window.button('CONFIRM REVIEWED TRANSITION').disabled")
        await js("const field=document.querySelector('input[aria-label=\"Credential root event ID\"]');field.value='"+ROOT+"';field.dispatchEvent(new Event('input',{bubbles:true}));window.button('REVIEW CURRENT CREDENTIAL >').click()")
        await until("document.body.innerText.includes('Operator fixture')")
        await js("document.querySelector('input[type=checkbox]').click()")
        await until("!window.button('CONFIRM REVIEWED TRANSITION').disabled")
        await js("window.button('CONFIRM REVIEWED TRANSITION').click()")
        await until("document.body.innerText.includes('exact event remains recoverable')")
        assert await js("window.button('CONFIRM REVIEWED TRANSITION').disabled&&!document.querySelector('input[type=checkbox]').checked")
        await js("window.button('CONFIRM REVIEWED TRANSITION').click()")
        assert await js("window.calls.filter(c=>c.command==='revoke_publisher_credential').length===1")
        assert await js("window.calls.find(c=>c.command==='revoke_publisher_credential').args.request.action==='cancel_authority'")
        # Switching actions discards the reviewed authority, and no action is automatic.
        await js("const action=document.querySelector('select');action.value='grant';action.dispatchEvent(new Event('change',{bubbles:true}))")
        await until("!document.body.innerText.includes('Operator fixture')")
        await js("window.delayReview=true;window.button('REVIEW CURRENT CREDENTIAL >').click()")
        await until("typeof window.releaseReview==='function'")
        await js("document.querySelector('[data-testid=management-account-switch]').click()")
        await js("window.releaseReview({root_event_id:'"+ROOT+"',tip_event_id:'"+ROOT+"',coordinate:'fixture',subject:'STALE ACCOUNT REVIEW',terminal:false})")
        await until("document.querySelector('input[aria-label=\"Credential root event ID\"]').value===''")
        assert await js("!document.body.innerText.includes('STALE ACCOUNT REVIEW')&&window.button('CONFIRM REVIEWED TRANSITION').disabled")
        calls=await js('window.calls')
        # A fresh session can review a different action, but a delayed mutation
        # response must not leak into another account after explicit submission.
        await call('Page.reload')
        await until("document.body.innerText.includes('Unknown remote outcome')")
        await js("window.button=text=>Array.from(document.querySelectorAll('button')).find(b=>b.innerText===text);const action=document.querySelector('select');action.value='grant';action.dispatchEvent(new Event('change',{bubbles:true}));const field=document.querySelector('input[aria-label=\"Credential root event ID\"]');field.value='"+ROOT+"';field.dispatchEvent(new Event('input',{bubbles:true}))")
        await js("window.button('REVIEW CURRENT CREDENTIAL >').click()")
        await until("document.body.innerText.includes('Grant fixture')")
        await js("document.querySelector('input[type=checkbox]').click();window.delayRevoke=true")
        await until("!window.button('CONFIRM REVIEWED TRANSITION').disabled")
        await js("window.button('CONFIRM REVIEWED TRANSITION').click()")
        await until("typeof window.rejectRevoke==='function'")
        assert await js("window.calls.find(c=>c.command==='revoke_publisher_credential').args.request.action==='revoke_grant'")
        await js("document.querySelector('[data-testid=management-account-switch]').click();window.rejectRevoke('STALE MUTATION RESPONSE')")
        await until("document.querySelector('input[aria-label=\"Credential root event ID\"]').value===''")
        assert await js("!document.body.innerText.includes('STALE MUTATION RESPONSE')&&window.button('CONFIRM REVIEWED TRANSITION').disabled")
        # Verify the panel fits a narrow viewport without truncating safety text.
        await call('Emulation.setDeviceMetricsOverride',dict(width=390,height=844,deviceScaleFactor=1,mobile=True))
        assert await js('document.documentElement.scrollWidth<=document.documentElement.clientWidth')
        calls+=await js('window.calls')
        assert {c['command'] for c in calls}<={'publisher_distribution_state','review_publisher_credential','revoke_publisher_credential'}
        OUT.joinpath('gate6-phase5-ui-result.json').write_text(json.dumps(dict(result='PASS',mode='real WASM / mock IPC',calls=calls,live_actions=0),indent=2))
        print('PASS: historical distribution disclosure, separate revocation actions, exact reviewed consent, no blind retry, stale-account isolation, narrow viewport; mock IPC only')

if __name__=='__main__':
    root=Path(sys.argv[1]).resolve();assert root.joinpath('index.html').is_file()
    server=http.server.ThreadingHTTPServer(('127.0.0.1',0),functools.partial(Quiet,directory=str(root)))
    threading.Thread(target=server.serve_forever,daemon=True).start()
    with tempfile.TemporaryDirectory(prefix='publisher-management-chrome-',dir=str(OUT)) as profile:
        chrome=subprocess.Popen(['chromium','--headless','--no-sandbox','--disable-gpu','--remote-debugging-port=0','--user-data-dir='+profile,'about:blank'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        try:
            active=Path(profile)/'DevToolsActivePort'
            for _ in range(100):
                if active.exists():break
                time.sleep(.1)
            asyncio.run(verify(server.server_port,int(active.read_text().splitlines()[0])))
        finally:
            chrome.terminate();chrome.wait(timeout=10);server.shutdown()
