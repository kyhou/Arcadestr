#!/usr/bin/env python3
"""One-shot capped setup through the existing desktop signer. Never a recovery payer."""
import hashlib
import json
import os
from pathlib import Path
import runpy
import sqlite3
import subprocess
import sys

REPO = Path(__file__).resolve().parent.parent
NPUB = 'npub1xmt6ra5afgf8eeh65g5v42y0qkarq3qv73kfn6ffjdq73j2g6d8s0ggn5h'
PUBKEY = '36d7a1f69d4a127ce6faa228caa88f05ba30440cf46c99e9299341e8c948d34f'


def complete_history(reg):
    response = reg['lncli']('bob', 'listpayments', '--include_incomplete',
                            '--count_total_payments', '--max_payments=1000000')
    payments = response.get('payments')
    if (not isinstance(payments, list) or 'total_num_payments' not in response or
            len(payments) != int(response['total_num_payments']) or
            any(not p.get('payment_hash') or p.get('status') not in
                ('SUCCEEDED', 'FAILED', 'IN_FLIGHT', 'INITIATED') for p in payments)):
        raise RuntimeError('Wallet history incomplete or inconclusive; no payment allowed')
    return payments


def validate(decoded, invoice, nodes, payer, seen):
    if not invoice.lower().startswith('lnbcrt'):
        raise RuntimeError('Only regtest invoices allowed')
    destination = decoded.get('destination')
    if destination not in {v['identity_pubkey'] for v in nodes.values()}:
        raise RuntimeError('Destination is not an existing verified regtest node')
    if destination == nodes[payer]['identity_pubkey']:
        raise RuntimeError('Self-payment rejected')
    if int(decoded.get('num_msat', 0)) != 100000:
        raise RuntimeError('Each transfer must be exactly 100 sats')
    if destination not in {nodes[n]['identity_pubkey'] for n in ('alice', 'carol')}:
        raise RuntimeError('Unexpected fixture recipient')
    if decoded['payment_hash'] in seen or destination in seen.values():
        raise RuntimeError('Duplicate invoice or recipient rejected')


def main():
    os.umask(0o077)
    if len(sys.argv) != 3 or sys.argv[1] not in ('stage-once', 'continue-unpaid-setup', 'continue-rejected-command', 'finish-second-transfer'):
        raise RuntimeError('Only explicit setup is supported; no recovery payments')
    root = Path(sys.argv[2]).resolve(strict=True)
    parent = root.parent
    if parent != Path.home()/'.local/state/arcadestr-regtest/gate3-runs/1793337474483978484':
        raise RuntimeError('Existing private run required')
    if root.stat().st_mode & 0o077:
        raise RuntimeError('Owner-only evidence directory required')
    harness = runpy.run_path(str(REPO/'scripts/regtest-gate3-remaining.py'))
    reg = runpy.run_path(str(REPO/'scripts/regtest'))
    save = harness['save']

    def invoke(command, request=None):
        args = {} if request is None else {'request': request}
        script = ('const done=arguments[arguments.length-1];'
                  f'window.__TAURI_INTERNALS__.invoke({json.dumps(command)},{json.dumps(args)})'
                  '.then(value=>done({ok:true,value})).catch(error=>done({ok:false,error:String(error)}));')
        result = harness['gui'](parent, script)
        if not result.get('ok'):
            save(root/(command+'-failure.json'), result)
            raise RuntimeError('Desktop operation failed: '+command+'; retained state, do not retry setup')
        return result.get('value')

    if invoke('get_public_key') != NPUB:
        raise RuntimeError('Active desktop identity differs from authorized NIP-46 account')
    nodes = reg['verify_nodes']()
    reg['verify_bitcoin']()
    manifest = json.loads((root/'fixture-ready.json').read_text())
    # This marker is global to the approved run, not per failed server attempt.
    marker = parent/'nip46-setup-transfers-started'
    corrected = sys.argv[1] == 'continue-rejected-command'
    finish = sys.argv[1] == 'finish-second-transfer'
    if finish:
        if (marker.read_text() != str(root) or not (root/'transfer-0-verified.json').is_file() or
                (root/'transfer-1-admitted').exists()):
            raise RuntimeError('Second transfer requires verified first leg and unused second admission')
    elif corrected:
        raw = json.loads((root/'transfer-0-raw.json').read_text())
        if (marker.read_text() != str(root) or raw['returncode'] != 1 or
                'flag provided but not defined: -fee_limit_sat' not in raw['stderr'] or
                (root/'transfer-1-admitted').exists()):
            raise RuntimeError('Only confirmed pre-dispatch option rejection may continue')
        # Human authorized this one corrected command. Retain the failed admission.
        with (root/'corrected-command-started').open('x') as f:
            f.write('Explicit human approval; original invoice only; no uncertain-result retry')
    elif sys.argv[1] == 'continue-unpaid-setup':
        if (not marker.is_file() or marker.read_text() != str(root) or
                list(parent.glob('nip46-*/transfer-*-admitted')) or (root/'purchase.json').exists()):
            raise RuntimeError('Continuation permitted only before any payment admission or stored quote')
    else:
        with marker.open('x') as f:
            f.write(str(root))
    db = parent/'desktop-data/arcadestr/arcadestr.db'
    with sqlite3.connect(f'file:{db}?mode=ro', uri=True) as c:
        if not (root/'desktop-before-setup.db').exists():
            backup = sqlite3.connect(root/'desktop-before-setup.db')
            c.backup(backup)
            backup.close()
    save(root/'setup-reservation.json', {'buyer': PUBKEY, 'payer': 'bob',
         'maximum_transfers': 2, 'principal_msat': 200000, 'fee_limit_msat_per_transfer': 0,
         'reserved_msat': 200000, 'retained_even_if_ambiguous': True,
         'initial_signer_approval': 'existing automatic authorization; not per-request manual approval'})
    request = {k: manifest[k] for k in ('server_url', 'publisher_npub', 'listing_id')}
    request.update(refund_lud16='bob@127.0.0.1:18080', purchase_id=None)
    if corrected or finish:
        quote = json.loads((root/'purchase.json').read_text())
    else:
        quote = invoke('start_split_purchase', request)
        save(root/'purchase.json', quote)
    signed = quote['quote']
    if (signed['buyer_pubkey'] != PUBKEY or signed['gross_msat'] != 200000 or
            len(signed['legs']) != 2 or any(leg['amount_msat'] != 100000 for leg in signed['legs'])):
        raise RuntimeError('Quote violates authorized identity or two-leg cap; no payment sent')
    qid = quote['quote_id']
    with sqlite3.connect(f'file:{db}?mode=ro', uri=True) as c:
        assert c.execute('SELECT account_pubkey FROM settlement_quotes WHERE quote_id=?', (qid,)).fetchall() == [(PUBKEY,)]
    seen = {}
    if finish:
        original = json.loads((root/'invoice-0.json').read_text())
        decoded = reg['lncli']('bob', 'decodepayreq', original['bolt11'])
        matches = [p for p in complete_history(reg) if p['payment_hash'] == original['payment_hash']]
        if (len(matches) != 1 or matches[0]['status'] != 'SUCCEEDED' or
                int(matches[0].get('value_msat', -1)) != 100000):
            raise RuntimeError('First transfer not conclusively settled; no second payment allowed')
        with sqlite3.connect(f'file:{db}?mode=ro', uri=True) as c:
            row = c.execute('SELECT encrypted_proof IS NOT NULL,state FROM settlement_invoice_attempts WHERE account_pubkey=? AND attempt_id=?', (PUBKEY, original['attempt_id'])).fetchone()
            if row != (1, 'settled'):
                raise RuntimeError('First proof not durably settled; no second payment allowed')
        seen[original['payment_hash']] = decoded['destination']
    for index in range(1 if finish else 0, 2):
        assert invoke('get_public_key') == NPUB
        if corrected and index == 0:
            inv = json.loads((root/'invoice-0.json').read_text())
            history = complete_history(reg)
            if any(p['payment_hash'] == inv['payment_hash'] for p in history):
                raise RuntimeError('Original invoice has wallet history; correction prohibited')
        else:
            inv = invoke('next_split_invoice', {'server_url': request['server_url'], 'quote_id': qid})
        decoded = reg['lncli']('bob', 'decodepayreq', inv['bolt11'])
        validate(decoded, inv['bolt11'], nodes, 'bob', seen)
        reg['validate_invoice'](inv['bolt11'], decoded, decoded['destination'], 100000)
        assert inv['payment_hash'] == decoded['payment_hash'] and inv['amount_msat'] == 100000
        history = complete_history(reg)
        if any(p['payment_hash'] == inv['payment_hash'] for p in history):
            raise RuntimeError('Invoice already has wallet history; no dispatch allowed')
        seen[decoded['payment_hash']] = decoded['destination']
        save(root/f'invoice-{index}.json', inv)
        # Admission is durable BEFORE invoking LND. No failure or restart retries it.
        admission = f'transfer-{index}-corrected-admitted' if corrected and index == 0 else f'transfer-{index}-admitted'
        with (root/admission).open('x') as f:
            f.write(decoded['payment_hash'])
        command = ['docker', 'exec', 'polar-n1-bob', 'lncli', '--lnddir=/home/lnd/.lnd',
                   '--network=regtest', 'payinvoice', '--force', '--fee_limit=0', inv['bolt11']]
        paid = subprocess.run(command, capture_output=True, text=True, timeout=90)
        raw_name = f'transfer-{index}-corrected-raw.json' if corrected and index == 0 else f'transfer-{index}-raw.json'
        save(root/raw_name, {'returncode': paid.returncode, 'stdout': paid.stdout, 'stderr': paid.stderr})
        if paid.returncode:
            raise RuntimeError('Setup transfer failed/ambiguous; reservation retained, never retry')
        matches = [p for p in complete_history(reg) if p['payment_hash'] == inv['payment_hash']]
        if len(matches) != 1 or matches[0]['status'] != 'SUCCEEDED':
            raise RuntimeError('Payment history is not conclusively successful; never retry')
        result = matches[0]
        preimage = result.get('payment_preimage', '')
        if (result.get('payment_error') or len(preimage) != 64 or
                hashlib.sha256(bytes.fromhex(preimage)).hexdigest() != decoded['payment_hash']):
            raise RuntimeError('Invalid/ambiguous setup proof; stop without retry')
        matches = [p for p in complete_history(reg) if p['payment_hash'] == inv['payment_hash']]
        if (len(matches) != 1 or matches[0]['status'] != 'SUCCEEDED' or
                int(matches[0].get('value_msat', -1)) != 100000 or
                int(matches[0].get('fee_msat', -1)) != 0):
            raise RuntimeError('Payment history confirmation inconclusive; stop, never redispatch')
        if index == 1:
            (root/'fixture/gui-pause-purchase-proof').touch(exist_ok=False)
        try:
            invoke('submit_split_payment', {'server_url': request['server_url'], 'quote_id': qid,
                   'attempt_id': inv['attempt_id'], 'preimage': preimage})
        except RuntimeError:
            if index != 1:
                raise
        with sqlite3.connect(f'file:{db}?mode=ro', uri=True) as c:
            row = c.execute('SELECT encrypted_proof IS NOT NULL,state FROM settlement_invoice_attempts WHERE account_pubkey=? AND attempt_id=?', (PUBKEY, inv['attempt_id'])).fetchone()
            assert row and row[0]
            if index == 0:
                assert row[1] == 'settled'
        save(root/f'transfer-{index}-verified.json', {'payment_hash': decoded['payment_hash'],
             'destination': decoded['destination'], 'principal_msat': 100000,
             'proof_valid': True, 'encrypted_checkpoint_present': True})
    save(root/'ready-for-signer-stop.json', {'buyer': PUBKEY, 'quote_id': qid,
         'setup_transfers': 2, 'setup_principal_msat': 200000, 'recovery_transfers': 0,
         'proof_submission_paused': True, 'next': 'Human stops existing bunker; do not change authorized-client list'})
    print('Two 100-sat setup transfers verified; encrypted NIP-46 checkpoint retained. PAUSE for human bunker stop.')


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print('BLOCKED:', type(error).__name__, str(error) if isinstance(error, RuntimeError) else 'inspect retained private evidence; do not retry payments')
        sys.exit(1)
