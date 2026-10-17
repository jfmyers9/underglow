import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { chmod, mkdir, mkdtemp, realpath, rename, rm, symlink, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { performance } from "node:perf_hooks";
import test from "node:test";
import projectExtension from "../../.pi/extensions/underglow/index.ts";
import { PiConnector, registerPiConnector } from "./connector.mjs";
import { developmentRuntime, projectTargetResolver } from "./discovery.mjs";

async function until(predicate) {
  const end = performance.now() + 2000;
  while (!predicate()) {
    assert.ok(performance.now() < end, "condition timed out");
    await delay(5);
  }
}

async function fixture(t) {
  const checkout = await realpath(await mkdtemp("/tmp/pi-project-"));
  const runtimePath = developmentRuntime(checkout);
  await mkdir(runtimePath, { mode: 0o700 });
  const runtime = await realpath(runtimePath);
  const current = join(runtime, "current.json");
  const engines = [];
  const clients = [];
  const descriptor = (mode = "simulation", overrides = {}) => ({
    schema_version: 1, checkout, state_dir: join(runtime, mode),
    socket: join(runtime, mode, "events.sock"), mode,
    supervisor_pid: process.pid, engine_pid: process.pid, nonce: "first-engine",
    ...overrides,
  });
  const publish = async (data = descriptor()) => {
    const temp = join(runtime, "next.json");
    await writeFile(temp, JSON.stringify(data), { mode: 0o600 });
    await rename(temp, current);
  };
  const engine = async (mode = "simulation", handler) => {
    const state = join(runtime, mode);
    await mkdir(state, { mode: 0o700 });
    const path = join(state, "events.sock");
    const messages = [];
    const sockets = new Set();
    const server = createServer((socket) => {
      sockets.add(socket);
      socket.on("error", () => {});
      socket.on("close", () => sockets.delete(socket));
      let buffer = "";
      socket.on("data", (bytes) => {
        buffer += bytes;
        if (!buffer.includes("\n")) return;
        const message = JSON.parse(buffer.split("\n")[0]);
        messages.push(message);
        if (handler) handler(message, socket);
        else socket.end('{"schema_version":1,"ok":true,"outcome":"applied"}\n');
      });
    });
    await new Promise((resolve, reject) => {
      server.once("error", reject);
      server.listen(path, resolve);
    });
    await chmod(path, 0o600);
    engines.push({ server, sockets });
    return { path, messages, state };
  };
  const options = { heartbeatMs: 20, timeoutMs: 100, shutdownMs: 30,
    backoffMinMs: 10, backoffMaxMs: 40, resolveTarget: projectTargetResolver(checkout, {}) };
  const client = () => {
    const value = new PiConnector(options);
    clients.push(value);
    return value;
  };
  t.after(async () => {
    await Promise.all(clients.map((c) => c.stop()));
    for (const { server, sockets } of engines) {
      for (const socket of sockets) socket.destroy();
      await new Promise((resolve) => server.close(resolve));
    }
    await rm(runtime, { recursive: true, force: true });
    await rm(checkout, { recursive: true, force: true });
  });
  return { checkout, runtime, current, descriptor, publish, engine, client, options };
}

test("project adapter loads and registers without starting resources", () => {
  const names = [];
  projectExtension({ on: (name) => names.push(name) });
  assert.deepEqual(names, ["session_start", "agent_start", "agent_settled", "session_shutdown"]);
});

test("Pi before dev reconnects, follows mode switches, and never uses an installed fallback", async (t) => {
  const f = await fixture(t);
  const simulation = await f.engine();
  const hardware = await f.engine("hardware");
  const c = f.client();
  c.start(); c.working(); c.settled();
  await delay(60);
  assert.equal(simulation.messages.length, 0, "a socket alone is not discovery");
  assert.equal(f.options.resolveTarget(), undefined);
  await f.publish();
  await until(() => simulation.messages.length > 0);
  const instance = simulation.messages[0].instance;
  assert.equal(simulation.messages[0].payload.state, "idle");
  assert.equal(simulation.messages.some((m) => m.payload.kind === "event"), false);
  c.working();
  await until(() => simulation.messages.some((m) => m.payload.state === "working"));
  await rm(f.current);
  const before = simulation.messages.length;
  c.settled();
  await delay(60);
  assert.equal(simulation.messages.length, before, "removal immediately revokes old socket");
  await f.publish(f.descriptor("hardware", { nonce: "second-engine" }));
  await until(() => hardware.messages.length > 0);
  assert.equal(hardware.messages[0].instance, instance);
  assert.ok(hardware.messages[0].sequence > simulation.messages.at(-1).sequence);
  assert.equal(hardware.messages[0].payload.state, "idle");
  assert.equal(hardware.messages.some((m) => m.payload.kind === "event"), false);
  await c.stop();
  assert.equal(hardware.messages.at(-1).payload.kind, "remove");
});

test("new engine nonce at the same socket cancels in-flight work and drops old completion", async (t) => {
  const f = await fixture(t);
  let acknowledge = false;
  const engine = await f.engine("simulation", (_message, socket) => {
    if (acknowledge) socket.end('{"schema_version":1,"ok":true,"outcome":"applied"}\n');
  });
  await f.publish();
  const c = f.client();
  c.start(); c.working(); c.settled();
  await until(() => engine.messages.length > 0);
  const count = engine.messages.length;
  acknowledge = true;
  await f.publish(f.descriptor("simulation", { nonce: "restarted" }));
  c.publishState();
  await until(() => engine.messages.length > count);
  await delay(50);
  assert.equal(engine.messages.slice(count).some((m) => m.payload.kind === "event"), false);
  assert.equal(engine.messages[count].payload.state, "idle");
});

test("discovery rejects stale/malformed/redirected descriptors and unsafe filesystem entries", async (t) => {
  const f = await fixture(t);
  const engine = await f.engine();
  const resolve = f.options.resolveTarget;
  await f.publish();
  assert.equal(resolve().path, engine.path);
  for (const changes of [
    { schema_version: 2 }, { checkout: "/" }, { mode: "../simulation" },
    { state_dir: "/tmp" }, { socket: "/tmp/installed.sock" }, { nonce: "" },
    { supervisor_pid: 2147483647 }, { engine_pid: 2147483647 }, { engine_pid: 0 },
    { engine_pid: "1" },
  ]) {
    await f.publish(f.descriptor("simulation", changes));
    assert.equal(resolve(), undefined, JSON.stringify(changes));
  }
  for (const contents of ["null", "{", "x".repeat(4097)]) {
    await writeFile(f.current, contents);
    assert.equal(resolve(), undefined);
  }
  await f.publish();
  for (const [path, unsafe, safe] of [
    [f.current, 0o644, 0o600], [f.runtime, 0o755, 0o700],
    [engine.state, 0o755, 0o700], [engine.path, 0o666, 0o600],
  ]) {
    await chmod(path, unsafe);
    assert.equal(resolve(), undefined, path);
    await chmod(path, safe);
  }
  await rename(f.current, join(f.runtime, "saved.json"));
  await symlink(join(f.runtime, "saved.json"), f.current);
  assert.equal(resolve(), undefined, "descriptor symlink");
  await rm(f.current);
  await mkdir(f.current, { mode: 0o700 });
  assert.equal(resolve(), undefined, "descriptor is not a file");
  await rm(f.current, { recursive: true });
  await f.publish();
  await rename(engine.state, `${engine.state}-real`);
  await symlink(`${engine.state}-real`, engine.state);
  assert.equal(resolve(), undefined, "state directory symlink");
  await rm(engine.state);
  await rename(`${engine.state}-real`, engine.state);
  await rename(engine.path, `${engine.path}-real`);
  await symlink(`${engine.path}-real`, engine.path);
  assert.equal(resolve(), undefined, "socket symlink");
});

test("explicit overrides remain authoritative even without checkout/dev metadata", () => {
  assert.deepEqual(projectTargetResolver("/does-not-exist", {
    WOOTING_PI_SOCKET: "/explicit.sock", WOOTING_STATE_DIR: "/ignored",
  })(), { path: "/explicit.sock", key: "/explicit.sock" });
  assert.deepEqual(projectTargetResolver("/does-not-exist", {
    WOOTING_STATE_DIR: "/explicit-state",
  })(), { path: "/explicit-state/events.sock", key: "/explicit-state/events.sock" });
  assert.equal(projectTargetResolver("/does-not-exist", {})(), undefined);
});

for (const order of ["generic-first", "project-first"]) {
  test(`duplicate adapters publish only one lifecycle, project wins (${order})`, async (t) => {
    const f = await fixture(t);
    const engine = await f.engine();
    const installed = await f.engine("hardware");
    const bus = new EventEmitter();
    const apis = [new EventEmitter(), new EventEmitter()];
    for (const api of apis) api.events = { emit: (...args) => bus.emit(...args), on: (...args) => bus.on(...args) };
    const generic = () => registerPiConnector(apis[0], { ...f.options, resolveTarget: undefined, path: installed.path });
    const project = () => registerPiConnector(apis[1], f.options);
    if (order === "generic-first") { generic(); project(); } else { project(); generic(); }
    // Also loading the explicit project copy must not add a second publisher.
    registerPiConnector(apis[1], f.options);
    const dispatch = async (name) => {
      for (const api of apis) for (const handler of api.listeners(name)) await handler();
    };
    t.after(() => dispatch("session_shutdown"));
    await delay(30);
    assert.equal(engine.messages.length, 0);
    await dispatch("session_start");
    await delay(50);
    assert.equal(engine.messages.length, 0, "project waits for ready metadata");
    assert.equal(installed.messages.length, 0, "generic adapter cannot leak to installed engine");
    await f.publish();
    await until(() => engine.messages.length > 0);
    await dispatch("agent_start");
    await until(() => engine.messages.some((m) => m.payload.state === "working"));
    await dispatch("agent_settled");
    await until(() => engine.messages.some((m) => m.payload.kind === "event"));
    await dispatch("session_shutdown");
    assert.equal(installed.messages.length, 0);
    assert.equal(new Set(engine.messages.map((m) => m.instance)).size, 1);
    assert.equal(engine.messages.filter((m) => m.payload.kind === "event").length, 1);
    assert.equal(engine.messages.filter((m) => m.payload.kind === "remove").length, 1);
  });
}
