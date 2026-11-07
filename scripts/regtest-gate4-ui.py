#!/usr/bin/env python3
"""Real WASM, allowlisted mock IPC, no wallet/credentials/live invoices.

Usage: python3 -B scripts/regtest-gate4-ui.py /tmp/opencode/gate4-frontend
Requires Chromium and Python websockets. Build the debug frontend with Trunk first.
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

OUT = Path("/tmp/opencode")
BUYER = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"


def plan():
    legs = [dict(leg_id="developer", role="developer", recipient_pubkey="22" * 32,
                 destination="bob@example.test", amount_msat=100000),
            dict(leg_id="operator", role="operator", recipient_pubkey="33" * 32,
                 destination="carol@example.test", amount_msat=100000, accepted_terms_id="55" * 32)]
    return dict(quote=dict(version=1, quote_id="mock-purchase", buyer_pubkey=BUYER,
                          listing_event_id="11" * 32, listing_coordinate="30402:" + "22" * 32 + ":offline",
                          processor_pubkey="33" * 32, gross_msat=200000, issued_at=100,
                          payment_expires_at=4102444800, confirmation_expires_at=4102531200,
                          refund_destination_commitment="44" * 32, legs=legs, state="active"),
                quote_event_id="66" * 32, commitment="77" * 32, network="Regtest",
                terms_verified=True, approved=False, approved_limits=None, approved_fee_policy=None,
                halted=False, purchase_completed=False, settlement_state="active", wallet_connected=True,
                wallet_capabilities=dict(methods=["get_info", "lookup_invoice"], network="regtest"),
                legs=[dict(leg_id=leg["leg_id"], progress="pending", attempt_id=None, payment_hash=None,
                           invoice_payee=None, fees_paid_msat=None, can_reconcile=False) for leg in legs],
                restriction="Live quotes, invoices and payment dispatch remain locked.")


class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_args):
        pass


async def verify(port, debug):
    with urllib.request.urlopen(f"http://127.0.0.1:{debug}/json") as response:
        page = next(p for p in json.load(response) if p.get("type") == "page" and p.get("url") == "about:blank")
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
            for _ in range(1200):
                if await js(expression):
                    return
                await asyncio.sleep(0.1)
            raise AssertionError((expression, await js("document.body.innerText")))

        mock = """window.plan=PLAN;window.calls=[];window.__TAURI__={core:{invoke:async(cmd,args)=>{
          window.calls.push({cmd,args});
          if(cmd==='get_purchase_plan')return structuredClone(window.plan);
          if(cmd==='approve_purchase_plan'){
            if(!args.request.confirm_complete_plan)throw 'Missing consent';
            window.plan.approved=true;window.plan.approved_fee_policy=args.request.fee_policy;
            window.plan.approved_limits={principal_msat:200000,fee_reserve_msat:args.request.fee_reserve_msat,total_msat:args.request.total_msat,maximum_dispatches:2};
            return structuredClone(window.plan);
          }
          if(cmd==='resume_split_payment')return {state:'active',settled_leg:null,receipt:null};
          throw 'Forbidden mock command: '+cmd;
        }}};""".replace("PLAN", json.dumps(plan()))
        await call("Page.enable")
        await call("Runtime.enable")
        await call("Network.enable")
        await call("Network.setBlockedURLs", dict(urls=["https://*"]))
        await call("Page.addScriptToEvaluateOnNewDocument", dict(source=mock))
        await call("Page.navigate", dict(url=f"http://127.0.0.1:{port}/?purchase-plan-preview=1"))
        await until("!!document.querySelector('[data-testid=approve-purchase-plan]')")
        assert await js("document.body.innerText.includes('bob@example.test')&&document.body.innerText.includes('carol@example.test')")
        assert await js("document.body.innerText.includes('publicly link the buyer')")
        assert await js("document.querySelector('[data-testid=approve-purchase-plan]').disabled")
        await js("document.querySelector('[data-testid=purchase-approval-check]').click()")
        await until("!document.querySelector('[data-testid=approve-purchase-plan]').disabled")
        await js("document.querySelector('[data-testid=approve-purchase-plan]').click()")
        await until("document.querySelector('[data-testid=plan-parent-approved]').innerText==='true'")
        assert await js("document.querySelector('[data-testid=plan-parent-dispatch-ready]').innerText==='false'")
        assert await js("window.calls.find(c=>c.cmd==='approve_purchase_plan').args.request.fee_policy==='provider_ceiling'")
        await js("window.plan.wallet_capabilities.methods.push('pay_invoice');document.querySelector('[data-testid=mock-wallet-change]').click()")
        await until("document.querySelector('[data-testid=plan-parent-dispatch-ready]').innerText==='true'")
        await js("window.plan.legs[0].progress='paid';window.plan.legs[1].progress='uncertain';window.plan.legs[1].can_reconcile=true;window.plan.legs[1].attempt_id='original-attempt';Array.from(document.querySelectorAll('button')).find(b=>b.innerText==='REFRESH ORIGINAL STATE').click()")
        await until("document.body.innerText.includes('UNCERTAIN — NO REPAYMENT')")
        assert await js("document.querySelector('[data-testid=plan-parent-dispatch-ready]').innerText==='false'&&document.querySelector('[data-testid=purchase-completion]').innerText.includes('NOT COMPLETED')")
        await js("Array.from(document.querySelectorAll('button')).find(b=>b.innerText.includes('CHECK ORIGINAL PAYMENT')&&!b.disabled).click()")
        await until("document.querySelector('[data-testid=plan-parent-recovered]').innerText==='1'")
        await until("!!document.querySelector('[data-testid=purchase-completion]')")
        await js("window.plan.wallet_connected=false;window.plan.approved=false;document.querySelector('[data-testid=mock-wallet-change]').click()")
        await until("document.querySelector('[data-testid=plan-parent-approved]').innerText==='false'")
        assert await js("document.querySelector('[data-testid=approve-purchase-plan]').disabled")
        for width, height, name in [(1920, 1080, "desktop"), (390, 844, "mobile")]:
            await call("Emulation.setDeviceMetricsOverride", dict(width=width, height=height, deviceScaleFactor=1, mobile=width < 500))
            assert await js(f"document.documentElement.scrollWidth<={width}")
            shot = await call("Page.captureScreenshot", dict(format="png", captureBeyondViewport=True))
            OUT.joinpath(f"gate4-plan-{name}.png").write_bytes(base64.b64decode(shot["data"]))
        calls = await js("window.calls")
        assert all(c["cmd"] in ["get_purchase_plan", "approve_purchase_plan", "resume_split_payment"] for c in calls)
        OUT.joinpath("gate4-ui-result.json").write_text(json.dumps(dict(result="PASS", mode="real WASM / mock IPC only", calls=calls, live_payments=0, live_invoices=0), indent=2))
        print("PASS: explicit consent, privacy, parent approval, capability gating, wallet invalidation, partial/uncertain recovery, parent refresh, responsive UI; mock IPC only")


if __name__ == "__main__":
    root = Path(sys.argv[1]).resolve()
    assert root.joinpath("index.html").is_file(), "Build the isolated debug frontend first"
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=str(root)))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix="gate4-chrome-", dir=str(OUT)) as profile:
        chrome = subprocess.Popen(["chromium", "--headless", "--no-sandbox", "--disable-gpu", "--remote-debugging-port=0", "--user-data-dir=" + profile, "about:blank"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            active = Path(profile) / "DevToolsActivePort"
            for _ in range(100):
                if active.exists():
                    break
                time.sleep(0.1)
            asyncio.run(verify(server.server_port, int(active.read_text().splitlines()[0])))
        finally:
            chrome.terminate()
            chrome.wait(timeout=10)
            server.shutdown()
