#!/usr/bin/env python3
"""Outstanding Gate 3 tools. Launch existing fixtures; arm/verify a manual reboot.

No invoice creation, payment dispatch, credential replacement or reboot command
exists here. The separate principal fixture requires its own explicit opt-in.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import runpy
import sqlite3
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

REPO = Path(__file__).resolve().parent.parent
STATE = Path.home() / '.local/state/arcadestr-regtest'
QUOTE = 'regtest-gate2-f4b3a25e-a984-4b09-8442-e66e06441bff'


def save(path, data):
    temporary = path.with_suffix('.tmp')
    temporary.write_text(json.dumps(data, indent=2))
    os.chmod(temporary, 0o600)
    temporary.replace(path)


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1048576), b''):
            h.update(block)
    return h.hexdigest()


def encrypted_blob_digest(value):
    return hashlib.sha256(value if isinstance(value, bytes) else (value or '').encode()).hexdigest()


def boot_id():
    return Path('/proc/sys/kernel/random/boot_id').read_text().strip()


def same_boot(checkpoint, current):
    return checkpoint['boot_id'] == current


def private_root(path):
    path = path.resolve(strict=True)
    if not path.is_relative_to(STATE / 'gate3-runs') or path.stat().st_uid != os.getuid() or path.stat().st_mode & 0o077:
        raise RuntimeError('Expected an existing owner-only Gate 3 run directory')
    return path


def database(root, readonly=True):
    path = (root / 'desktop-data/arcadestr/arcadestr.db').resolve()
    if not path.is_file() or not path.is_relative_to(root):
        raise RuntimeError('Original private desktop database required')
    return sqlite3.connect(f'file:{path}?mode={"ro" if readonly else "rw"}', uri=True)


def request(path, data=None, method=None, timeout=90):
    req = urllib.request.Request('http://127.0.0.1:4444' + path,
                                 data=None if data is None else json.dumps(data).encode(),
                                 headers={'Content-Type': 'application/json'}, method=method)
    with urllib.request.urlopen(req, timeout=timeout) as response:
        value = json.load(response)
    if isinstance(value.get('value'), dict) and value['value'].get('error') and 'message' in value['value']:
        raise RuntimeError('WebDriver operation failed; inspect private diagnostics')
    return value['value']


def gui(root, script, args=None, timeout=90):
    sid = json.loads((root / 'webdriver-session.json').read_text())['session_id']
    return request(f'/session/{sid}/execute/async', {'script': script, 'args': args or []}, timeout=timeout)


def snapshot(root, name):
    sid = json.loads((root / 'webdriver-session.json').read_text())['session_id']
    data = request(f'/session/{sid}/execute/sync', {'script': 'return {text:document.body.innerText,payButtons:document.querySelectorAll("[data-testid=pay-refund]").length}', 'args': []})
    save(root / (name + '.json'), data)
    try:
        image = request(f'/session/{sid}/screenshot', timeout=5)
        (root / (name + '.png')).write_bytes(base64.b64decode(image))
    except OSError:
        # Some Wayland compositors suspend snapshots of an inactive window.
        # Keep actual GUI DOM evidence; do not misreport a missing screenshot.
        save(root / (name + '-screenshot-unavailable.json'), {'screenshot_available': False, 'dom_evidence_retained': True})
    return data


def load_refund_gui(root, fixture, prepare=False):
    script = """const done=arguments[arguments.length-1],f=arguments[0],prepare=arguments[1];
function wait(test){return new Promise((resolve,reject)=>{const o=new MutationObserver(check);const t=setTimeout(()=>{o.disconnect();reject(Error('UI timeout'));},25000);function check(){if(test()){clearTimeout(t);o.disconnect();resolve();}}o.observe(document.body,{childList:true,subtree:true,characterData:true,attributes:true});check();});}
(async()=>{if(!document.querySelector('[data-testid=refund-processor]')){await wait(()=>[...document.querySelectorAll('button')].some(b=>b.innerText.includes('npub')&&b.innerText.includes('expand_more')));[...document.querySelectorAll('button')].find(b=>b.innerText.includes('npub')&&b.innerText.includes('expand_more')).click();await wait(()=>[...document.querySelectorAll('button')].some(b=>b.innerText.trim()==='Purchases'));[...document.querySelectorAll('button')].find(b=>b.innerText.trim()==='Purchases').click();await wait(()=>document.querySelector('[data-testid=refund-processor]'));}const input=document.querySelector('[data-testid=refund-processor]');input.value=f.server_url;input.dispatchEvent(new Event('input',{bubbles:true}));await wait(()=>!document.querySelector('[data-testid=load-refunds]').disabled);document.querySelector('[data-testid=load-refunds]').click();await new Promise(resolve=>setTimeout(resolve,0));await wait(()=>!document.querySelector('[data-testid=load-refunds]').disabled&&document.querySelector('[data-testid=recipient-refund-card]')?.innerText.includes(f.quote_id));if(prepare){document.querySelector('[data-testid=prepare-refund]').click();await wait(()=>document.querySelector('[data-testid=pay-refund]'));}return {text:document.querySelector('[data-testid=recipient-refund-card]').innerText,payButtons:document.querySelectorAll('[data-testid=pay-refund]').length};})().then(done).catch(e=>done({error:String(e),text:document.body.innerText}));"""
    result = gui(root, script, [fixture, prepare], timeout=110)
    if 'error' in result:
        raise RuntimeError('Refund GUI did not reach expected original operation')
    return result


def alive(pid, marker):
    try:
        return marker.encode() in Path(f'/proc/{pid}/cmdline').read_bytes()
    except OSError:
        return False


def spawn(root, name, command, env=None):
    with (root / (name + '.log')).open('ab') as output:
        process = subprocess.Popen(command, cwd=REPO, env=env, stdin=subprocess.DEVNULL,
                                   stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
    return {'pid': process.pid, 'command': command}


def reachable(port):
    import socket
    try:
        with socket.create_connection(('127.0.0.1', port), timeout=1):
            return True
    except OSError:
        return False


def start_desktop(root):
    response = request('/session', {'capabilities': {'alwaysMatch': {'tauri:options': {
        'application': str(REPO / 'target/debug/arcadestr-desktop')}}}})
    save(root / 'webdriver-session.json', {'session_id': response['sessionId']})
    request(f"/session/{response['sessionId']}/timeouts", {'script': 120000, 'pageLoad': 120000})


def original_env():
    m = runpy.run_path(str(REPO / 'scripts/regtest'))
    adp = Path(m['settings']()['adp_repo'])
    env = m['fixture_env'](m['verify_nodes']())
    env.update(m['adp_env'](adp))
    env.update({'ARCADESTR_REGTEST_DATA_DIR': str(Path.home() / '.local/share/arcadestr'),
                'NO_PROXY': '127.0.0.1,localhost,::1', 'no_proxy': '127.0.0.1,localhost,::1'})
    env.pop('ARCADESTR_REGTEST_GATE3_PAYMENTS', None)
    return env


def launch(root):
    fixtures = root / 'refund-outages'
    manifest = json.loads((fixtures / 'gui-fixtures.json').read_text())
    services = {}
    fixture_ports = []
    for row in manifest['fixtures']:
        url = urllib.parse.urlparse(row['server_url'])
        if url.hostname != '127.0.0.1' or not url.port:
            raise RuntimeError('Nonlocal fixture binding refused')
        fixture_ports.append(url.port)
    if not all(reachable(p) for p in fixture_ports):
        if any(reachable(p) for p in fixture_ports):
            raise RuntimeError('Partially occupied fixture bindings; inspect ownership manually')
        services['fixtures'] = spawn(root, 'restored-fixtures',
            [str(REPO / 'target/debug/examples/regtest_gate3_restore'), '--existing-fixtures', str(fixtures)], original_env())
        deadline = time.monotonic() + 30
        while not all(reachable(p) for p in fixture_ports):
            if time.monotonic() > deadline:
                raise RuntimeError('Existing-only fixture restoration not ready')
            time.sleep(.2)
    if not reachable(5173):
        if not (root / 'frontend/index.html').is_file():
            raise RuntimeError('Previously built frontend required; no implicit build')
        services['frontend'] = spawn(root, 'static-frontend', [sys.executable, '-m', 'http.server',
            '5173', '--bind', '127.0.0.1', '--directory', str(root / 'frontend')])
    if reachable(4444):
        metadata = json.loads((root / 'runtime-processes.json').read_text())
        service = metadata.get('services', {}).get('driver', {})
        if metadata.get('boot_id') != boot_id() or not alive(service.get('pid', -1), 'tauri-driver'):
            raise RuntimeError('Automation port occupied; do not replace an existing session')
        environment = Path(f"/proc/{service['pid']}/environ").read_bytes().split(b'\0')
        if ('XDG_DATA_HOME=' + str(root / 'desktop-data')).encode() not in environment:
            raise RuntimeError('Automation account scope mismatch')
        if services:
            save(root / ('runtime-processes-retained-' + str(time.time_ns()) + '.json'), metadata)
            metadata['services'].update(services)
            save(root / 'runtime-processes.json', metadata)
        session_path = root / 'webdriver-session.json'
        if session_path.exists():
            sid = json.loads(session_path.read_text())['session_id']
            try:
                request(f'/session/{sid}/url', timeout=5)
                return
            except (OSError, RuntimeError):
                pass
        start_desktop(root)
        return
    proxy_port = 43178
    if reachable(proxy_port):
        raise RuntimeError('Private proxy port occupied; inspect ownership manually')
    network = str(REPO / 'scripts/regtest-gate3-network.py')
    for filename in ('network-audit.json', 'runtime-processes.json'):
        source = root / filename
        if source.exists():
            save(root / (source.stem + '-retained-' + str(time.time_ns()) + '.json'), json.loads(source.read_text()))
    services['proxy'] = spawn(root, 'network-proxy', [sys.executable, network, str(root), '--port', str(proxy_port)])
    env = os.environ.copy()
    env.update({'XDG_DATA_HOME': str(root / 'desktop-data'), 'XDG_CONFIG_HOME': str(root / 'desktop-config'),
                'XDG_CACHE_HOME': str(root / 'desktop-cache'), 'ARCADESTR_REGTEST_ALLOW_LOOPBACK_HTTP': 'true',
                'ARCADESTR_RELAYS': 'ws://127.0.0.1:7777,ws://127.0.0.1:7778', 'ARCADESTR_BLOCK_DISCOVERY': 'true',
                'ARCADESTR_REGTEST_NWC_RELAY_PROXY': f'ws://127.0.0.1:{proxy_port}'})
    services['driver'] = spawn(root, 'webdriver', ['tauri-driver', '--port', '4444', '--native-port', '4445', '--native-driver', '/usr/bin/WebKitWebDriver'], env)
    save(root / 'runtime-processes.json', {'boot_id': boot_id(), 'proxy_port': proxy_port, 'services': services})
    deadline = time.monotonic() + 30
    while True:
        try:
            if request('/status', timeout=2).get('ready'):
                break
        except (OSError, ValueError):
            pass
        if time.monotonic() > deadline:
            raise RuntimeError('Private automation not ready; inspect private logs')
        time.sleep(.2)
    start_desktop(root)
    print('Actual private desktop launched; original wallet URI/app binding unchanged')


def protected_fingerprints():
    config = Path.home() / '.config/arcadestr-regtest'
    paths = [config / name for name in ('carol-adp.env', 'dave-adp.env', 'dave-operator.key',
                                      'signer-locations.json', 'settings.json')]
    paths.append(Path.home() / '.local/share/arcadestr/.master_key')
    result = {}
    for path in paths:
        if not path.is_file() or path.stat().st_uid != os.getuid() or path.stat().st_mode & 0o077:
            raise RuntimeError('Protected configuration absent or permissions unsafe')
        result[str(path)] = digest(path)
    c = sqlite3.connect(f'file:{Path.home()}/.local/share/arcadestr/accounts.db?mode=ro', uri=True)
    result['account_bindings'] = [(p, mode, encrypted_blob_digest(cipher))
                                 for p, mode, cipher in c.execute('SELECT pubkey,signing_mode,encrypted_nsec FROM accounts ORDER BY pubkey')]
    c.close()
    # JSON normalizes tuples across the persisted checkpoint.
    return json.loads(json.dumps(result))


def private_credentials(root):
    directory = root / 'desktop-data/arcadestr'
    for name in ('.master_key', 'accounts.db'):
        path = directory / name
        if not path.is_file() or not path.resolve().is_relative_to(root) or path.stat().st_uid != os.getuid() or path.stat().st_mode & 0o077:
            raise RuntimeError('Existing private signer storage required; refusing regeneration')
    c = sqlite3.connect(f'file:{directory}/accounts.db?mode=ro', uri=True)
    bindings = [(p, mode, encrypted_blob_digest(cipher)) for p, mode, cipher in
                c.execute('SELECT pubkey,signing_mode,encrypted_nsec FROM accounts ORDER BY pubkey')]
    c.close()
    return {'master_key_sha256': digest(directory / '.master_key'), 'account_bindings': json.loads(json.dumps(bindings))}


def seal_private_credentials(root):
    path = root / 'reboot-private-credentials.json'
    if path.exists():
        raise RuntimeError('Private signer checkpoint already exists; never overwrite it')
    private = private_credentials(root)
    original = protected_fingerprints()
    if private['account_bindings'] != original['account_bindings'] or private['master_key_sha256'] != original[str(Path.home() / '.local/share/arcadestr/.master_key')]:
        raise RuntimeError('Private signer copies differ from original identities')
    save(path, private)


def infrastructure():
    m = runpy.run_path(str(REPO / 'scripts/regtest'))
    result = {'containers': {}, 'wallets': {}, 'balances_msat': {}}
    for node in ('backend1', *m['NODES']):
        p = subprocess.run(['docker', 'inspect', '--format', '{{.Id}}', 'polar-n1-' + node], capture_output=True, text=True, check=True)
        result['containers'][node] = p.stdout.strip()
    for node in m['NODES']:
        info = m['lncli'](node, 'getinfo')
        if info['chains'] != [{'chain': 'bitcoin', 'network': 'regtest'}]:
            raise RuntimeError('Not exclusively regtest')
        channels = m['lncli'](node, 'listchannels')['channels']
        if any(c.get('pending_htlcs') for c in channels):
            raise RuntimeError('Pending HTLCs require separate reboot coordination')
        result['wallets'][node] = info['identity_pubkey']
        result['balances_msat'][node] = sum(int(c['local_balance']) * 1000 for c in channels)
    return result


def dispatch_audit(root):
    with database(root) as c:
        return [dict(zip(('account', 'hash', 'dispatch_count', 'lookup_count'), row))
                for row in c.execute('SELECT account_pubkey,payment_hash,dispatch_count,lookup_count FROM nwc_payment_journal ORDER BY account_pubkey,payment_hash')]


def arm_reboot(root):
    checkpoint = root / 'reboot-checkpoint.json'
    if checkpoint.exists():
        raise RuntimeError('Checkpoint already exists; do not overwrite an armed run')
    sid = json.loads((root / 'webdriver-session.json').read_text())['session_id']
    request(f'/session/{sid}', method='DELETE')
    original = infrastructure()
    protected = protected_fingerprints()
    with database(root, readonly=False) as c:
        backup = sqlite3.connect(root / 'before-reboot-injection.db')
        c.backup(backup)
        backup.close()
        account, receipt = c.execute('SELECT account_pubkey,receipt_event_json FROM settlement_quotes WHERE quote_id=?', (QUOTE,)).fetchone()
        if not receipt:
            raise RuntimeError('Expected already-verified paid receipt in private copy')
        bindings = [row[0] for row in c.execute('SELECT DISTINCT wallet_identity FROM nwc_payment_journal WHERE account_pubkey=?', (account,))]
        if len(bindings) != 1:
            raise RuntimeError('Original wallet/application binding is not unique')
        coordinate = c.execute('SELECT listing_coordinate FROM purchases WHERE buyer_pubkey=? AND order_id=?', (account, QUOTE)).fetchone()[0]
        attempt, leg = c.execute('SELECT a.attempt_id,a.leg_id FROM settlement_invoice_attempts a JOIN settlement_legs l ON a.account_pubkey=l.account_pubkey AND a.quote_id=l.quote_id AND a.leg_id=l.leg_id WHERE a.account_pubkey=? AND a.quote_id=? AND a.encrypted_proof IS NOT NULL ORDER BY l.leg_order DESC LIMIT 1', (account, QUOTE)).fetchone()
        # Only this tool's private copy changes. Signed originals and backups remain.
        c.execute("UPDATE settlement_quotes SET state='active',receipt_event_json=NULL WHERE account_pubkey=? AND quote_id=?", (account, QUOTE))
        c.execute("UPDATE settlement_invoice_attempts SET state='disclosed' WHERE account_pubkey=? AND quote_id=? AND attempt_id=?", (account, QUOTE, attempt))
        c.execute("UPDATE settlement_legs SET state='pending' WHERE account_pubkey=? AND quote_id=? AND leg_id=?", (account, QUOTE, leg))
        c.execute('DELETE FROM purchases WHERE buyer_pubkey=? AND order_id=?', (account, QUOTE))
        c.execute('DELETE FROM download_tokens WHERE buyer_pubkey=? AND game_coordinate=?', (account, coordinate))
    c.execute('PRAGMA wal_checkpoint(TRUNCATE)')
    c.close()
    reservations = {str(p): digest(p) for p in STATE.glob('*reservation.json')}
    if str(STATE / 'gate3-outages-reservation.json') not in reservations:
        raise RuntimeError('Outstanding operation reservation missing')
    data = {'boot_id': boot_id(), 'machine_reboot_verified': False, 'state': 'armed-awaiting-manual-reboot',
            'quote_id': QUOTE, 'buyer': account, 'coordinate': coordinate,
            'wallet_application_identity': bindings[0],
            'receipt_sha256': hashlib.sha256(json.dumps(json.loads(receipt), sort_keys=True).encode()).hexdigest(),
            'protected': protected, 'infrastructure': original, 'reservations': reservations,
            'dispatch_audit': dispatch_audit(root),
            'database_sha256': digest(root / 'desktop-data/arcadestr/arcadestr.db'),
            'binary_sha256': digest(REPO / 'target/debug/arcadestr-desktop'),
            'fault_classification': 'private copy: original paid proof retained, receipt/ownership/token persistence interrupted',
            'created_at': int(time.time())}
    save(checkpoint, data)
    seal_private_credentials(root)
    print('ARMED: durable private checkpoint recorded; no reboot was requested')


def after_reboot(root):
    checkpoint = json.loads((root / 'reboot-checkpoint.json').read_text())
    if same_boot(checkpoint, boot_id()):
        save(root / 'reboot-verification.json', {'machine_reboot_verified': False, 'state': 'blocked-same-boot', 'payment_dispatches': 0})
        raise RuntimeError('Same boot: application restart is not a machine reboot')
    if protected_fingerprints() != checkpoint['protected']:
        raise RuntimeError('Protected identity/configuration changed')
    if private_credentials(root) != json.loads((root / 'reboot-private-credentials.json').read_text()):
        raise RuntimeError('Private signer copies changed; do not regenerate credentials')
    if digest(REPO / 'target/debug/arcadestr-desktop') != checkpoint['binary_sha256']:
        raise RuntimeError('Desktop executable changed since checkpoint')
    for path, expected in checkpoint['reservations'].items():
        if digest(Path(path)) != expected:
            raise RuntimeError('A retained reservation changed')
    started_path = root / 'post-reboot-started.json'
    if started_path.exists():
        started = json.loads(started_path.read_text())
        if started['boot_id'] != boot_id() or started['checkpoint_sha256'] != digest(root / 'reboot-checkpoint.json'):
            raise RuntimeError('Interrupted verification belongs to another checkpoint/boot')
    else:
        if digest(root / 'desktop-data/arcadestr/arcadestr.db') != checkpoint['database_sha256']:
            raise RuntimeError('Pre-startup private database checkpoint differs')
        save(started_path, {'boot_id': boot_id(), 'checkpoint_sha256': digest(root / 'reboot-checkpoint.json'), 'verified_pre_startup_durability': True})
    # This starts only previously configured containers/services; never mines.
    with (root / 'post-reboot-services.log').open('w') as out:
        p = subprocess.run([str(REPO / 'scripts/regtest'), 'up', '--no-build'], cwd=REPO, stdout=out, stderr=subprocess.STDOUT)
    if p.returncode:
        raise RuntimeError('Existing infrastructure not ready; unlock existing wallet/check private service log')
    current = infrastructure()
    if current != checkpoint['infrastructure']:
        raise RuntimeError('Existing container/wallet/balance identity changed')
    save(root / 'network-mode.json', {'mode': 'online', 'phase': 'post-machine-reboot'})
    launch(root)
    deadline = time.monotonic() + 120
    while True:
        with database(root) as c:
            row = c.execute('SELECT state,receipt_event_json FROM settlement_quotes WHERE account_pubkey=? AND quote_id=?', (checkpoint['buyer'], QUOTE)).fetchone()
            owned = c.execute('SELECT COUNT(*) FROM purchases WHERE buyer_pubkey=? AND order_id=?', (checkpoint['buyer'], QUOTE)).fetchone()[0]
            token = c.execute('SELECT server_url,token FROM download_tokens WHERE buyer_pubkey=? AND game_coordinate=? AND expires_at>?', (checkpoint['buyer'], checkpoint['coordinate'], int(time.time()))).fetchone()
        if row[0] == 'settled' and row[1] and owned == 1 and token:
            break
        if time.monotonic() > deadline:
            raise RuntimeError('Automatic startup reconciliation did not complete; do not pay')
        time.sleep(1)
    receipt_digest = hashlib.sha256(json.dumps(json.loads(row[1]), sort_keys=True).encode()).hexdigest()
    if receipt_digest != checkpoint['receipt_sha256']:
        raise RuntimeError('Recovered receipt differs from verified baseline')
    audit = dispatch_audit(root)
    expected = {(r['account'], r['hash']): r['dispatch_count'] for r in checkpoint['dispatch_audit']}
    if {(r['account'], r['hash']): r['dispatch_count'] for r in audit} != expected:
        raise RuntimeError('Unexpected new payment admission')
    verify = gui(root, "const done=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke('verify_nwc_wallet',{expectedAccountPubkey:arguments[0]}).then(v=>done({status:v})).catch(()=>done({status:'unavailable'}));", [checkpoint['buyer']])
    if verify.get('status') != 'verified':
        raise RuntimeError('Existing NWC wallet did not reconnect')
    metadata = gui(root, "const done=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke('get_nwc_wallet',{expectedAccountPubkey:arguments[0]}).then(done).catch(()=>done(null));", [checkpoint['buyer']])
    wallet, application = checkpoint['wallet_application_identity'].split(':')
    traffic = json.loads((root / 'network-audit.json').read_text())['request_events']
    if not metadata or metadata['wallet_pubkey'] != wallet or not traffic or any(row.get('application_pubkey') != application for row in traffic):
        raise RuntimeError('Wallet/application identity changed; credentials were not replaced by this tool')
    # Authenticated operational reads only. No prepare/pay command is invoked.
    fixtures = json.loads((root / 'refund-outages/gui-fixtures.json').read_text())['fixtures']
    supervisor = runpy.run_path(str(REPO / 'scripts/regtest'))
    wallet_history = supervisor['lncli']('alice', 'listpayments', '--include_incomplete')['payments']
    wallet_counts = []
    for index, fixture in enumerate(fixtures):
        with database(root) as c:
            payment_hash = c.execute('SELECT payment_hash FROM client_refund_attempts WHERE quote_id=?', (fixture['quote_id'],)).fetchone()[0]
        matching = [p for p in wallet_history if p['payment_hash'] == payment_hash]
        invoice = supervisor['lncli']('bob', 'lookupinvoice', payment_hash)
        if index == 0 and (matching or invoice['settled']):
            raise RuntimeError('Unresolved original refund unexpectedly paid')
        if index == 1 and (len(matching) != 1 or matching[0]['status'] != 'SUCCEEDED' or not invoice['settled']):
            raise RuntimeError('Previously verified wallet payment history differs')
        wallet_counts.append(len(matching))
        view = load_refund_gui(root, fixture)
        if view['payButtons']:
            raise RuntimeError('Unexpected repay control after machine reboot')
        if index == 0 and 'UNCERTAIN' not in view['text']:
            raise RuntimeError('Unpaid original intent must remain unresolved')
        if index == 1 and 'VERIFIED' not in view['text']:
            raise RuntimeError('Previously paid refund state is inconsistent')
        snapshot(root, 'post-reboot-refund-' + str(index))
    server, secret_token = token
    parsed = urllib.parse.urlparse(server)
    if parsed.hostname not in ('localhost', '127.0.0.1'):
        raise RuntimeError('Nonlocal download refused')
    url = server.rstrip('/') + '/game/' + urllib.parse.quote(checkpoint['coordinate'], safe='') + '?token=' + urllib.parse.quote(secret_token, safe='')
    with urllib.request.urlopen(url, timeout=20) as response:
        payload = response.read()
    m = runpy.run_path(str(REPO / 'scripts/regtest'))
    reference = Path(m['settings']()['adp_repo']) / 'testgame.bin'
    if hashlib.sha256(payload).hexdigest() != digest(reference):
        raise RuntimeError('Recovered download hash mismatch')
    save(root / 'reboot-verification.json', {'machine_reboot_verified': True, 'boot_changed': True,
        'protected_configuration_preserved': True, 'existing_infrastructure_preserved': True,
        'startup_recovered_original_receipt_ownership_token': True, 'new_payment_admissions': 0,
        'download_bytes': len(payload), 'download_sha256': hashlib.sha256(payload).hexdigest(),
        'nwc_reconnected': True, 'dispatch_audit': audit,
        'refund_wallet_payment_counts': wallet_counts, 'unresolved_refund_principal_msat': 100000,
        'reservations_retained': True,
        'gate3_closed': False, 'remaining_blocker': 'real locked remote signer unavailable'})
    print('PASS actual changed-boot startup recovery; original identities/reservations retained; no new dispatch admissions')


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['launch', 'arm-reboot', 'after-reboot'])
    parser.add_argument('--root', required=True, type=Path)
    args = parser.parse_args()
    root = private_root(args.root)
    if args.action == 'launch':
        launch(root)
    elif args.action == 'arm-reboot':
        arm_reboot(root)
    elif args.action == 'after-reboot':
        after_reboot(root)
    else:
        raise RuntimeError('Unknown action')


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        # Error messages from HTTP/credential providers may contain secrets.
        print('BLOCKED:', type(error).__name__, '— inspect private evidence; no reboot/payment requested', file=sys.stderr)
        sys.exit(1)
