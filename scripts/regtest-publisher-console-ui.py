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
        window.operations=[{operation_id:'op-unfinished',kind:'listing',coordinate:'30402:79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798:fixture',intent:{},artifact_hash:null,state:'incomplete',result:null}];
        window.__TAURI__={event:{listen:async(name,callback)=>{window.handlers[name]=callback;return ()=>delete window.handlers[name]}},core:{invoke:async(command,args)=>{
          window.calls.push({command,args});
          if(command==='publisher_recovery_state')return {drafts:structuredClone(window.drafts),operations:structuredClone(window.operations),checkpoints:[]};
          if(command==='publisher_distribution_state')return [{server_url:'https://one.test',operation_id:'fixture-operation',state:'registration_acknowledged',operator:'fixture-operator',capabilities:['upload-build','issue-receipt'],diagnostic:null}];
          if(command==='discover_campaigns'||command==='discover_campaign_summaries'||command==='enrich_store_pages'||command==='discover_adp_servers')return [];
          if(command==='check_adp_server')return {adp_version:'0.2.0',pubkey:args.serverUrl==='https://two.test'?'c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5':'79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798',max_upload_bytes:1048576};
          if(command==='load_authoritative_adp_edit_metadata')return {state:'absent'};
          if(command==='resolve_adp_operator')return 'https://one.test';
          if(command==='save_publisher_draft'){
            const current=window.drafts.find(old=>old.draft_id===args.request.draft.draft_id);
            if((current?current.revision:null)!==(args.request.expected_revision??null))throw 'Draft changed on this device; restore it before saving again';
            const draft=structuredClone(args.request.draft);draft.revision=(args.request.expected_revision||0)+1;
            window.drafts=window.drafts.filter(old=>old.draft_id!==draft.draft_id);window.drafts.push(draft);return draft.revision;
          }
          if(command==='publish_store_page'){const id='77'.repeat(32);return {store_page_coordinate:'30078:fixture',store_page:{event_id:id,success_count:2,failure_count:0,propagation_confirmed:true},listing_updates:[],complete:true,retryable:false,cache_error:null,retry_scope_complete:false};}
          if(command==='delete_publisher_draft'){window.drafts=window.drafts.filter(old=>old.draft_id!==args.request.draft_id);return null;}
          if(command==='preview_publisher_terms'){
            const keys=['79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798','c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5'];
            return {context:args.request,accepted_terms:keys.map((key,index)=>({operator_pubkey:key,terms_event_id:(index?'55':'44').repeat(32)})),offers:keys.map(key=>({version:1,operator_pubkey:key,payment_destination:'operator@fixture.invalid',fee:{type:'none'},limitations:['Synthetic signed-review response']})),legs:[{leg_id:'developer',role:'developer',recipient_pubkey:args.request.publisher_pubkey,destination:args.request.developer_destination,amount_msat:args.request.price_sats*1000,accepted_terms_id:null}]};
          }
          if(command==='load_publisher_store_page_editor'){
            const coordinate='30402:79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798:fixture';
            const draft={presentation_id:'fixture-page',listing_coordinates:[coordinate],content:{schema:'io.arcadestr.store-page',version:1,description_markdown:'Fixture content'},compact_tags:{title:null,summary:null,developer:null,publisher:null,release_date:null,genres:[],features:[],languages:[],website:null,support:null},loaded_event_id:'66'.repeat(32)};
            return {draft,baseline_draft:structuredClone(draft),listings:[{listing_coordinate:coordinate,event_id:'11'.repeat(32),reciprocal:true}],diagnostics:[]};
          }
          if(command==='validate_store_page_draft')return {valid:true,diagnostics:[],preview:null};
          if(command==='get_blossom_server_settings')return {publisher_pubkey:'79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798',servers:[],mode:'manual',target_policy:'all'};
          throw 'Forbidden console fixture command: '+command;
        }}};"""
        await call('Page.enable');await call('Runtime.enable');await call('Network.enable')
        await call('Network.setBlockedURLs',dict(urls=['https://*','wss://*','ws://*']))
        await call('Page.addScriptToEvaluateOnNewDocument',dict(source=mock))
        await call('Page.navigate',dict(url=f'http://127.0.0.1:{port}/?publisher-console-preview=1'))
        # textContent: kickers and chips are uppercased by CSS, which innerText reflects.
        text="document.body.textContent"
        await js("window.click=text=>{const button=Array.from(document.querySelectorAll('button')).find(button=>button.innerText.trim()===text);if(!button)throw 'Missing button '+text;button.click()};"
                 "window.row=title=>Array.from(document.querySelectorAll('.v2-hub-row-button')).find(button=>button.querySelector('.v2-hub-row-title').innerText.trim()===title);"
                 "window.setInput=(selector,value)=>{const input=document.querySelector(selector);if(!input)throw 'Missing input '+selector;input.value=value;input.dispatchEvent(new Event('input',{bubbles:true}))};"
                 "window.shot=async name=>{}")
        async def shot(name):
            data=await call('Page.captureScreenshot',dict(format='png',captureBeyondViewport=True))
            import base64;OUT.joinpath(f'publisher-hub-{name}.png').write_bytes(base64.b64decode(data['data']))
        await call('Emulation.setDeviceMetricsOverride',dict(width=1440,height=1000,deviceScaleFactor=1,mobile=False))

        # Live game: the hub overview is a checklist of sections, not a stepper.
        await until(f"{text}.includes('Your game')&&!!window.row('Delivery & builds')")
        for section in ['Basics','Store page','Price & access','Delivery & builds']:
            assert await js(f"!!window.row({json.dumps(section)})"), section
        assert await js(f"!{text}.includes('Step 1 of')&&!{text}.includes('Continue >')"), 'no wizard stepper'
        assert await js(f"{text}.includes('Live')"), 'live status chip'
        await until(f"{text}.includes(\"Publishing didn't finish\")&&{text}.includes('Finish publishing')")
        await shot('live-overview')
        # Access management is one disclosure on the overview, in plain language.
        await js("Array.from(document.querySelectorAll('summary')).find(summary=>summary.innerText.trim()==='Revoke access').click()")
        await until(f"{text}.includes('Registration acknowledged (historical upload)')")
        assert await js(f"{text}.includes('Stop a distribution service from giving out new copies')")

        # Editing a section autosaves the game's draft; no manual save button exists.
        await js("window.row('Basics').click()")
        await until("!!document.querySelector('#publish-title')")
        assert await js(f"!{text}.includes('SAVE DRAFT')")
        await js("window.setInput('#publish-title','Autosaved title')")
        await until("window.drafts.length===1&&window.drafts[0].payload.form.title==='Autosaved title'")
        assert await js("window.drafts[0].draft_id==='listing:fixture'&&window.drafts[0].kind==='listing'")
        await until(f"{text}.includes('Draft saved on this device')")
        await until(f"{text}.includes('Live · changes not published yet')")
        await js("window.click('< Back to overview')")
        await until("!!window.row('Basics')&&window.row('Basics').innerText.includes('Autosaved title')")

        # Publishing opens a buyer's-view preview first; nothing is sent until confirmed.
        await js("window.click('Publish changes')")
        await until("!!document.querySelector('[data-testid=buyer-preview]')")
        assert await js("document.querySelector('[data-testid=buyer-preview]').textContent.includes('Autosaved title')")
        assert await js("document.querySelector('[data-testid=buyer-preview]').textContent.includes('Game card')&&document.querySelector('[data-testid=buyer-preview]').textContent.includes('Store page')")
        assert await js(f"{text}.includes('Confirm and publish changes')")
        assert await js("document.querySelector('[data-testid=publish-scope]').textContent.includes('Your game and its store page')"), 'basics change needs the full publish'
        await shot('buyer-preview')
        assert await js("!window.calls.some(call=>call.command==='publish_adp_listing_with_contract')")
        await js("const dialog=document.getElementById('publish-buyer-preview-title')?.closest('dialog');if(!dialog)throw 'Missing preview dialog';Array.from(dialog.querySelectorAll('button')).find(button=>button.innerText.trim()==='Keep editing').click()")
        await until("!document.querySelector('[data-testid=buyer-preview]')")

        # Delivery: fee terms are visible without opening advanced options.
        await js("window.row('Delivery & builds').click()")
        await until(f"{text}.includes('How players get your game')||{text}.includes('Arcadestr installs it')")
        assert await js(f"{text}.includes('Service fee terms')"), 'fee terms visible by default'
        await shot('delivery')
        await js("const select=document.querySelector('#publish-fee-terms');select.value='replace';select.dispatchEvent(new Event('change',{bubbles:true}))")
        await until(f"{text}.includes(\"Get the service's terms\")")
        await js("window.click(\"Get the service's terms\")")
        await until(f"{text}.includes('No service fee')")
        await js("window.consent=()=>Array.from(document.querySelectorAll('label')).find(label=>label.textContent.includes('I accept these fee terms')).querySelector('input');window.consent().click()")
        assert await js("window.consent().checked")
        await js("window.click(\"Get the service's terms\")")
        await until("window.calls.filter(call=>call.command==='preview_publisher_terms').length===2")
        await until("!window.consent().checked")
        assert await js("document.body.textContent.includes('What each service may do')&&document.body.textContent.includes('Record purchases')")

        # Store page: one section with tabs for each part.
        await js("window.click('< Back to overview')")
        await until("!!window.row('Store page')")
        await js("window.row('Store page').click()")
        await until("!!Array.from(document.querySelectorAll('.v2-hub-tab')).find(tab=>tab.innerText.includes('Media'))")
        await js("Array.from(document.querySelectorAll('.v2-hub-tab')).find(tab=>tab.innerText.includes('Media')).click()")
        await until(f"{text}.includes('Upload hero')")
        await shot('store-page-media')
        # Requirements point to where platforms are actually set, and link there.
        await js("Array.from(document.querySelectorAll('.v2-hub-tab')).find(tab=>tab.innerText.includes('System requirements')).click()")
        await until(f"{text}.includes('set in Delivery & builds')")
        assert await js(f"!{text}.includes('target above')")
        await js("window.click('Edit platforms >')")
        await until(f"{text}.includes('Arcadestr installs it')&&{text}.includes('Platforms')")
        await js("window.click('< Back to overview')")
        await until("!!window.row('Store page')")

        # A live game whose only change is its store page publishes just the store page.
        await js("window.drafts=[];document.querySelector('[data-testid=fixture-manage]').click()")
        await until("!!window.row('Store page')&&!document.body.textContent.includes('Autosaved title')")
        await js("window.row('Store page').click()")
        await until("!!document.querySelector('.v2-hub-tab')")
        await js("Array.from(document.querySelectorAll('.v2-hub-tab')).find(tab=>tab.innerText.includes('Description')).click()")
        await until("!!document.querySelector('textarea')&&Array.from(document.querySelectorAll('textarea')).some(area=>area.value==='Fixture content')")
        await js("{const area=Array.from(document.querySelectorAll('textarea')).find(area=>area.value==='Fixture content');area.value='Updated store page copy';area.dispatchEvent(new Event('input',{bubbles:true}))}")
        await js("window.click('< Back to overview')")
        await until("Array.from(document.querySelectorAll('button')).some(button=>button.innerText.trim()==='Publish changes'&&!button.disabled)")
        await js("window.click('Publish changes')")
        await until("!!document.querySelector('[data-testid=publish-scope]')")
        assert await js("document.querySelector('[data-testid=publish-scope]').textContent.includes('Only your store page')"), 'store-page-only scope'
        await js("window.click('Confirm and publish changes')")
        await until("window.calls.some(call=>call.command==='publish_store_page')")
        await until(f"{text}.includes(\"Your store page is updated. The rest of your game didn't change.\")")
        published=await js("window.calls.find(call=>call.command==='publish_store_page').args.request")
        assert published['listing_mutations']==[], 'no game changes in a store-page-only publish'
        assert published['draft']['content']['description_markdown']=='Updated store page copy'
        assert published['draft']['loaded_event_id']=='66'*32, 'revises the published page'
        assert await js("!window.calls.some(call=>call.command==='publish_adp_listing_with_contract')"), 'game itself not republished'
        await until("Array.from(document.querySelectorAll('button')).some(button=>button.innerText.trim()==='Publish changes'&&button.disabled)")

        # Advanced store page tools autosave instead of manual draft buttons.
        await js("window.click('Advanced store page tools')")
        await until(f"{text}.includes('Advanced store page tools')&&!!document.querySelector('#store-editor-tab-description')")
        assert await js(f"!{text}.includes('SAVE STORE PAGE DRAFT')&&!{text}.includes('RESTORE SAVED DRAFT')")
        await js("document.querySelector('#store-editor-tab-description').click()")
        await until("Array.from(document.querySelectorAll('.v2-store-page-editor textarea')).some(area=>area.value==='Fixture content')")
        await js("{const area=Array.from(document.querySelectorAll('.v2-store-page-editor textarea')).find(area=>area.value==='Fixture content');area.value='Advanced tools edit';area.dispatchEvent(new Event('input',{bubbles:true}))}")
        await until("window.drafts.some(draft=>draft.kind==='store_page'&&draft.payload.draft.content.description_markdown==='Advanced tools edit')")
        await js("window.click('< Back to game')")
        await until("!!window.row('Store page')")
        assert await js(f"!{text}.includes('Discard unsaved Store Page changes')"), 'autosaved changes do not block leaving'

        # A new game opens on Basics and asks how players get it before anything else is required.
        await js("document.querySelector('[data-testid=fixture-new-game]').click()")
        await until("!!document.querySelector('#publish-title')&&document.querySelector('#publish-title').value===''")
        await js("window.setInput('#publish-title','Brand new game')")
        await js("window.click('< Back to overview')")
        await until("!!window.row('Delivery & builds')&&window.row('Delivery & builds').innerText.includes('Not chosen yet')")
        await until("window.drafts.some(draft=>draft.draft_id.startsWith('local:')&&draft.payload.form.title==='Brand new game')")
        assert await js(f"{text}.includes('Draft · not ready to publish')")
        # An empty store page is publishable but never claims to be finished.
        await until("window.row('Store page').innerText.includes('Can be improved')&&window.row('Store page').innerText.includes('Add a description')")
        await shot('new-overview')
        await js("window.row('Delivery & builds').click()")
        await until(f"{text}.includes('Store page only')")
        await js("Array.from(document.querySelectorAll('[role=radio]')).find(radio=>radio.innerText.includes('Store page only')).click()")
        await js("window.click('< Back to overview')")
        await until("window.row('Delivery & builds').innerText.includes('Ready')")
        await until("window.drafts.some(draft=>draft.draft_id.startsWith('local:')&&draft.payload.delivery_chosen===true)")

        await js("document.querySelector('[data-testid=fixture-account-switch]').click()")
        await until("Array.from(document.querySelectorAll('button')).filter(button=>button.innerText.trim().startsWith('Publish')&&button.innerText.trim()!=='Publish').every(button=>button.disabled)")
        constellations=await js('window.calls')
        forbidden={'publish_adp_listing_with_contract','publish_campaign','revoke_publisher_credential','pay_nwc_invoice','start_blossom_upload','resume_publisher_operation'}
        assert not any(call['command'] in forbidden for call in constellations)
        OUT.joinpath('publisher-hub-console-ui-result.json').write_text(json.dumps(dict(result='PASS',mode='game hub / mock IPC only',calls=constellations,live_actions=0),indent=2))
        print('PASS: game hub overview, per-game finish publishing, autosaved drafts, section editing, buyer preview, store-page-only updates, advanced store page tools autosave, visible fee terms, store page tabs, new-game delivery choice and account fencing; mock IPC only')

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
