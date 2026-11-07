#!/usr/bin/env python3
"""Retired provider-specific live driver; retained for historical evidence only."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import runpy
import sqlite3
import subprocess
import sys
import time

REPO = Path(__file__).resolve().parent.parent
H = runpy.run_path(str(REPO/'scripts/regtest-gate3-denial.py'))
I = runpy.run_path(str(REPO/'scripts/regtest-gate3-integration.py'))
R = runpy.run_path(str(REPO/'scripts/regtest'))


def read(path):
    return json.loads(path.read_text())


def db(root):
    return sqlite3.connect(f'file:{root}/desktop-data/arcadestr/arcadestr.db?mode=ro', uri=True)


def history(node='alice'):
    value = R['lncli'](node,'listpayments','--include_incomplete','--count_total_payments','--max_payments=1000000')
    rows = value.get('payments')
    if not isinstance(rows,list) or 'total_num_payments' not in value or len(rows)!=int(value['total_num_payments']):
        raise RuntimeError('Incomplete wallet history; no dispatch allowed')
    if any(row.get('status') not in ('SUCCEEDED','FAILED','IN_FLIGHT','INITIATED') or not row.get('payment_hash') for row in rows):
        raise RuntimeError('Inconclusive wallet history; no dispatch allowed')
    return rows


def public_history(node='alice'):
    return [{key:row.get(key) for key in ('payment_hash','status','value_msat','fee_msat')} for row in history(node)]


def balance_snapshot():
    nodes = {}
    for node in R['NODES']:
        channels = R['lncli'](node,'listchannels')['channels']
        nodes[node] = {'local_sats':sum(int(c['local_balance']) for c in channels),
            'remote_sats':sum(int(c['remote_balance']) for c in channels),
            'pending_htlcs':sum(len(c.get('pending_htlcs',[])) for c in channels),
            'channel_ids':sorted(c['chan_id'] for c in channels),'outgoing_payments':public_history(node)}
    return nodes


def validate_invoice(invoice, decoded, nodes, seen):
    if (not invoice.lower().startswith('lnbcrt') or int(decoded.get('num_msat',0))!=100000
            or decoded.get('destination') not in {nodes[n]['identity_pubkey'] for n in ('bob','carol')}
            or decoded['destination']==nodes['alice']['identity_pubkey']
            or decoded['payment_hash'] in seen or decoded['destination'] in seen.values()):
        raise RuntimeError('Invoice violates exact amount, regtest destination, self-payment or duplicate guard')
    R['validate_invoice'](invoice,decoded,decoded['destination'],100000)
    if int(decoded['timestamp'])+int(decoded['expiry']) < time.time()+60:
        raise RuntimeError('Original invoice near expiry; stop without replacement')


def permissions(root):
    private = root/'phase3-new-purchase'
    if (private/'paused-by-user.json').exists():
        raise RuntimeError('Purchase execution paused by user; explicit resumption and fresh safety checks required')
    raise RuntimeError('Retired wallet-specific Gate 3 driver. Use the Arcadestr execution plan; no live dispatch is authorized.')


def signer_decisions(root):
    app = read(root/'sign-initial-request.json')['app']
    for row in H['ncli'](root,'bunker','pending','list'):
        if row['app']!=app or row['method'] not in ('get_public_key','sign_event','nip44_encrypt','nip44_decrypt'):
            raise RuntimeError('Unexpected signer request; no decision or payment made')
        if row['method']=='sign_event' and row.get('kind') not in (None,27235):
            raise RuntimeError('Unexpected signing kind; stop')
        H['save'](root/'phase3-new-purchase','signer-approved-'+row['id']+'.json',row)
        H['ncli'](root,'bunker','pending','approve',row['id'],'--always','--for','24h')


def wait(root, predicate, timeout=90):
    end = time.monotonic()+timeout
    while time.monotonic()<end:
        signer_decisions(root)
        if predicate():
            return
        time.sleep(.15)
    raise RuntimeError('Expected state not conclusively verified; stop without redispatch')


def preflight(root, label):
    approval, manifest = permissions(root)
    nodes = R['verify_nodes']()
    R['verify_bitcoin']()
    for name in R['NODES']:
        channels = R['lncli'](name,'listchannels')['channels']
        pending = R['lncli'](name,'pendingchannels')
        if any(c.get('pending_htlcs') for c in channels) or any(v for v in pending.values() if isinstance(v,list)):
            raise RuntimeError('Pending HTLC/channel blocks dispatch')
    if read(root/'phase2-nwc-proxy/network-mode.json')['mode']!='online':
        raise RuntimeError('Pre-payment NWC transport must be conclusively online')
    active = I['invoke'](root,'get_public_key')
    identity = H['ncli'](root,'id','gate3-denial')
    if active!={'ok':True,'value':identity['npub']} or approval['buyer']!=identity['pub_hex']:
        raise RuntimeError('Wrong active buyer; no payment allowed')
    challenge = I['invoke'](root,'regtest_nip44_probe',timeout=45)
    if not challenge.get('ok'):
        raise RuntimeError('Signer unavailable; no dispatch allowed')
    audit = root/'phase3-new-purchase'/('preflight-'+label+'-'+str(time.time_ns()))
    audit.mkdir(mode=0o700)
    (audit/'nwc-setup.env').symlink_to(root/'nwc-setup.env')
    env = I['private_environment'](root)
    env.update(ARCADESTR_REGTEST_INTEGRATION_ROOT=str(audit),ARCADESTR_REGTEST_BUYER_PUBKEY=approval['buyer'],
               ARCADESTR_REGTEST_EXPECTED_LND_PUBKEY=nodes['alice']['identity_pubkey'])
    result = subprocess.run([str(REPO/'target/debug/examples/regtest_gate3_nwc_binding')],env=env,
        capture_output=True,text=True,timeout=30)
    H['save'](audit,'command.json',{'returncode':result.returncode,'stdout':result.stdout,'stderr':result.stderr})
    if result.returncode:
        raise RuntimeError('Fresh wallet/application/network binding audit failed; no dispatch')
    binding = read(audit/'phase2-nwc-binding.json')
    original = read(root/'phase2-nwc-binding.json')
    if any(binding[key]!=original[key] for key in ('buyer','wallet_pubkey','application_pubkey','payment_identity','lnd_pubkey')):
        raise RuntimeError('Wallet/application binding changed')
    return approval,manifest,nodes


def current_invoice(root,index,nodes):
    code = H['js'](root,'return document.querySelector(".v2-detail-invoice code")?.innerText || ""')
    if not code:
        raise RuntimeError('Original GUI invoice unavailable; do not request a replacement')
    decoded = R['lncli']('alice','decodepayreq',code)
    private = root/'phase3-new-purchase'
    seen = {}
    if index:
        first = read(private/'invoice-0.json')
        seen[first['payment_hash']] = first['destination']
    validate_invoice(code,decoded,nodes,seen)
    approval = read(private/'budget-approval.json')
    with db(root) as c:
        rows = c.execute('SELECT a.quote_id,a.attempt_id,a.leg_id,a.state,q.listing_event_id FROM settlement_invoice_attempts a JOIN settlement_quotes q ON q.account_pubkey=a.account_pubkey AND q.quote_id=a.quote_id WHERE a.account_pubkey=? AND a.payment_hash=?',
                         (approval['buyer'],decoded['payment_hash'])).fetchall()
    if len(rows)!=1 or rows[0][3]!='disclosed' or rows[0][4]!=approval['fixture_listing_event_id']:
        raise RuntimeError('Invoice is not the original disclosed attempt of the approved listing/buyer')
    qid,attempt,leg,_,_ = rows[0]
    result = {'quote_id':qid,'attempt_id':attempt,'leg_id':leg,'payment_hash':decoded['payment_hash'],
              'destination':decoded['destination'],'bolt11':code,'invoice_sha256':hashlib.sha256(code.encode()).hexdigest(),
              'amount_msat':100000,'expires_at':int(decoded['timestamp'])+int(decoded['expiry'])}
    path = private/f'invoice-{index}.json'
    if path.exists() and read(path)!=result:
        raise RuntimeError('Original invoice changed; stop without replacement')
    if not path.exists():
        H['save'](private,path.name,result)
    return result


def prepare(root):
    approval,manifest,nodes = preflight(root,'prepare')
    private = root/'phase3-new-purchase'
    with db(root) as c:
        if c.execute('SELECT count(*) FROM settlement_quotes').fetchone()[0] or c.execute('SELECT count(*) FROM nwc_payment_journal').fetchone()[0]:
            raise RuntimeError('Purchase preparation is one-shot; retain existing records')
    if approval['fixture_listing_event_id'] not in H['js'](root,'return document.body.innerText'):
        raise RuntimeError('Actual GUI is not on the approved new listing')
    with (private/'purchase-prepare-admitted').open('x') as output:
        output.write('Only approved listing; no payment dispatched by preparation')
    H['js'](root,"const e=document.getElementById('split-refund-lud16');e.value='alice@127.0.0.1:18080';e.dispatchEvent(new Event('input',{bubbles:true}));return true;")
    H['js'](root,"const b=[...document.querySelectorAll('button')].find(b=>b.innerText.toLowerCase().includes('buy with lightning')&&!b.disabled);if(!b)throw Error('Buy unavailable');b.click();return true;")
    wait(root,lambda: H['js'](root,'return !!document.querySelector(".v2-detail-invoice code")?.innerText'))
    invoice = current_invoice(root,0,nodes)
    H['save'](private,'purchase.json',{'buyer':approval['buyer'],'quote_id':invoice['quote_id'],
        'listing_event_id':approval['fixture_listing_event_id'],'server_url':manifest['server_url']})
    H['snapshot'](root,'phase3-original-first-invoice-prepared')


def admit(root,index):
    approval,manifest,nodes = preflight(root,'dispatch-'+str(index))
    private = root/'phase3-new-purchase'
    if len(list(private.glob('dispatch-*.admitted.json')))!=index:
        raise RuntimeError('Dispatch admission already used or out of order; no retry')
    invoice = current_invoice(root,index,nodes)
    purchase = read(private/'purchase.json')
    if invoice['quote_id']!=purchase['quote_id']:
        raise RuntimeError('Quote changed; no dispatch')
    with db(root) as c:
        if c.execute('SELECT count(*) FROM nwc_payment_journal WHERE account_pubkey=? AND payment_hash=?',
                     (approval['buyer'],invoice['payment_hash'])).fetchone()[0]:
            raise RuntimeError('Existing intent prohibits another dispatch; recovery only')
        if c.execute('SELECT COALESCE(sum(dispatch_count),0) FROM nwc_payment_journal WHERE account_pubkey=?',
                     (approval['buyer'],)).fetchone()[0]!=index:
            raise RuntimeError('Unexpected durable dispatch counter; stop')
    if any(p['payment_hash']==invoice['payment_hash'] for p in history()):
        raise RuntimeError('Existing wallet outcome never authorizes redispatch')
    if index and not (private/'first-leg-verified.json').exists():
        raise RuntimeError('First leg must be conclusively paid and acknowledged before second dispatch')
    if index==0:
        if (private/'balances-and-payments-before.json').exists():
            raise RuntimeError('Pre-payment snapshot already exists; retain state, do not restart setup')
        H['save'](private,'balances-and-payments-before.json',balance_snapshot())
    else:
        initial=read(private/'balances-and-payments-before.json')['alice']['outgoing_payments']
        first=read(private/'first-leg-verified.json')
        current=public_history()
        remaining=[p for p in current if p['payment_hash']!=first['payment_hash']]
        if remaining!=initial or len(current)!=len(initial)+1:
            raise RuntimeError('Unexpected wallet-history changes; no second dispatch')
        if first['fee_msat']>50000:
            raise RuntimeError('Approved fee reserve exhausted; no second dispatch')
    with (private/f'dispatch-{index}.admitted.json').open('x') as output:
        json.dump({'buyer':approval['buyer'],'app_id':approval['app_id'],'quote_id':invoice['quote_id'],
                   'payment_hash':invoice['payment_hash'],'invoice_sha256':invoice['invoice_sha256'],
                   'amount_msat':100000,'explicit_budget_msat':250000,'automatic_retry_allowed':False},output)
    phase = 'phase3-first-payment' if index==0 else 'phase3-second-payment-reply-loss'
    H['save'](root/'phase2-nwc-proxy','network-mode.json',{'mode':'online' if index==0 else 'drop-responses','phase':phase})
    H['js'](root,"const b=[...document.querySelectorAll('button')].find(b=>b.innerText.trim()==='Pay this leg'&&!b.disabled);if(!b)throw Error('Pay control unavailable');b.click();return true;")
    return invoice


def paid(invoice):
    rows = [p for p in history() if p['payment_hash']==invoice['payment_hash']]
    if not rows:
        return None
    if len(rows)!=1 or rows[0]['status'] not in ('IN_FLIGHT','INITIATED','SUCCEEDED'):
        raise RuntimeError('Payment failed or verification inconclusive; never retry')
    row = rows[0]
    if row['status']!='SUCCEEDED':
        return None
    proof = row.get('payment_preimage','')
    if (int(row.get('value_msat',0))!=100000 or len(proof)!=64
            or hashlib.sha256(bytes.fromhex(proof)).hexdigest()!=invoice['payment_hash']):
        raise RuntimeError('Wallet proof verification failed; stop without redispatch')
    return {'payment_hash':invoice['payment_hash'],'status':'SUCCEEDED','amount_msat':100000,
            'fee_msat':int(row['fee_msat']),'proof_valid':True}


def journal(root,invoice,require_result=None):
    approval=read(root/'phase3-new-purchase/budget-approval.json')
    binding=read(root/'phase2-nwc-binding.json')
    with db(root) as c:
        row=c.execute('SELECT wallet_identity,invoice_commitment,amount_msat,encrypted_invoice,encrypted_payment,dispatch_count,lookup_count FROM nwc_payment_journal WHERE account_pubkey=? AND payment_hash=?',
                      (approval['buyer'],invoice['payment_hash'])).fetchone()
    if (not row or row[0]!=binding['payment_identity'] or row[1]!=invoice['invoice_sha256']
            or row[2]!=100000 or not row[3] or row[5]!=1
            or invoice['bolt11'].encode() in row[3]):
        raise RuntimeError('Durable buyer/wallet/application/invoice/amount binding invalid; stop')
    if require_result is not None and bool(row[4])!=require_result:
        raise RuntimeError('Unexpected proof-checkpoint state; retain evidence, no redispatch')
    return {'buyer':approval['buyer'],'wallet_identity':row[0],'invoice_commitment':row[1],
            'amount_msat':row[2],'encrypted_invoice_present':True,'encrypted_invoice_sha256':hashlib.sha256(row[3]).hexdigest(),
            'encrypted_result_present':bool(row[4]),'dispatch_count':row[5],'lookup_count':row[6]}


def first(root):
    invoice = admit(root,0)
    def done():
        with db(root) as c:
            row = c.execute('SELECT state,encrypted_proof IS NOT NULL FROM settlement_invoice_attempts WHERE attempt_id=?',
                            (invoice['attempt_id'],)).fetchone()
        code = H['js'](root,'return document.querySelector(".v2-detail-invoice code")?.innerText || ""')
        return row==('settled',1) and bool(code) and code!=invoice['bolt11']
    wait(root,done)
    result = paid(invoice)
    if result is None or result['fee_msat']>50000:
        raise RuntimeError('First payment not conclusively within budget; no second dispatch')
    result['durable_journal']=journal(root,invoice,True)
    current_invoice(root,1,R['verify_nodes']())
    H['save'](root/'phase3-new-purchase','first-leg-verified.json',result)
    H['snapshot'](root,'phase3-first-leg-paid-and-acknowledged')


if __name__=='__main__':
    os.umask(0o077)
    if len(sys.argv)!=3 or sys.argv[1] not in ('prepare','first'):
        raise SystemExit('usage: regtest-gate3-purchase.py {prepare|first} PRIVATE_ROOT')
    root = Path(sys.argv[2]).resolve(strict=True)
    with (root/'phase3-new-purchase/controller.lock').open('a') as lock:
        fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
        globals()[sys.argv[1]](root)
    print('PASS approved purchase stage; no automatic redispatch path')
