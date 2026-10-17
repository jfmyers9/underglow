#!/usr/bin/env python3
"""A focus-session producer using only Python's standard library (macOS/Linux)."""
import argparse
import json
import math
import os
import select
import signal
import socket
import sys
import threading
import time
import uuid

SOURCE = '__SOURCE__'
TTL_MS = 15000
DEADLINE_SECONDS = 0.5


def deliver(message, socket_path):
    data = json.dumps(message, separators=(',', ':')).encode() + b'\n'
    if len(data) > 4097:
        raise ValueError('message exceeds 4096 JSON bytes')
    deadline = time.monotonic() + DEADLINE_SECONDS
    if socket_path is None:
        # Nonblocking stdout plus an absolute deadline also bounds a stalled pipe.
        fd = sys.stdout.fileno()
        was_blocking = os.get_blocking(fd)
        try:
            os.set_blocking(fd, False)
            while data:
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not select.select([], [fd], [], remaining)[1]:
                    raise TimeoutError('stdout deadline exceeded')
                try:
                    data = data[os.write(fd, data):]
                except BlockingIOError:
                    continue
        finally:
            os.set_blocking(fd, was_blocking)
        return
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
        client.settimeout(max(0.001, deadline - time.monotonic()))
        client.connect(socket_path)
        client.settimeout(max(0.001, deadline - time.monotonic()))
        client.sendall(data)
        reply = bytearray()
        while b'\n' not in reply:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError('socket deadline exceeded')
            client.settimeout(remaining)
            chunk = client.recv(4097 - len(reply))
            if not chunk:
                raise ValueError('engine closed before ACK')
            reply.extend(chunk)
            if len(reply) > 4096:
                raise ValueError('engine reply exceeds 4096 bytes')
        ack = json.loads(reply)
        if (type(ack) is not dict or type(ack.get('schema_version')) is not int
                or ack.get('schema_version') != 1 or ack.get('ok') is not True
                or ack.get('outcome') != 'applied'):
            raise ValueError(f'engine did not apply message: {ack}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--stdout', action='store_true', help='emit NDJSON without socket ACKs')
    mode.add_argument('--socket', help='explicit private events.sock path')
    parser.add_argument('--duration-seconds', type=float, default=60)
    args = parser.parse_args()
    if not math.isfinite(args.duration_seconds) or not 0 <= args.duration_seconds <= 86400:
        parser.error('duration must be finite and between 0 and 86400 seconds')
    stop = threading.Event()
    for sig in (signal.SIGINT, signal.SIGTERM):
        signal.signal(sig, lambda *_: stop.set())
    instance = str(uuid.uuid4())
    sequence = 0

    def publish(payload):
        nonlocal sequence
        sequence += 1  # Shared across state, event, remove; never reuse after failure.
        deliver({'schema_version': 1, 'source': SOURCE, 'instance': instance,
                 'sequence': sequence, 'payload': payload}, args.socket)

    try:
        end = time.monotonic() + args.duration_seconds
        publish({'kind': 'state', 'state': 'focusing', 'ttl_ms': TTL_MS})
        while not stop.is_set():
            remaining = end - time.monotonic()
            if remaining <= 0:
                publish({'kind': 'event', 'event': 'done', 'ttl_ms': 2000})
                break
            if stop.wait(min(5, remaining)):
                break
            if time.monotonic() < end:
                publish({'kind': 'state', 'state': 'focusing', 'ttl_ms': TTL_MS})
    finally:
        # One bounded attempt; failed cleanup falls back to lease expiry.
        publish({'kind': 'remove'})


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError) as error:
        print(f'connector stopped: {error}', file=sys.stderr)
        sys.exit(1)
