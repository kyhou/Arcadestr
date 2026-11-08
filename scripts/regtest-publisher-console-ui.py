#!/usr/bin/env python3
"""Exercise existing publisher routes/components in real WASM with mock IPC only."""
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
class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self,*_args):pass

async def verify(port,debug):
    with urllib.request.urlopen(f'http://127.0.0.1:{debug}/json') as response:
        page=next(p for p in json.load(response) if p.get('type')=='page' and p.get('url')=='about:blank')
    async with websockets.connect(page['webSocketDebuggerUrl'],max_size=30_000_000) as ws:
        seq=0
        async def call(method,params=None):
            nonlocal seq
            seq+=1;await ws.send(json.dumps(dict(id=seq,method=method,params=params or {})))
            while True:
                value=json.loads(await ws.recv())
                if value.get('id')==seq:
                    assert 'error' not in value,value
                    return value.get('result',{})
        async def js(expression):
            result=await call('Runtime.evaluate',dict(expression=expression,returnByValue=True,awaitPromise=True))
            assert 'exceptionDetails' not in result,result
            return result.get('result',{}).get('value')
        async def until(expression):
            for _ in range(400):
                if await js(expression):return
                await asyncio.sleep(.05)
            raise AssertionError((expression,await js('document.body.innerText'),await js('window.calls')))
        mock=r"""window.calls=[];window.drafts=[];window.handlers={};
        window.__TAURI__={event:{listen:async(name,callback)=>{window.handlers[name]=callback;return ()=>delete window.handlers[name]}},core:{invoke:async(command,args)=>{
          window.calls.push({command,args});
          if(command==='publisher_recovery_state')return {drafts:structuredClone(window.drafts),operations:[],checkpoints:[]};
          if(command==='publisher_distribution_state')return [{server_url:'https://one.test',operation_id:'fixture-operation',state:'registration_acknowledged',operator:'fixture-operator',capabilities:['upload-build','issue-receipt'],diagnostic:null}];
          if(command==='discover_campaigns'||command==='discover_campaign_summaries'||command==='enrich_store_pages'||command==='discover_adp_servers')return [];
          if(command==='check_adp_server')return {adp_version:'0.2.0',pubkey:args.serverUrl==='https://two.test'?'c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5':'79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798',max_upload_bytes:1048576};
          if(command==='load_authoritative_adp_edit_metadata')return {state:'absent'};
          if(command==='resolve_adp_operator')return 'https://one.test';
          if(command==='save_publisher_draft'){
            const draft=structuredClone(args.request.draft);draft.revision=(args.request.expected_revision||0)+1;
            window.drafts=window.drafts.filter(old=>old.draft_id!==draft.draft_id);window.drafts.push(draft);return draft.revision;
          }
          if(command==='preview_publisher_terms'){
            const keys=['79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798','c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5'];
            return {context:args.request,accepted_terms:keys.map((key,index)=>({operator_pubkey:key,terms_event_id:(index?'55':'44').repeat(32)})),offers:keys.map(key=>({version:1,operator_pubkey:key,payment_destination:'operator@fixture.invalid',fee:{type:'none'},limitations:['Synthetic signed-review response']})),legs:[{leg_id:'developer',role:'developer',recipient_pubkey:args.request.publisher_pubkey,destination:args.request.developer_destination,amount_msat:args.request.price_sats*1000,accepted_terms_id:null}]};
          }
          if(command==='load_publisher_store_page_editor'){
            const coordinate='30402:79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798:fixture';
            const draft={presentation_id:'fixture-page',listing_coordinates:[coordinate],content:{schema:'io.arcadestr.store-page',version:1,description_markdown:'Fixture content'},compact_tags:{title:null,summary:null,developer:null,publisher:null,release_date:null,genres:[],features:[],languages:[],website:null,support:null},loaded_event_id:null};
            return {draft,baseline_draft:structuredClone(draft),listings:[{listing_coordinate:coordinate,event_id:'11'.repeat(32),reciprocal:false}],diagnostics:[]};
          }
          if(command==='validate_store_page_draft')return {valid:true,diagnostics:[],preview:null};
          if(command==='get_blossom_server_settings')return {publisher_pubkey:'79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798',servers:[],mode:'manual',target_policy:'all'};
          throw 'Forbidden console fixture command: '+command;
        }}};"""
        await call('Page.enable');await call('Runtime.enable');await call('Network.enable')
        await call('Network.setBlockedURLs',dict(urls=['https://*','wss://*','ws://*']))
        await call('Page.addScriptToEvaluateOnNewDocument',dict(source=mock))
        await call('Page.navigate',dict(url=f'http://127.0.0.1:{port}/?publisher-console-preview=1'))
        await until("document.body.innerText.includes('DISTRIBUTION & CREDENTIAL MANAGEMENT')")
        await until("document.body.innerText.includes('Registration acknowledged (historical upload)')")
        assert await js("document.body.innerText.includes('CANCEL FULFILLMENT AUTHORITY')&&document.body.innerText.includes('REVOKE ONE ENTITLEMENT GRANT')")
        await js("window.click=text=>{const button=Array.from(document.querySelectorAll('button')).find(button=>button.innerText.trim()===text);if(!button)throw 'Missing button '+text;button.click()};document.querySelector('[data-testid=fixture-edit]').click()")
        await until("document.body.innerText.includes('SAVE DRAFT')")
        await js("window.click('SAVE DRAFT')")
        await until("window.drafts.length===1")
        assert await js("window.drafts[0].payload.form.title==='Publisher lifecycle fixture'")
        await js("const title=document.querySelector('#publish-title');if(!title)throw 'Title input missing';title.value='Unsaved local metadata';title.dispatchEvent(new Event('input',{bubbles:true}))")
        await js("window.click('RESTORE SAVED DRAFT (REPLACES FORM)')")
        await until("document.querySelector('#publish-title').value==='Publisher lifecycle fixture'")
        # Existing staged editor: the details and pricing steps lead to distribution.
        await js("window.next=()=>{const button=Array.from(document.querySelectorAll('button')).find(button=>button.innerText.includes('Continue')||button.innerText.includes('Next'));if(!button)throw 'No staged editor continue action';button.click()};window.next()")
        await until("document.body.innerText.includes('Step 2 of 4')")
        await js("window.next()")
        await until("document.body.innerText.includes('Step 3 of 4')")
        assert await js("document.body.textContent.includes('OPERATOR ISSUING ROLES')&&document.body.textContent.includes('SERVICE FEE CONSENT')")
        await js("const intent=document.querySelector('select[aria-label=\"Commercial intention\"]');intent.value='replace';intent.dispatchEvent(new Event('change',{bubbles:true}))")
        await until("document.body.textContent.includes('RETRIEVE SIGNED TERMS >')")
        await js("document.querySelectorAll('details').forEach(details=>details.open=true);window.click('RETRIEVE SIGNED TERMS >')")
        await until("document.body.innerText.includes('terms '+ '44'.repeat(32))")
        assert await js("document.body.innerText.includes('terms '+ '55'.repeat(32))"), 'both selected operators require exact-root disclosure'
        await js("window.commercialConsent=()=>Array.from(document.querySelectorAll('label')).find(label=>label.textContent.includes('I approve these exact signed terms')).querySelector('input');window.commercialConsent().click()")
        assert await js("window.commercialConsent().checked")
        await js("window.click('RETRIEVE SIGNED TERMS >')")
        await until("window.calls.filter(call=>call.command==='preview_publisher_terms').length===2&&document.body.innerText.includes('terms '+ '44'.repeat(32))")
        assert await js("!window.commercialConsent().checked"), 'retrieving a fresh commercial review must visibly reset exact-terms consent'
        assert await js("document.body.textContent.includes('ISSUE RECEIPTS')&&document.body.textContent.includes('ISSUE GRANTS')")
        # Normal publisher navigation must expose the actual Store Page editor.
        await js("window.storeTab=()=>Array.from(document.querySelectorAll('button')).find(button=>button.innerText.trim()==='Store Page');if(!window.storeTab())throw 'Missing Store Page tab';window.storeTab().click()")
        await until("window.calls.some(call=>call.command==='load_publisher_store_page_editor')")
        await until("document.body.innerText.includes('SAVE STORE PAGE DRAFT')")
        await js("window.click('SAVE STORE PAGE DRAFT')")
        await until("window.drafts.some(draft=>draft.kind==='store_page')")
        assert await js("document.body.textContent.includes('RESTORE SAVED DRAFT (REPLACES EDITOR)')")
        await js("document.querySelector('[data-testid=fixture-account-switch]').click()")
        await until("Array.from(document.querySelectorAll('button')).filter(button=>button.innerText.includes('Publish')).every(button=>button.disabled||button.innerText.trim()!=='Publish')")
        constellations=await js('window.calls')
        forbidden={'publish_adp_listing_with_contract','publish_store_page','publish_campaign','revoke_publisher_credential','pay_nwc_invoice','start_blossom_upload','resume_publisher_operation'}
        assert not any(call['command'] in forbidden for call in constellations)
        OUT.joinpath('gate6-phase6-console-ui-result.json').write_text(json.dumps(dict(result='PASS',mode='existing publisher UI / mock IPC only',calls=constellations,live_actions=0),indent=2))
        print('PASS: existing managed-game routes expose recovery, distribution/revocation, listing draft save/restore, commercial review, operator roles, Store Page drafts and account fencing; mock IPC only')

if __name__=='__main__':
    root=Path(sys.argv[1]).resolve();assert root.joinpath('index.html').is_file()
    server=http.server.ThreadingHTTPServer(('127.0.0.1',0),functools.partial(Quiet,directory=str(root)));threading.Thread(target=server.serve_forever,daemon=True).start()
    with tempfile.TemporaryDirectory(prefix='publisher-console-chrome-',dir=str(OUT)) as profile:
        chrome=subprocess.Popen(['chromium','--headless','--no-sandbox','--disable-gpu','--remote-debugging-port=0','--user-data-dir='+profile,'about:blank'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        try:
            active=Path(profile)/'DevToolsActivePort'
            for _ in range(100):
                if active.exists():break
                time.sleep(.1)
            asyncio.run(verify(server.server_port,int(active.read_text().splitlines()[0])))
        finally:
            chrome.terminate();chrome.wait(timeout=10);server.shutdown()
