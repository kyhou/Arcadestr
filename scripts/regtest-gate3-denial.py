#!/usr/bin/env python3
"""Non-paying ncli authorization-denial GUI harness; all state is isolated."""
import json
import os
from pathlib import Path
import subprocess
import time
import urllib.request

REPO = Path(__file__).resolve().parent.parent


def save(root, name, value):
    path = root/name
    path.write_text(json.dumps(value, indent=2))
    path.chmod(0o600)


def ncli(root, *args):
    env = os.environ.copy()
    env.update(HOME=str(root/'ncli-home'), XDG_CONFIG_HOME=(root/'config-alias').read_text(),
               NCLI_VAULT_PASSWORD=(root/'vault-password').read_text())
    result = subprocess.run([str(root/'bin/ncli'), *args, '--json'], env=env, cwd=root,
                            capture_output=True, text=True, timeout=15)
    if result.returncode:
        save(root, 'ncli-last-failure-private.json', {'args': list(args), 'stderr': result.stderr})
        raise RuntimeError('ncli operation failed; inspect protected diagnostics')
    value = json.loads(result.stdout) if result.stdout.strip() else None
    return value.get('result', value) if isinstance(value, dict) else value


def request(path, data=None, method=None, timeout=20):
    req = urllib.request.Request('http://127.0.0.1:4446'+path,
        data=None if data is None else json.dumps(data).encode(), method=method,
        headers={'Content-Type': 'application/json'})
    with urllib.request.urlopen(req, timeout=timeout) as response:
        value = json.load(response)['value']
    if isinstance(value, dict) and value.get('error'):
        raise RuntimeError('WebDriver error; do not publish page contents')
    return value


def js(root, script, args=None):
    sid = json.loads((root/'gui-session.json').read_text())['session_id']
    return request(f'/session/{sid}/execute/sync', {'script': script, 'args': args or []})


def snapshot(root, name):
    import base64
    data = js(root, 'return {text:document.body.innerText,url:location.href}')
    # Never call this while pairing credentials are displayed.
    save(root, name+'.json', data)
    sid = json.loads((root/'gui-session.json').read_text())['session_id']
    try:
        image = request(f'/session/{sid}/screenshot', timeout=5)
        (root/(name+'.png')).write_bytes(base64.b64decode(image))
    except (OSError, RuntimeError):
        save(root, name+'-screenshot-unavailable.json', {'actual_dom_retained': True})
    return data


def pending(root, method, timeout=12):
    deadline = time.monotonic()+timeout
    while time.monotonic() < deadline:
        value = ncli(root, 'bunker', 'pending', 'list')
        rows = value if isinstance(value, list) else value.get('pending', value.get('requests', []))
        known = root/'sign-initial-request.json'
        if method != 'connect' and known.exists():
            app = json.loads(known.read_text())['app']
            for row in rows:
                if row['method'] == 'connect' and row['app'] == app:
                    ncli(root, 'bunker', 'pending', 'approve', row['id'], '--always', '--for', '24h')
        matches = [row for row in rows if row.get('method') == method]
        if len(matches) == 1:
            return matches[0]
        if len(matches) > 1:
            raise RuntimeError('Ambiguous pending request; no decision made')
        time.sleep(.1)
    raise RuntimeError('Expected actual NIP-46 request did not arrive')


def wait_gui(root, predicate, timeout=12):
    deadline = time.monotonic()+timeout
    while time.monotonic() < deadline:
        if js(root, 'return !!('+predicate+')'):
            return
        time.sleep(.1)
    raise RuntimeError('Actual GUI did not reach expected state')


def purchases(root):
    if not js(root, 'return !!document.querySelector("[data-testid=refund-processor]")'):
        wait_gui(root, '[...document.querySelectorAll("button")].some(b=>b.innerText.includes("npub")&&b.innerText.includes("expand_more"))')
        js(root, "[...document.querySelectorAll('button')].find(b=>b.innerText.includes('npub')&&b.innerText.includes('expand_more')).click();return true;")
        js(root, "[...document.querySelectorAll('button')].find(b=>b.innerText.trim()==='Purchases').click();return true;")
    wait_gui(root, 'document.querySelector("[data-testid=refund-processor]")')
    fixture = json.loads((root/'active-fixture.json').read_text())
    js(root, "const e=document.querySelector('[data-testid=refund-processor]');e.value=arguments[0];e.dispatchEvent(new Event('input',{bubbles:true}));return true;", [fixture['server_url']])


def remove_grant(root, app, method):
    for grant in ncli(root, 'bunker', 'sessions', 'grants', app):
        if grant['method'] == method:
            args = ['bunker', 'sessions', 'revoke-grant', app, '--method', method]
            if 'kind' in grant:
                args += ['--kind', str(grant['kind'])]
            ncli(root, *args)
    grants = ncli(root, 'bunker', 'sessions', 'grants', app)
    if any(g['method'] == method for g in grants):
        raise RuntimeError('Auto-approval grant remains; denial test stopped')
    return grants


def signing_refusal_cycle(root):
    purchases(root)
    app = json.loads((root/'sign-initial-request.json').read_text())['app']
    save(root, 'sign-fixed-grants-revoked.json', remove_grant(root, app, 'sign_event'))
    js(root, "document.querySelector('[data-testid=load-refunds]').click();return true;")
    # A restored lazy session can require a fresh handshake using the SAME saved app key.
    rows = ncli(root, 'bunker', 'pending', 'list')
    for row in rows:
        if row['method'] == 'connect' and row['app'] == app:
            ncli(root, 'bunker', 'pending', 'approve', row['id'], '--always', '--for', '24h')
    row = pending(root, 'sign_event', timeout=25)
    if row['app'] != app:
        raise RuntimeError('Wrong client; no rejection decision made')
    save(root, 'sign-fixed-denied-request.json', row)
    start = time.monotonic()
    ncli(root, 'bunker', 'pending', 'reject', row['id'])
    wait_gui(root, 'document.body.innerText.includes("Remote signer denied authorization")')
    elapsed = time.monotonic()-start
    data = snapshot(root, 'sign-fixed-explicit-denial')
    if 'timed out' in data['text'].lower() or 'disconnected' in data['text'].lower():
        raise RuntimeError('GUI misclassified refusal')
    save(root, 'sign-fixed-denial-result.json', {'request_id': row['id'], 'method': 'sign_event',
         'explicit_cli_reject': True, 'gui_denial_observed': True, 'seconds_after_reject': elapsed})
    js(root, "document.querySelector('[data-testid=load-refunds]').click();return true;")
    row = pending(root, 'sign_event')
    ncli(root, 'bunker', 'pending', 'approve', row['id'], '--always', '--for', '24h')
    wait_gui(root, 'document.body.innerText.includes("No obligations are owed by this account at this processor.")')
    snapshot(root, 'sign-fixed-permission-restored')
    save(root, 'sign-fixed-regrant-result.json', {'request_id': row['id'], 'gui_recovered': True})


def pair_and_initialize(root):
    wait_gui(root, 'document.body.innerText.includes("DISCOVER") || document.body.innerText.includes("Connect another account") || document.getElementById("bunker-uri")', timeout=25)
    if js(root, 'return document.body.innerText.includes("DISCOVER")'):
        wait_gui(root, '[...document.querySelectorAll("button")].some(b=>b.innerText.includes("npub")&&b.innerText.includes("expand_more"))')
        js(root, "[...document.querySelectorAll('button')].find(b=>b.innerText.includes('npub')&&b.innerText.includes('expand_more')).click();return true;")
        js(root, "[...document.querySelectorAll('button')].find(b=>b.innerText.trim()==='Sign out').click();return true;")
    if not js(root, 'return !!document.getElementById("bunker-uri")'):
        wait_gui(root, 'document.body.innerText.includes("Connect another account")')
        js(root, "[...document.querySelectorAll('button')].find(b=>b.innerText.includes('Connect another account')).click();return true;")
    uri = ncli(root, 'bunker', 'connect')
    save(root, 'matrix-pairing-private.json', {'uri': uri})
    wait_gui(root, 'document.getElementById("bunker-uri")')
    js(root, "for(const [id,value] of [['bunker-uri',arguments[0]],['bunker-name','gate3-denial-matrix']]){const e=document.getElementById(id);e.value=value;e.dispatchEvent(new Event('input',{bubbles:true}));} [...document.querySelectorAll('button')].find(b=>b.innerText==='Connect remote signer').click();return true;", [uri])
    for method in ['connect', 'get_public_key']:
        row = pending(root, method)
        ncli(root, 'bunker', 'pending', 'approve', row['id'], '--always', '--for', '24h')
    wait_gui(root, 'document.body.innerText.includes("DISCOVER")')
    purchases(root)
    js(root, "document.querySelector('[data-testid=load-refunds]').click();return true;")
    row = pending(root, 'sign_event')
    previous = root/'sign-initial-request.json'
    if previous.exists():
        save(root, 'sign-initial-retained-'+str(time.time_ns())+'.json', json.loads(previous.read_text()))
    save(root, 'sign-initial-request.json', row)
    ncli(root, 'bunker', 'pending', 'approve', row['id'], '--always', '--for', '24h')
    wait_gui(root, 'document.body.innerText.includes("No obligations are owed by this account at this processor.")')
    snapshot(root, 'matrix-initial-sign-approved')


def decryption_refusal_cycle(root):
    app = json.loads((root/'sign-initial-request.json').read_text())['app']
    if not js(root, 'return !!document.querySelector("[data-testid=regtest-nip44-probe]")'):
        js(root, "[...document.querySelectorAll('button')].find(b=>b.innerText.trim()==='Debug').click();return true;")
    wait_gui(root, 'document.querySelector("[data-testid=regtest-nip44-probe]")')
    def click():
        js(root, "document.querySelector('[data-testid=regtest-nip44-probe]').click();return true;")
    def decision(verb, name, always=False):
        row = pending(root, 'nip44_decrypt')
        if row['app'] != app:
            raise RuntimeError('Wrong decryption client; no decision made')
        save(root, name+'-request.json', row)
        start = time.monotonic()
        args = ['bunker', 'pending', verb, row['id']]
        if always:
            args += ['--always', '--for', '24h']
        ncli(root, *args)
        expected = 'Remote signer denied authorization' if verb == 'reject' else 'NIP-44 decryption verified.'
        wait_gui(root, 'document.querySelector("[data-testid=regtest-nip44-result]").innerText.includes('+json.dumps(expected)+')')
        save(root, name+'-result.json', {'request_id': row['id'], 'method': row['method'],
             'decision': verb, 'seconds_after_decision': time.monotonic()-start,
             'gui_expected_text': expected, 'gui_observed': True})
        snapshot(root, name)
    click()
    decision('approve', 'decrypt-initial-approved', always=True)
    save(root, 'decrypt-grants-revoked.json', remove_grant(root, app, 'nip44_decrypt'))
    click()
    decision('reject', 'decrypt-explicit-denial')
    click()
    decision('approve', 'decrypt-permission-restored', always=True)


def audit_wire(root):
    requests = {}
    for name, verdict in [('sign-fixed-denial', 'reject'), ('sign-fixed-regrant', 'approve'),
            ('decrypt-initial-approved', 'approve'), ('decrypt-explicit-denial', 'reject'),
            ('decrypt-permission-restored', 'approve')]:
        row = json.loads((root/(name+'-result.json')).read_text())
        requests[row['request_id']] = verdict
    if (root/'actual-nip46-wire-verification.json').exists():
        audit = json.loads((root/'actual-nip46-wire-verification.json').read_text())
        if set(audit['requests']) == set(requests):
            return
        raise RuntimeError('Different wire audit already exists; retain it, do not overwrite')
    # Reveal stays only in process memory/the child environment. Never log it or
    # put it in argv, the GUI, or the durable non-secret verification artifacts.
    identity = ncli(root, 'id', 'gate3-denial', '--reveal')
    app = json.loads((root/'sign-initial-request.json').read_text())['app']
    env = os.environ.copy()
    env.update(ARCADESTR_REGTEST_DENIAL_ROOT=str(root),
        ARCADESTR_REGTEST_DENIAL_IDENTITY_KEY=identity['priv_hex'],
        ARCADESTR_REGTEST_DENIAL_APP=app, ARCADESTR_REGTEST_DENIAL_REQUESTS=json.dumps(requests))
    result = subprocess.run([str(REPO/'target/debug/examples/regtest_gate3_denial_wire')],
        env=env, capture_output=True, text=True, timeout=20)
    save(root, 'wire-audit-command.json', {'returncode': result.returncode,
        'stdout': result.stdout, 'stderr': result.stderr})
    if result.returncode:
        raise RuntimeError('Actual NIP-46 wire audit incomplete; no success claimed')


def verify(root):
    import hashlib
    import sqlite3
    baseline = json.loads((root/'protected-baseline.json').read_text())
    changed = [path for path, digest in baseline.items()
               if not Path(path).is_file() or hashlib.sha256(Path(path).read_bytes()).hexdigest() != digest]
    if changed:
        save(root, 'protected-state-mismatch-private.json', changed)
        raise RuntimeError('Protected existing state changed; stop and retain evidence')
    status = ncli(root, 'bunker', 'status')
    save(root, 'final-bunker-status.json', status)
    identity = ncli(root, 'id', 'gate3-denial')
    initial = json.loads((root/'identity-private.json').read_text())
    initial = initial.get('result', initial)
    if (identity['pub_hex'] != initial['pub_hex'] or status['identity_pub'] != identity['pub_hex']
            or not status['running'] or status['pending_count'] != 0):
        raise RuntimeError('Isolated persistent signer identity/readiness is not verified')
    relays = status['relays']
    if relays != ['ws://127.0.0.1:7778']:
        raise RuntimeError('Signer relay scope differs from required local relay')
    history = ncli(root, 'bunker', 'history')
    save(root, 'final-request-history-private.json', history)
    wire = json.loads((root/'actual-nip46-wire-verification.json').read_text())
    if not wire['signatures_verified'] or wire['only_relay'] != 'ws://127.0.0.1:7778':
        raise RuntimeError('Wire signatures or relay scope not verified')
    cases = {}
    for name in ['sign-fixed-denial', 'decrypt-explicit-denial']:
        result = json.loads((root/(name+'-result.json')).read_text())
        rows = [entry for entry in history if entry['id'] == result['request_id']]
        if len(rows) != 1 or rows[0]['verdict'] != 2 or rows[0]['expired']:
            raise RuntimeError('Refusal was not conclusively an explicit, non-expired rejection')
        if (wire['requests'][result['request_id']]['method'] != result['method'] or
                wire['responses'][result['request_id']]['error'] != 'denied: rejected' or
                wire['responses'][result['request_id']]['result_present']):
            raise RuntimeError('Actual wire response does not prove refusal')
        cases[result['method']] = {'request_id': result['request_id'], 'verdict': 'explicitly rejected',
            'expired': False, 'gui_denial': True, 'permission_recovery_verified': True,
            'response_error': 'denied: rejected'}
    c = sqlite3.connect(f'file:{root}/desktop-data/arcadestr/arcadestr.db?mode=ro', uri=True)
    counts = {table:c.execute('SELECT count(*) FROM '+table).fetchone()[0] for table in
        ['nwc_payment_journal','settlement_invoice_attempts','settlement_quotes','client_refund_attempts','purchases','download_tokens']}
    c.close()
    if any(counts.values()):
        raise RuntimeError('Unexpected payment/purchase state in isolated desktop')
    report = {'authorization_denial_gui_verified': True, 'methods': cases, 'only_relay': relays[0],
        'existing_protected_file_count': len(baseline), 'existing_protected_files_unchanged': True,
        'isolated_table_counts': counts, 'lightning_payments': 0,
        'original_nak_bunker_and_nip46_account_untouched': True,
        'persistent_isolated_signer_identity_verified': True,
        'ncli': json.loads((root/'ncli-revision.json').read_text()),
        'nip44_probe_scope': 'opt-in debug GUI challenge, cryptographically validated; no business-state writes',
        'actual_signed_request_response_pairs_audited': len(wire['responses']),
        'restored_session_note': 'one restored isolated session failed before any pending request; not counted as refusal evidence; matrix used fresh GUI pairing to same persistent isolated signer',
        'nip46_nwc_durability_verified': False, 'complete_purchase_recovery_verified': False,
        'gate3_closed': False}
    save(root, 'authorization-denial-verification.json', report)
    return report


if __name__ == '__main__':
    import sys
    os.umask(0o077)
    if len(sys.argv) != 3 or sys.argv[1] not in ('sign-cycle', 'matrix', 'verify'):
        raise SystemExit('usage: regtest-gate3-denial.py {sign-cycle|matrix|verify} PRIVATE_ROOT')
    root = Path(sys.argv[2]).resolve(strict=True)
    if sys.argv[1] == 'verify':
        verify(root)
        print('PASS saved actual refusals, isolated zero-payment state and unchanged protected files')
        raise SystemExit(0)
    if sys.argv[1] == 'matrix':
        if (root/'authorization-denial-verification.json').exists():
            raise SystemExit('Already verified; use verify instead of repeating the matrix')
        with (root/'matrix-started').open('x') as marker:
            marker.write('One-shot real GUI refusal matrix; retain failures, never overwrite evidence')
        pair_and_initialize(root)
    signing_refusal_cycle(root)
    if sys.argv[1] == 'matrix':
        decryption_refusal_cycle(root)
        audit_wire(root)
        verify(root)
    print('PASS actual GUI NIP-46 refusal and regrant; zero payment actions')
