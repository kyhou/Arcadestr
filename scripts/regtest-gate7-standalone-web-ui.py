#!/usr/bin/env python3
"""Real WASM feature-boundary regression; synthetic IPC only, no external traffic.
Usage: python3 -B scripts/regtest-gate7-standalone-web-ui.py MODE FRONTEND OUTPUT_DIR
MODE: standalone | desktop-bridge
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


class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_args):
        pass


async def verify(port, debug, mode, out):
    with urllib.request.urlopen(f"http://127.0.0.1:{debug}/json") as response:
        page = next(p for p in json.load(response) if p.get("type") == "page")
    async with websockets.connect(page["webSocketDebuggerUrl"], max_size=20_000_000) as ws:
        seq = 0

        async def call(method, params=None):
            nonlocal seq
            seq += 1
            await ws.send(json.dumps(dict(id=seq, method=method, params=params or {})))
            while True:
                result = json.loads(await ws.recv())
                if result.get("id") == seq:
                    assert "error" not in result, result
                    return result.get("result", {})

        async def js(expression):
            result = await call("Runtime.evaluate", dict(expression=expression, returnByValue=True, awaitPromise=True))
            assert "exceptionDetails" not in result, result
            return result.get("result", {}).get("value")

        async def until(expression):
            for _ in range(600):
                if await js(expression):
                    return
                await asyncio.sleep(.1)
            raise AssertionError((expression, await js("document.body.innerText")))

        mock = """window.ipcCalls=[];window.eventCalls=[];
        window.__TAURI__={event:{listen:async(name,callback)=>{window.eventCalls.push(name);return ()=>{};}},core:{invoke:async(cmd,args)=>{
          window.ipcCalls.push({cmd,args});
          if(cmd==='regtest_nip44_probe')return 'SYNTHETIC DESKTOP PROBE';
          if(cmd==='get_version_info')return {version:'fixture',revision:1,full:'fixture',os:'linux',arch:'x86_64'};
          if(cmd==='is_authenticated')return false;
          if(['get_saved_users','get_accounts','list_accounts','get_connected_relays'].includes(cmd))return [];
          throw 'No fixture executor: '+cmd;
        }}};
        localStorage.setItem('arcadestr.debug.storefront','1');"""
        await call("Page.enable")
        await call("Runtime.enable")
        await call("Network.enable")
        await call("Network.setBlockedURLs", dict(urls=["https://*", "ws://*", "wss://*"]))
        await call("Page.addScriptToEvaluateOnNewDocument", dict(source=mock))
        layout_observations = []
        if mode == "standalone":
            # The existing real-component fixture has an account selected: controls
            # must be unavailable because of the feature, not because of logout.
            await call("Page.navigate", dict(url=f"http://127.0.0.1:{port}/?receipt-portability-preview=1"))
            await until("!!document.querySelector('[data-testid=receipt-sync-status]')")
            assert await js("document.querySelector('[data-testid=receipt-sync-status]').innerText.includes('require the desktop application')")
            assert await js("document.querySelector('[data-testid=receipt-import-input]').disabled")
            assert await js("Array.from(document.querySelectorAll('button')).filter(b=>['SYNC RECEIPTS','RESCAN AVAILABLE HISTORY','VERIFY AND IMPORT'].includes(b.innerText)).every(b=>b.disabled)")
            # Synthetic dispatch cannot bypass the method-level feature guard.
            await js("{const f=document.querySelector('[data-testid=receipt-import-input]');f.value='[]';f.dispatchEvent(new Event('input',{bubbles:true}));document.querySelector('[data-testid=receipt-import-submit]').dispatchEvent(new MouseEvent('click',{bubbles:true}));}")
            assert await js("window.ipcCalls.length===0&&window.eventCalls.length===0")
            await call("Page.navigate", dict(url=f"http://127.0.0.1:{port}/"))
            await until("!!document.querySelector('#account-control-trigger')")
            await js("Array.from(document.querySelectorAll('.arc-primary-nav button')).find(b=>b.textContent.trim()==='Library').click()")
            await until("document.body.innerText.includes('Installed-game records and native filesystem information are available only in the Arcadestr desktop application.')")
            await js("Array.from(document.querySelectorAll('.arc-primary-nav button')).find(b=>b.textContent.trim()==='Publish').click()")
            await until("document.body.innerText.includes('This standalone web build does not provide nonfunctional publishing controls.')")
            await js("document.querySelector('#account-control-trigger').click()")
            await until("!!document.querySelector('#account-control-menu')")
            await js("Array.from(document.querySelectorAll('#account-control-menu button')).find(b=>b.textContent.trim()==='Purchases').click()")
            await until("!!document.querySelector('[data-testid=load-refunds]')")
            assert await js("document.body.innerText.includes('Recipient refund discovery, recovery and payment require the desktop application.')")
            assert await js("document.querySelector('[data-testid=load-refunds]').disabled&&document.querySelector('[data-testid=refund-processor]').disabled")
            assert await js("document.querySelector('[data-recovery-events]').dataset.recoveryEvents==='unavailable'")
            await js("document.querySelector('[data-testid=load-refunds]').dispatchEvent(new MouseEvent('click',{bubbles:true}))")
        else:
            await call("Page.navigate", dict(url=f"http://127.0.0.1:{port}/"))

        await until("Array.from(document.querySelectorAll('button')).some(b=>b.textContent.trim()==='Debug')")
        await js("Array.from(document.querySelectorAll('button')).find(b=>b.textContent.trim()==='Debug').click()")
        await until("!!document.querySelector('[data-testid=regtest-nip44-probe]')")
        if mode == "standalone":
            assert await js("document.querySelector('[data-testid=regtest-nip44-probe]').disabled")
            assert await js("document.querySelector('[data-testid=regtest-nip44-result]').innerText.includes('only in the desktop app')")
            assert await js("window.ipcCalls.length===0&&window.eventCalls.length===0"), await js("window.ipcCalls")
            # Keep full-shell sizing as a diagnostic, not feature-boundary
            # acceptance: blocked external icon fonts render wide ligature names.
            # No stylesheet substitution or unrelated app layout change is made.
            icons = await js("Array.from(document.querySelectorAll('.material-symbols-outlined')).map(e=>({text:e.textContent,width:e.getBoundingClientRect().width,font:getComputedStyle(e).fontSize}))")
            out.joinpath("blocked-font-fallback.json").write_text(json.dumps(icons, indent=2))
            for width, height in [(1920, 1080), (390, 844)]:
                await call("Emulation.setDeviceMetricsOverride", dict(width=width, height=height, deviceScaleFactor=1, mobile=width < 500))
                dimensions = await js("({viewport:innerWidth,scroll:document.documentElement.scrollWidth})")
                layout_observations.append(dict(requested_width=width, **dimensions))
                out.joinpath(f"standalone-layout-{width}.json").write_text(json.dumps(dimensions, indent=2))
                shot = await call("Page.captureScreenshot", dict(format="png", captureBeyondViewport=True))
                out.joinpath(f"standalone-layout-{width}.png").write_bytes(base64.b64decode(shot["data"]))
                assert await js("Array.from(document.querySelectorAll('[data-testid=receipt-sync-status],[data-recovery-events]>p[role=status]')).every(e=>e.scrollWidth<=e.clientWidth+1)")
            shot = await call("Page.captureScreenshot", dict(format="png", captureBeyondViewport=True))
            out.joinpath("standalone-disabled-states.png").write_bytes(base64.b64decode(shot["data"]))
        else:
            assert await js("!document.querySelector('[data-testid=regtest-nip44-probe]').disabled")
            await js("document.querySelector('[data-testid=regtest-nip44-probe]').click()")
            await until("document.querySelector('[data-testid=regtest-nip44-result]').innerText==='SYNTHETIC DESKTOP PROBE'")
            assert await js("window.ipcCalls.filter(c=>c.cmd==='regtest_nip44_probe').length===1")
        out.joinpath(mode + "-boundary-ui.json").write_text(json.dumps(dict(result="PASS", mode=mode, validation="real WASM / mock IPC, no real signer or wallet", ipc=await js("window.ipcCalls"), listeners=await js("window.eventCalls"), layout_observations=layout_observations, layout_note="Full-shell mobile overflow with blocked external icon fonts is observed, not certified or fixed by this feature-boundary test."), indent=2))
        print("PASS", mode, "feature boundary, explicit unavailable states / unchanged desktop signer-probe dispatch")


if __name__ == "__main__":
    mode = sys.argv[1]
    assert mode in ["standalone", "desktop-bridge"]
    root, out = map(lambda value: Path(value).resolve(), sys.argv[2:4])
    assert root.joinpath("index.html").is_file()
    out.mkdir(parents=True, exist_ok=True)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=str(root)))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix="frontend-boundary-chrome-", dir=str(out)) as profile:
        with out.joinpath(mode + "-chromium.log").open("w") as stderr:
            chrome = subprocess.Popen(["chromium", "--headless", "--no-sandbox", "--disable-gpu", "--disable-background-networking", "--remote-debugging-port=0", "--user-data-dir=" + profile, "about:blank"], stdout=subprocess.DEVNULL, stderr=stderr)
            try:
                active = Path(profile) / "DevToolsActivePort"
                for _ in range(300):
                    if active.exists():
                        break
                    time.sleep(.1)
                assert active.is_file(), "Chromium startup failed; see captured stderr"
                asyncio.run(verify(server.server_port, int(active.read_text().splitlines()[0]), mode, out))
            finally:
                chrome.terminate()
                chrome.wait(timeout=10)
                server.shutdown()
