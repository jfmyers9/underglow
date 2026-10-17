import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { createServer } from "node:net";
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { performance } from "node:perf_hooks";
import test from "node:test";
import extension from "./index.ts";
import { PiConnector, registerPiConnector, socketPath } from "./connector.mjs";

class FakePi extends EventEmitter {
  async dispatch(name, event = {}) {
    for (const handler of this.listeners(name)) await handler(event, { secret: "private-context" });
  }
}

async function until(predicate, timeout = 2000) {
  const deadline = performance.now() + timeout;
  while (!predicate()) {
    assert.ok(performance.now() < deadline, "condition timed out");
    await delay(5);
  }
}

const ack = (socket, outcome = "applied") =>
  socket.end(`${JSON.stringify({ schema_version: 1, ok: true, outcome })}\n`);

async function fixture(t, handler) {
  // Keep Unix socket names short even on macOS with its long default temp directory.
  const dir = await mkdtemp("/tmp/wooting-pi-");
  const path = join(dir, "events.sock");
  const messages = [];
  const sockets = new Set();
  const clients = [];
  let server;
  const stopServer = async () => {
    for (const socket of sockets) socket.destroy();
    if (server) await new Promise((resolve) => server.close(resolve));
    server = undefined;
  };
  const listen = async () => {
    server = createServer((socket) => {
      sockets.add(socket);
      socket.on("error", () => {});
      socket.on("close", () => sockets.delete(socket));
      let buffer = "";
      socket.on("data", (chunk) => {
        buffer += chunk;
        if (!buffer.includes("\n")) return;
        const message = JSON.parse(buffer.split("\n")[0]);
        messages.push(message);
        (handler ?? ((_message, connection) => ack(connection)))(message, socket);
      });
    });
    await new Promise((resolve, reject) => {
      server.once("error", reject);
      server.listen(path, resolve);
    });
  };
  const options = { path, heartbeatMs: 40, timeoutMs: 40, shutdownMs: 30,
    backoffMinMs: 10, backoffMaxMs: 40 };
  const client = (overrides) => {
    const value = new PiConnector({ ...options, ...overrides });
    clients.push(value);
    return value;
  };
  const pi = () => {
    const value = new FakePi();
    registerPiConnector(value, options);
    clients.push({ stop: () => value.dispatch("session_shutdown") });
    return value;
  };
  t.after(async () => {
    await Promise.all(clients.map((value) => value.stop()));
    await stopServer();
    await rm(dir, { recursive: true, force: true });
  });
  return { path, messages, sockets, options, listen, stopServer, client, pi };
}

test("socket discovery follows engine paths and explicit overrides", () => {
  assert.equal(socketPath({}, "darwin", "/home/u"),
    "/home/u/Library/Application Support/wooting-signals/runtime/events.sock");
  assert.equal(socketPath({}, "linux", "/home/u"), "/home/u/.local/state/wooting-signals/events.sock");
  assert.equal(socketPath({ XDG_STATE_HOME: "/state" }, "linux", "/home/u"), "/state/wooting-signals/events.sock");
  assert.equal(socketPath({ WOOTING_STATE_DIR: "/dev-state" }, "darwin"), "/dev-state/events.sock");
  assert.equal(socketPath({ WOOTING_PI_SOCKET: "/custom.sock", WOOTING_STATE_DIR: "/ignored" }), "/custom.sock");
});

test("thin adapter registers only lifecycle handlers without inspecting Pi", () => {
  const pi = new FakePi();
  extension(pi);
  assert.deepEqual(pi.eventNames(), ["session_start", "agent_start", "agent_settled", "session_shutdown"]);
});

test("factory is inert; lifecycle facts renew, finish only on settled, and exclude sensitive data", async (t) => {
  const f = await fixture(t);
  await f.listen();
  const pi = f.pi();
  const sensitive = { prompt: "private-prompt", messages: "private-response", toolArguments: "private-args", command: "private-command" };
  await delay(60);
  assert.equal(f.messages.length, 0);
  await pi.dispatch("agent_start", sensitive); // No session means no resources.
  assert.equal(f.messages.length, 0);
  await pi.dispatch("session_start", sensitive);
  await until(() => f.messages.some((m) => m.payload.state === "idle"));
  await pi.dispatch("agent_start", sensitive);
  await until(() => f.messages.filter((m) => m.payload.state === "working").length >= 2);
  await pi.dispatch("agent_end", sensitive);
  assert.equal(f.messages.some((m) => m.payload.kind === "event"), false);
  await pi.dispatch("agent_settled", sensitive);
  await pi.dispatch("agent_settled", sensitive);
  await until(() => f.messages.some((m) => m.payload.event === "finished"));
  await pi.dispatch("session_shutdown", sensitive);
  assert.equal(f.messages.filter((m) => m.payload.kind === "event").length, 1);
  assert.equal(f.messages.at(-1).payload.kind, "remove");
  const event = f.messages.find((m) => m.payload.kind === "event");
  assert.ok(event.payload.ttl_ms > 0 && event.payload.ttl_ms <= 2000);
  assert.equal(f.messages.filter((m) => m.payload.kind === "state").every((m) => m.payload.ttl_ms === 15000), true);
  for (let i = 0; i < f.messages.length; i++) {
    const m = f.messages[i];
    assert.deepEqual(Object.keys(m).sort(), ["instance", "payload", "schema_version", "sequence", "source"]);
    assert.equal(m.source, "pi");
    assert.equal(m.schema_version, 1);
    assert.ok(Number.isSafeInteger(m.sequence));
    if (i) assert.ok(m.sequence > f.messages[i - 1].sequence);
  }
  assert.doesNotMatch(JSON.stringify(f.messages), /private-|prompt|messages|toolArguments|command|success/);
});

test("two live sessions stay independent; quit/reload/session replacement remove once and rotate identity", async (t) => {
  const f = await fixture(t);
  await f.listen();
  const a = f.pi();
  const b = f.pi();
  await a.dispatch("session_start");
  await b.dispatch("session_start");
  await until(() => new Set(f.messages.map((m) => m.instance)).size === 2);
  const first = f.messages[0].instance;
  await a.dispatch("agent_start");
  await until(() => f.messages.some((m) => m.instance === first && m.payload.state === "working"));
  for (const reason of ["quit", "reload", "new", "resume", "fork"]) {
    const previousIds = new Set(f.messages.map((m) => m.instance));
    await a.dispatch("session_shutdown", { reason });
    await a.dispatch("session_shutdown", { reason });
    await a.dispatch("session_start", { reason });
    await until(() => f.messages.some((m) => m.payload.kind === "state" && !previousIds.has(m.instance)));
  }
  await a.dispatch("session_shutdown");
  await b.dispatch("session_shutdown");
  for (const id of new Set(f.messages.map((m) => m.instance))) {
    assert.equal(f.messages.filter((m) => m.instance === id && m.payload.kind === "remove").length, 1);
  }
});

test("unexpected repeated session_start retires old session before starting another", async (t) => {
  const f = await fixture(t);
  await f.listen();
  const pi = f.pi();
  await pi.dispatch("session_start");
  await until(() => f.messages.length > 0);
  const first = f.messages[0].instance;
  await pi.dispatch("session_start");
  await until(() => f.messages.some((m) => m.instance !== first));
  assert.equal(f.messages.filter((m) => m.instance === first && m.payload.kind === "remove").length, 1);
  const retired = f.messages.findIndex((m) => m.payload.kind === "remove");
  const replacement = f.messages.findIndex((m) => m.instance !== first);
  assert.ok(retired < replacement);
});

test("unreachable daemon never blocks activity and reconnect publishes only current state", async (t) => {
  const f = await fixture(t);
  const c = f.client();
  c.start();
  const start = performance.now();
  for (let i = 0; i < 10000; i++) { c.working(); c.settled(); }
  c.working();
  assert.ok(performance.now() - start < 300, "activity must not wait for network delivery");
  await delay(120);
  assert.ok(c.backoffMs <= 40);
  assert.equal(c.pendingState, true);
  assert.equal(c.pendingEvent, undefined);
  await f.listen();
  await until(() => f.messages.length > 0);
  assert.equal(f.messages[0].payload.state, "working");
  assert.equal(f.messages.some((m) => m.payload.kind === "event"), false);
});

test("engine restart restores current working state with same identity, never old completion", async (t) => {
  const f = await fixture(t);
  await f.listen();
  const c = f.client();
  c.start(); c.working(); c.settled();
  await until(() => f.messages.some((m) => m.payload.event === "finished"));
  c.working();
  await until(() => f.messages.at(-1).payload.state === "working");
  const last = f.messages.at(-1);
  await f.stopServer();
  await delay(100);
  const count = f.messages.length;
  await f.listen();
  await until(() => f.messages.length > count);
  const resumed = f.messages[count];
  assert.equal(resumed.instance, last.instance);
  assert.ok(resumed.sequence > last.sequence);
  assert.equal(resumed.payload.state, "working");
  assert.equal(f.messages.slice(count).some((m) => m.payload.kind === "event"), false);
});

for (const response of ["lost_ack", "ignored_stale"]) {
  test(`completion is never retried after ${response}`, async (t) => {
    const f = await fixture(t, (m, socket) => {
      if (m.payload.kind === "event" && response === "lost_ack") socket.destroy();
      else ack(socket, response === "ignored_stale" ? "ignored_stale" : "applied");
    });
    await f.listen();
    const c = f.client();
    c.start(); c.working(); c.settled();
    await until(() => f.messages.some((m) => m.payload.kind === "event"));
    await delay(180);
    assert.equal(f.messages.filter((m) => m.payload.kind === "event").length, 1);
    assert.equal(f.messages.at(-1).payload.state, "idle");
  });
}

test("offline completion expires instead of replaying when engine appears", async (t) => {
  const f = await fixture(t);
  const c = f.client();
  c.start(); c.working(); c.settled();
  await delay(2100);
  await f.listen();
  await until(() => f.messages.length > 0);
  await delay(100);
  assert.equal(f.messages.some((m) => m.payload.kind === "event"), false);
  assert.equal(f.messages.at(-1).payload.state, "idle");
});

test("silent engine is bounded; stop cancels resources and is idempotent", async (t) => {
  const f = await fixture(t, () => {});
  await f.listen();
  const c = f.client({ heartbeatMs: 5, timeoutMs: 80 });
  c.start();
  await until(() => f.messages.length === 1);
  for (let i = 0; i < 1000; i++) { c.working(); c.settled(); }
  await delay(20);
  assert.equal(f.messages.length, 1, "only one request may be in flight");
  assert.equal(c.heartbeat.hasRef(), false);
  const start = performance.now();
  const stopped = c.stop();
  assert.equal(c.stop(), stopped);
  await stopped;
  assert.ok(performance.now() - start < 200);
  assert.equal(c.heartbeat, undefined);
  assert.equal(c.retry, undefined);
  assert.equal(c.inflight, undefined);
  assert.equal(c.pendingEvent, undefined);
  const count = f.messages.length;
  c.working(); c.settled(); c.start();
  await delay(120);
  assert.equal(f.messages.length, count);
  assert.equal(f.messages.filter((m) => m.payload.kind === "remove").length, 1);
  await until(() => f.sockets.size === 0);
});

for (const badAck of ["not-json\n", "x".repeat(4097), '{"schema_version":1,"ok":false,"error":"capacity"}\n']) {
  test(`invalid/rejected/oversized ACK fails closed (${badAck.length} bytes)`, async (t) => {
    const f = await fixture(t, (_m, socket) => socket.end(badAck));
    await f.listen();
    const c = f.client();
    c.start();
    await until(() => f.messages.length >= 2);
    assert.equal(f.messages.every((m) => m.payload.kind === "state"), true);
    await c.stop();
    assert.equal(c.inflight, undefined);
  });
}

test("safe-integer sequence limit disables connector without wrapping", async (t) => {
  const f = await fixture(t);
  await f.listen();
  const c = f.client();
  c.sequence = Number.MAX_SAFE_INTEGER - 1;
  c.start();
  await until(() => f.messages.length === 1);
  assert.equal(f.messages[0].sequence, Number.MAX_SAFE_INTEGER);
  c.working();
  await until(() => c.stopped);
  assert.equal(c.sequence, Number.MAX_SAFE_INTEGER);
  assert.equal(c.heartbeat, undefined);
});

test("dribbling reply cannot extend a request's hard deadline", async (t) => {
  const timers = new Set();
  const f = await fixture(t, (_m, socket) => {
    const timer = setInterval(() => socket.write(" "), 5);
    timers.add(timer);
    socket.once("close", () => { clearInterval(timer); timers.delete(timer); });
  });
  t.after(() => { for (const timer of timers) clearInterval(timer); });
  await f.listen();
  const c = f.client({ timeoutMs: 40, heartbeatMs: 10000 });
  c.start();
  await until(() => f.messages.length >= 2, 1000);
  assert.equal(f.messages.every((m) => m.payload.kind === "state"), true);
});
