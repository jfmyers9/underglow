// Run directly with Node 22.19+ type stripping. No npm packages or build step.
import { randomUUID } from 'node:crypto';
import { createConnection } from 'node:net';
import { setTimeout as sleep } from 'node:timers/promises';
import { parseArgs } from 'node:util';

const SOURCE = '__SOURCE__';
type Payload = { kind: 'state'; state: string; ttl_ms: number }
  | { kind: 'event'; event: string; ttl_ms: number } | { kind: 'remove' };
type Message = { schema_version: 1; source: string; instance: string;
  sequence: number; payload: Payload };

function deliver(message: Message, socketPath?: string): Promise<void> {
  const data = JSON.stringify(message) + '\n';
  if (Buffer.byteLength(data) > 4097) return Promise.reject(new Error('message too large'));
  return new Promise((resolve, reject) => {
    const client = socketPath === undefined ? undefined : createConnection(socketPath);
    let reply = Buffer.alloc(0);
    let settled = false;
    const timer = setTimeout(() => finish(new Error('500ms delivery deadline exceeded')), 500);
    function finish(error?: Error) {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      client?.destroy();
      if (error) reject(error); else resolve();
    }
    if (!client) {
      process.stdout.write(data, (error) => finish(error ?? undefined));
      return;
    }
    client.on('error', finish);
    client.on('connect', () => client.write(data));
    client.on('end', () => finish(new Error('engine closed before ACK')));
    client.on('data', (chunk: Buffer) => {
      if (reply.length + chunk.length > 4096) {
        finish(new Error('engine reply exceeds 4096 bytes'));
        return;
      }
      reply = Buffer.concat([reply, chunk]);
      if (!reply.includes(10)) return;
      try {
        const ack = JSON.parse(reply.toString('utf8'));
        if (ack.schema_version !== 1 || ack.ok !== true || ack.outcome !== 'applied') {
          throw new Error('engine did not apply message');
        }
        finish();
      } catch (error) { finish(error as Error); }
    });
  });
}

async function main() {
  const { values } = parseArgs({ options: {
    stdout: { type: 'boolean', default: false }, socket: { type: 'string' },
    'duration-seconds': { type: 'string', default: '60' },
  } });
  if (values.stdout === (values.socket !== undefined)) {
    throw new Error('select exactly one: --stdout or --socket PATH');
  }
  const duration = Number(values['duration-seconds']);
  if (!Number.isFinite(duration) || duration < 0 || duration > 86400) {
    throw new Error('duration must be finite and between 0 and 86400 seconds');
  }
  const stop = new AbortController();
  process.on('SIGINT', () => stop.abort());
  process.on('SIGTERM', () => stop.abort());
  const instance = randomUUID();
  let sequence = 0;
  async function publish(payload: Payload) {
    if (sequence >= Number.MAX_SAFE_INTEGER) throw new Error('sequence exhausted');
    await deliver({ schema_version: 1, source: SOURCE, instance,
      sequence: ++sequence, payload }, values.socket);
  }
  try {
    const end = performance.now() + duration * 1000;
    await publish({ kind: 'state', state: 'focusing', ttl_ms: 15000 });
    while (!stop.signal.aborted) {
      const remaining = end - performance.now();
      if (remaining <= 0) {
        await publish({ kind: 'event', event: 'done', ttl_ms: 2000 });
        break;
      }
      await sleep(Math.min(5000, remaining), undefined, { signal: stop.signal })
        .catch((error: Error) => { if (!stop.signal.aborted) throw error; });
      if (!stop.signal.aborted && performance.now() < end) {
        await publish({ kind: 'state', state: 'focusing', ttl_ms: 15000 });
      }
    }
  } finally {
    // One bounded attempt; crashes and failed removal fall back to lease expiry.
    await publish({ kind: 'remove' });
  }
}

main().catch((error: Error) => {
  console.error(`connector stopped: ${error.message}`);
  // Also terminate a stalled stdout write after its deadline.
  process.exit(1);
});
