#!/usr/bin/env python3
"""Real GameDetailView/WASM; mock IPC only, no wallet or external transport.

Usage: python3 -B scripts/regtest-gate7-claims-ui.py ISOLATED_FRONTEND [OUTPUT_DIR]
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
                data = json.loads(await ws.recv())
                if data.get("id") == seq:
                    assert "error" not in data, data
                    return data.get("result", {})

        async def js(expression):
            result = await call("Runtime.evaluate", dict(expression=expression, returnByValue=True, awaitPromise=True))
            assert "exceptionDetails" not in result, result
            return result.get("result", {}).get("value")

        async def until(expression):
            for _ in range(600):
                if await js(expression):
                    return
                await asyncio.sleep(0.1)
            raise AssertionError((expression, await js("document.body.innerText")))

        async def start_claim():
            await js("window.clickLabel('Claim and keep')")
            await until("Array.from(document.querySelectorAll('button')).some(b=>b.innerText==='Confirm claim')")
            await js("window.clickLabel('Confirm claim')")

        mock = """window.calls=[];window.pending=[];window.forbidden=[];
        window.__TAURI__={event:{listen:async()=>()=>{}},core:{invoke:async(cmd,args)=>{
          window.calls.push({cmd,args});
          if(cmd==='get_platform_info')return {os:'linux',arch:'x86_64'};
          if(cmd==='get_listing_ownership'||cmd==='is_game_in_library')return false;
          if(cmd==='get_installed_games'||cmd==='list_retained_packages')return [];
          if(cmd==='enrich_store_page_detail')return {generation:args.request.generation,cached:null,refreshed:{state:'not_associated'},listing_event_current:true};
          if(cmd==='discover_campaigns')return [{root_event_id:'22'.repeat(32),campaign_id:'isolated-campaign',starts_at:100,ends_at:4102444800,classification:'active',mode:'claim'}];
          if(cmd==='fetch_profile')throw 'No fixture profile';
          if(cmd==='claim_entitlement'){
            if(!/^[0-9a-f]{64}$/.test(args.request.expected_account_pubkey))throw 'Missing initiating buyer binding';
            return await new Promise((resolve,reject)=>window.pending.push({resolve,reject}));
          }
          window.forbidden.push(cmd);throw 'Forbidden fixture command: '+cmd;
        }}};
        window.clickLabel=(text)=>{let b=Array.from(document.querySelectorAll('button')).find(b=>b.innerText===text&&!b.disabled);if(!b)throw 'Missing enabled button '+text;b.click();};
        window.hasClaim=()=>Array.from(document.querySelectorAll('button')).some(b=>b.innerText==='Claim and keep'&&!b.disabled);
        window.resolveClaim=(index)=>window.pending[index].resolve({grant:{},download_token:'synthetic-token',token_expires_at:4102444800,already_claimed:false});
        """
        await call("Page.enable")
        await call("Network.enable")
        await call("Network.setBlockedURLs", dict(urls=["https://*", "ws://*", "wss://*"]))
        await call("Page.addScriptToEvaluateOnNewDocument", dict(source=mock))
        results = []
        await call("Page.navigate", dict(url=f"http://127.0.0.1:{port}/?claim-session-preview=1"))
        await until("window.hasClaim()")
        await js("window.clickLabel('Claim and keep')")
        await until("document.body.innerText.includes('Confirm permanent access claim')")
        await js("document.querySelector('[data-testid=claim-reconnect]').click()")
        await until("window.hasClaim()&&!document.body.innerText.includes('Confirm permanent access claim')")
        assert await js("window.pending.length===0"), "old confirmation must not authorize a new session"
        for transition, outcome in [(t, o) for t in ["roundtrip", "switch", "logout", "reconnect"] for o in ["success", "error"]]:
            await call("Page.navigate", dict(url=f"http://127.0.0.1:{port}/?claim-session-preview=1"))
            await until("window.hasClaim()")
            await start_claim()
            await until("window.pending.length===1")
            if transition in ("roundtrip", "switch"):
                await js("document.querySelector('[data-testid=claim-switch]').click()")
            elif transition == "logout":
                await js("document.querySelector('[data-testid=claim-logout]').click()")
            else:
                await js("document.querySelector('[data-testid=claim-reconnect]').click()")
            if transition in ("roundtrip", "logout"):
                await js("document.querySelector('[data-testid=claim-return]').click()")
            await until("window.hasClaim()")
            await start_claim()
            await until("window.pending.length===2")
            expected = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5" if transition == "switch" else "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
            assert await js("window.calls.filter(c=>c.cmd==='claim_entitlement')[1].args.request.expected_account_pubkey") == expected
            # A stale success must neither grant ownership nor idle the newer claim.
            if outcome == "success":
                await js("window.resolveClaim(0)")
            else:
                await js("window.pending[0].reject('Stale claim failure')")
            await asyncio.sleep(0.2)
            assert await js("!document.body.innerText.includes('Permanent access is recorded')"), transition
            assert await js("document.body.innerText.includes('Claiming access...')"), transition
            assert await js("!document.body.innerText.includes('Stale claim failure')"), transition
            await js("window.resolveClaim(1)")
            await until("document.body.innerText.includes('Permanent access is recorded')")
            assert await js("window.forbidden.length===0"), await js("window.forbidden")
            results.append(dict(transition=transition, stale_result=outcome, result="PASS"))
        out.joinpath("claims-ui.json").write_text(json.dumps(dict(result="PASS", evidence="real WASM / mock IPC", cases=results, live_operations=0), indent=2))
        print("PASS: stale claim success cannot grant access or clear a newer operation after switch, logout, reconnect or A-B-A; current claim still succeeds")


if __name__ == "__main__":
    root = Path(sys.argv[1]).resolve()
    out = Path(sys.argv[2] if len(sys.argv) > 2 else "/tmp/opencode")
    assert root.joinpath("index.html").is_file()
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=str(root)))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix="gate7-claims-chrome-", dir=str(out)) as profile:
        chrome = subprocess.Popen([
            "chromium", "--headless", "--no-sandbox", "--disable-gpu",
            "--disable-background-networking", "--disable-component-update", "--disable-sync", "--no-first-run",
            "--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1, EXCLUDE localhost",
            "--remote-debugging-port=0", "--user-data-dir=" + profile, "about:blank",
        ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            active = Path(profile) / "DevToolsActivePort"
            for _ in range(100):
                if active.exists():
                    break
                time.sleep(0.1)
            asyncio.run(verify(server.server_port, int(active.read_text().splitlines()[0]), out))
        finally:
            chrome.terminate()
            chrome.wait(timeout=10)
            server.shutdown()
