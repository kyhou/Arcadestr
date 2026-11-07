#!/usr/bin/env python3
"""Controlled private NWC relay faults; never decrypt/log encrypted payloads.

The debug desktop's transport override points here; stored wallet credentials
and wallet/application identity stay unchanged. The shared relay is untouched.
"""
import argparse
import asyncio
import json
import os
from pathlib import Path


def save(path, data):
    temporary = path.with_suffix('.tmp')
    temporary.write_text(json.dumps(data, indent=2))
    os.chmod(temporary, 0o600)
    temporary.replace(path)


def response_event(message):
    try:
        data = json.loads(message)
        return (isinstance(data, list) and len(data) == 3 and data[0] == 'EVENT'
                and isinstance(data[2], dict) and data[2].get('kind') == 23195)
    except (ValueError, TypeError):
        return False


async def serve(root, port):
    from websockets.asyncio.server import serve as websocket_server
    from websockets.asyncio.client import connect
    audit_path = root / 'network-audit.json'
    audit = {'requests_forwarded': 0, 'responses_dropped': 0,
             'connections_refused': 0, 'request_events': [], 'phases': []}
    connections = set()

    def mode():
        data = json.loads((root / 'network-mode.json').read_text())
        if data['mode'] not in ('online', 'offline', 'drop-responses'):
            raise ValueError('Unknown transport mode')
        return data

    def record():
        save(audit_path, audit)

    async def handshake(connection, request):
        if mode()['mode'] == 'offline':
            audit['connections_refused'] += 1
            record()
            return connection.respond(503, 'Controlled private NWC outage\n')
        return None

    async def websocket(downstream):
        connections.add(downstream)
        try:
            async with connect('ws://127.0.0.1:7778', max_size=2**22) as upstream:
                async def outbound():
                    async for message in downstream:
                        data = json.loads(message)
                        if (isinstance(data, list) and len(data) == 2 and data[0] == 'EVENT'
                                and isinstance(data[1], dict) and data[1].get('kind') == 23194):
                            audit['requests_forwarded'] += 1
                            audit['request_events'].append({'id': data[1]['id'], 'phase': mode()['phase'],
                                                            'application_pubkey': data[1]['pubkey']})
                            record()
                        await upstream.send(message)

                async def inbound():
                    async for message in upstream:
                        if mode()['mode'] == 'drop-responses' and response_event(message):
                            audit['responses_dropped'] += 1
                            record()
                            continue
                        await downstream.send(message)

                tasks = [asyncio.create_task(outbound()), asyncio.create_task(inbound())]
                try:
                    await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
                finally:
                    for task in tasks:
                        task.cancel()
                    await asyncio.gather(*tasks, return_exceptions=True)
        except Exception as error:
            audit['last_transport_error_type'] = type(error).__name__
            record()
        finally:
            connections.discard(downstream)

    async def control():
        previous = None
        while True:
            current = mode()
            if current != previous:
                audit['phases'].append(current)
                record()
                if current['mode'] == 'offline':
                    await asyncio.gather(*(c.close(code=1012, reason='Controlled outage')
                                           for c in tuple(connections)), return_exceptions=True)
                previous = current
            await asyncio.sleep(.1)

    record()
    async with websocket_server(websocket, '127.0.0.1', port,
                                process_request=handshake, max_size=2**22):
        await control()


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    parser.add_argument('--port', required=True, type=int)
    args = parser.parse_args()
    if not 1024 <= args.port <= 65535 or args.port in (4444, 5173, 7777, 7778, 9099, 19100):
        parser.error('Explicit independent unprivileged proxy port required')
    if args.root.stat().st_uid != os.getuid() or args.root.stat().st_mode & 0o077:
        parser.error('Evidence directory must be private and owned')
    asyncio.run(serve(args.root, args.port))


if __name__ == '__main__':
    main()
