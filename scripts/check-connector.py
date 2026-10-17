#!/usr/bin/env python3
"""Offline v1 NDJSON shape/sequence lint; not a replacement for the Rust registry."""
import argparse
import json
from pathlib import Path
import re
import sys

MAX_MESSAGE = 4096
MAX_MESSAGES = 10000
MAX_IDENTITIES = 256
TOKEN = re.compile(r'[A-Za-z0-9_.:/-]{1,128}\Z')


def integer(value, maximum):
    return type(value) is int and 0 <= value <= maximum


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f'duplicate field: {key}')
        result[key] = value
    return result


def validate_message(raw, sequences):
    """Validate one JSON byte string, then update per-identity high-water marks."""
    if len(raw) > MAX_MESSAGE:
        raise ValueError('message exceeds 4096 bytes')
    try:
        message = json.loads(raw.decode('utf-8'), object_pairs_hook=unique_object)
    except (UnicodeDecodeError, json.JSONDecodeError, RecursionError) as error:
        raise ValueError(f'invalid JSON: {error}') from error
    if type(message) is not dict or set(message) != {
        'schema_version', 'source', 'instance', 'sequence', 'payload'
    }:
        raise ValueError('envelope fields must be schema_version/source/instance/sequence/payload')
    if type(message['schema_version']) is not int or message['schema_version'] != 1:
        raise ValueError('schema_version must be integer 1')
    for field in ('source', 'instance'):
        if type(message[field]) is not str or not TOKEN.fullmatch(message[field]):
            raise ValueError(f'invalid {field} token')
    if not integer(message['sequence'], 2**64 - 1):
        raise ValueError('sequence must be an unsigned 64-bit integer')
    payload = message['payload']
    if type(payload) is not dict:
        raise ValueError('payload must be an object')
    kind = payload.get('kind')
    if kind == 'remove':
        if set(payload) != {'kind'}:
            raise ValueError('remove only accepts kind')
    elif kind in ('state', 'event'):
        if set(payload) != {'kind', kind, 'ttl_ms'}:
            raise ValueError(f'{kind} fields must be kind/{kind}/ttl_ms')
        if type(payload[kind]) is not str or not TOKEN.fullmatch(payload[kind]):
            raise ValueError(f'invalid {kind} label')
        if not integer(payload['ttl_ms'], 300000) or payload['ttl_ms'] == 0:
            raise ValueError('ttl_ms must be an integer from 1 to 300000')
    else:
        raise ValueError('payload.kind must be state, event, or remove')
    identity = (message['source'], message['instance'])
    if identity in sequences and message['sequence'] <= sequences[identity]:
        raise ValueError('sequence must strictly increase across every message kind')
    if identity not in sequences and len(sequences) >= MAX_IDENTITIES:
        raise ValueError('checker fixture identity limit reached (256)')
    sequences[identity] = message['sequence']
    return message


def check_stream(stream):
    sequences = {}
    count = 0
    while True:
        raw = stream.readline(MAX_MESSAGE + 2)
        if not raw:
            return count
        count += 1
        if count > MAX_MESSAGES:
            raise ValueError('checker fixture message limit reached (10000)')
        if not raw.endswith(b'\n'):
            raise ValueError(f'line {count}: newline required (or line exceeds byte limit)')
        try:
            validate_message(raw[:-1], sequences)
        except ValueError as error:
            raise ValueError(f'line {count}: {error}') from error


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('fixture', nargs='?', default='-', help='NDJSON fixture, or - for stdin')
    args = parser.parse_args()
    try:
        if args.fixture == '-':
            count = check_stream(sys.stdin.buffer)
        else:
            with Path(args.fixture).open('rb') as stream:
                count = check_stream(stream)
        if not count:
            raise ValueError('fixture is empty')
    except (OSError, ValueError) as error:
        parser.exit(1, f'connector check failed: {error}\n')
    print(f'OK: {count} messages (offline v1 shape/sequence check)')


if __name__ == '__main__':
    main()
