#!/usr/bin/env python3
"""Real debug WASM / allowlisted mock IPC only; no live publisher operations.

Usage: python3 -B scripts/regtest-publisher-recovery-ui.py /tmp/opencode/gate6-phase4-frontend
Uses the Chromium/CDP pattern from the existing Gate 4/receipt UI regressions.
"""
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

OUT = Path('/tmp/opencode')
OP = '22222222-2222-4222-8222-222222222222'
HASH = 'ab' * 32
BUYER = '79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798'


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
                data = json.loads(await ws.recv())
                if data.get('id') == seq:
                    assert 'error' not in data, data
                    return data.get('result', {})

        async def js(expression):
            result = await call('Runtime.evaluate', dict(expression=expression, returnByValue=True, awaitPromise=True))
            assert 'exceptionDetails' not in result, result
            return result.get('result', {}).get('value')

        async def until(expression):
            for _ in range(1000):
                if await js(expression):
                    return
                await asyncio.sleep(.05)
            raise AssertionError((expression, await js('document.body.innerText')))

        selection = dict(selection_id='11111111-1111-4111-8111-111111111111', safe_filename='fixture.zip', file_size=128, expires_at_unix=4102444800)
        counts = {key: 0 for key in ['logical_entries', 'indexed_entries', 'undecodable_entries', 'files', 'directory_entries', 'symlinks', 'hardlinks', 'other_entry_kinds', 'working_directories']}
        counts['hazards'] = {key: 0 for key in ['path_rule', 'symlink', 'hardlink', 'collision', 'other', 'undecodable', 'type_conflict', 'zip_count_mismatch']}
        counts['warnings'] = dict(efs_divergence=0, extension_mismatch=0)
        coordinate = f'30402:{BUYER}:fixture'
        state = dict(drafts=[], operations=[dict(operation_id=OP, kind='listing', coordinate=coordinate,
                     intent={}, artifact_hash=HASH, state='incomplete', result=None)],
                     checkpoints=[[OP, [dict(step_id='upload:fixture', kind='upload', target='https://operator.test',
                     intent={}, state='prepared', outcome=None, last_error='mock interrupted upload')]]])
        mock = r"""window.calls=[];window.recovery=STATE;window.selection=SELECTION;window.counts=COUNTS;
        window.__TAURI__={core:{invoke:async(command,args)=>{
          window.calls.push({command,args});
          if(command==='publisher_recovery_state')return structuredClone(window.recovery);
          if(command==='select_archive_build')return {selection:structuredClone(window.selection)};
          if(command==='inspect_archive')return {summary:{selection:structuredClone(window.selection),detected_format:'Zip',extension_format:'Zip',extension_mismatch_warning:null,counts:window.counts,blocking_hazards:false}};
          if(command==='hash_selected_archive')return {selection_id:window.selection.selection_id,sha256:'HASH'};
          if(command==='resume_publisher_operation'){
            const r=args.request;
            if(!r.confirm_resume||r.operation_id!=='OP')throw 'Missing explicit original-operation authorization';
            if(!r.selected_build||r.selected_build.sha256!=='HASH')throw 'Reselect the pinned archive';
            if(!r.confirm_uncertain_retry)throw 'Unknown upload outcome requires explicit retry acknowledgement';
            if(window.delayResume)return await new Promise(resolve=>window.resolveStale=resolve);
            window.recovery.operations[0].state='complete';return {event_id:'retained-original-event'};
          }
          throw 'Forbidden mock command: '+command;
        }}};""".replace('STATE', json.dumps(state)).replace('SELECTION', json.dumps(selection)).replace('COUNTS', json.dumps(counts)).replace('HASH', HASH).replace('OP', OP)
        await call('Page.enable')
        await call('Runtime.enable')
        await call('Network.enable')
        await call('Network.setBlockedURLs', dict(urls=['https://*', 'wss://*', 'ws://*']))
        await call('Page.addScriptToEvaluateOnNewDocument', dict(source=mock))
        await call('Page.navigate', dict(url=f'http://127.0.0.1:{port}/?publisher-recovery-preview=1'))
        await until("!!document.querySelector('summary')")
        await js("document.querySelector('summary').click()")
        await until("document.body.innerText.includes('retained operation(s)')")
        assert await js("window.calls.every(c=>c.command==='publisher_recovery_state')"), 'startup must be read-only'
        assert await js("document.body.innerText.includes('outcome unknown')&&document.body.innerText.includes('mock interrupted upload')")
        await js("window.resumeButton=()=>Array.from(document.querySelectorAll('button')).find(b=>b.innerText==='RECONCILE & RESUME ORIGINAL')")
        assert await js('window.resumeButton().disabled')
        await js("Array.from(document.querySelectorAll('button')).find(b=>b.innerText==='RESELECT PINNED ARCHIVE').click()")
        await until("document.body.innerText.includes('Reselected SHA-256')")
        await js("document.querySelectorAll('input[type=checkbox]')[0].click()")
        await until("!window.resumeButton().disabled")
        await js("window.resumeButton().click()")
        await until("document.body.innerText.includes('Unknown upload outcome requires explicit')")
        assert await js("window.calls.filter(c=>c.command==='resume_publisher_operation').length===1&&window.resumeButton().disabled&&!document.querySelectorAll('input[type=checkbox]')[0].checked"), 'each attempt must visibly clear resume consent'
        await js("document.querySelectorAll('input[type=checkbox]')[1].click();document.querySelectorAll('input[type=checkbox]')[0].click()")
        await until("!window.resumeButton().disabled")
        await js("window.resumeButton().click()")
        await until("window.recovery.operations[0].state==='complete'&&document.body.innerText.includes('· complete')&&window.resumeButton().disabled")
        assert await js("window.calls.filter(c=>c.command==='resume_publisher_operation').every(c=>c.args.request.operation_id==='" + OP + "')")
        # An account switch must suppress a delayed response and clear selections.
        await js("window.recovery.operations[0].state='incomplete';Array.from(document.querySelectorAll('button')).find(b=>b.innerText==='REFRESH LOCAL RECOVERY').click()")
        await until("document.body.innerText.includes('incomplete')")
        await js("Array.from(document.querySelectorAll('button')).find(b=>b.innerText==='RESELECT PINNED ARCHIVE').click()")
        await until("document.body.innerText.includes('Reselected SHA-256')")
        await js("window.delayResume=true;document.querySelectorAll('input[type=checkbox]')[1].click();document.querySelectorAll('input[type=checkbox]')[0].click()")
        await until("!window.resumeButton().disabled")
        await js("window.resumeButton().click()")
        await until("typeof window.resolveStale==='function'")
        await js("document.querySelector('[data-testid=publisher-recovery-account-switch]').click()")
        await until("!document.body.innerText.includes('retained operation(s)')")
        await js("window.resolveStale({event_id:'STALE ACCOUNT RESULT'})")
        assert await js("!document.body.innerText.includes('STALE ACCOUNT RESULT')&&!document.body.innerText.includes('Reselected SHA-256')")
        calls = await js('window.calls')
        allowed = {'publisher_recovery_state', 'select_archive_build', 'inspect_archive', 'hash_selected_archive', 'resume_publisher_operation'}
        assert all(c['command'] in allowed for c in calls)
        OUT.joinpath('gate6-phase4-ui-result.json').write_text(json.dumps(dict(result='PASS', mode='real WASM / mock IPC only', calls=calls, live_publications=0, live_uploads=0), indent=2))
        print('PASS: read-only startup, unknown outcomes, explicit resume/retry, pinned archive reselection, original operation identity and stale-account response isolation; mock IPC only')


if __name__ == '__main__':
    root = Path(sys.argv[1]).resolve()
    assert root.joinpath('index.html').is_file(), 'Build the isolated debug frontend first'
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), functools.partial(Quiet, directory=str(root)))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix='publisher-recovery-chrome-', dir=str(OUT)) as profile:
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
