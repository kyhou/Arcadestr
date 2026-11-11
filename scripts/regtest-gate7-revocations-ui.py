#!/usr/bin/env python3
"""Real WASM ownership-sync context / mock IPC, no relay or wallet traffic.
Usage: python3 -B scripts/regtest-gate7-revocations-ui.py FRONTEND OUTPUT_DIR
Database/protocol/History/Library/eligibility assertions live in native tests.
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


class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_args):
        pass


async def verify(port, debug, out):
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

        mock = """window.calls=[];window.pending=[];window.listeners={};
        window.revoked={checked:1,revoked:1,rejected_events:0,indeterminate:0,incomplete:true};
        window.__TAURI__={event:{listen:async(name,callback)=>{window.listeners[name]=callback;return ()=>delete window.listeners[name];}},core:{invoke:async(cmd,args)=>{
          window.calls.push({cmd,args});
          if(cmd==='sync_purchase_receipts')return {chains:[],rejected_events:0,relay_failures:0,incomplete:true,message:'Receipt fixture unchanged'};
          if(cmd==='reconcile_known_entitlements')return await new Promise((resolve,reject)=>window.pending.push({resolve,reject,args}));
          throw 'Forbidden IPC: '+cmd;
        }}};"""
        await call("Page.enable")
        await call("Runtime.enable")
        await call("Network.enable")
        await call("Network.setBlockedURLs", dict(urls=["https://*", "ws://*", "wss://*"]))
        await call("Page.addScriptToEvaluateOnNewDocument", dict(source=mock))
        results = []
        revision = "document.querySelector('[data-testid=receipt-parent-revision]').innerText"
        switch = "document.querySelector('[data-testid=mock-receipt-account-switch]').click()"
        sync = "Array.from(document.querySelectorAll('button')).find(b=>b.innerText==='SYNC RECEIPTS').click()"
        for mode in ["account-success", "account-error", "aba-success", "aba-error"]:
            await call("Page.navigate", dict(url=f"http://127.0.0.1:{port}/?receipt-portability-preview=1"))
            await until("window.pending.length===1&&!!window.listeners['relay-connection']&&!!document.querySelector('[data-testid=receipt-sync-summary]')")
            original = await js("window.pending[0].args.request.expected_account_pubkey")
            await js(switch)
            if mode.startswith("aba"):
                await js(switch)
            await js(sync)
            await until("window.pending.length===2&&!!document.querySelector('[data-testid=receipt-sync-summary]')")
            before = await js(revision)
            await js("window.pending[0]." + ("reject('stale error')" if mode.endswith("error") else "resolve(window.revoked)"))
            await asyncio.sleep(.15)
            assert await js(revision) == before, mode
            assert await js("window.pending.length") == 2, "stale callback must not retry or clear current work"
            # Reconnection while busy is coalesced and retried, not lost.
            await js("window.listeners['relay-connection']({payload:{type:'connected'}});window.listeners['relay-connection']({payload:{type:'connected'}})")
            assert await js("window.pending.length") == 2
            await js("window.pending[1].resolve(window.revoked)")
            await until("window.pending.length===3")
            assert int(await js(revision)) > int(before), mode
            await js("window.pending[2].resolve({...window.revoked,revoked:0,indeterminate:1})")
            await asyncio.sleep(.15)
            current = await js(revision)
            await js("window.listeners['receipt-ownership-updated']({payload:{account:'unrelated-account'}})")
            assert await js(revision) == current
            account = await js("window.pending[2].args.request.expected_account_pubkey")
            assert (account == original) == mode.startswith("aba")
            calls = await js("window.calls")
            assert all(c["cmd"] in ["sync_purchase_receipts", "reconcile_known_entitlements"] for c in calls)
            assert await js("document.body.innerText.includes('Receipt fixture unchanged')")
            results.append(dict(mode=mode, result="PASS", calls=calls))
        out.joinpath("revocations-ui.json").write_text(json.dumps(dict(result="PASS", mode="real WASM / mock IPC", scenarios=results, live_operations=0), indent=2))
        print("PASS startup, explicit refresh, account/ABA stale success/error, reconnection coalescing, account-scoped invalidation; real WASM/mock IPC")


if __name__ == "__main__":
    root, out = map(lambda p: Path(p).resolve(), sys.argv[1:3])
    assert root.joinpath("index.html").is_file()
    out.mkdir(parents=True, exist_ok=True)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=str(root)))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix="revocation-chrome-", dir=str(out)) as profile:
        chrome = subprocess.Popen(["chromium", "--headless", "--no-sandbox", "--disable-gpu", "--remote-debugging-port=0", "--user-data-dir=" + profile, "about:blank"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            active = Path(profile) / "DevToolsActivePort"
            for _ in range(300):
                if active.exists():
                    break
                time.sleep(.1)
            assert active.is_file(), "Chromium did not expose its local debugging endpoint"
            asyncio.run(verify(server.server_port, int(active.read_text().splitlines()[0]), out))
        finally:
            chrome.terminate()
            chrome.wait(timeout=10)
            server.shutdown()
