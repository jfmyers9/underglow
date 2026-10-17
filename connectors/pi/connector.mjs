import { randomUUID } from "node:crypto";
import { Socket } from "node:net";
import { homedir } from "node:os";
import { join } from "node:path";
import { performance } from "node:perf_hooks";

export function socketPath(env = process.env, platform = process.platform, home = homedir()) {
  if (env.WOOTING_PI_SOCKET) return env.WOOTING_PI_SOCKET;
  const stateDir = env.WOOTING_STATE_DIR || (platform === "darwin"
    ? join(home, "Library", "Application Support", "wooting-signals", "runtime")
    : join(env.XDG_STATE_HOME || join(home, ".local", "state"), "wooting-signals"));
  return join(stateDir, "events.sock");
}

// One connection/request, one bounded response. No CLI, dependency, or persistent socket.
function request(path, message, timeoutMs) {
  const socket = new Socket();
  socket.unref();
  let cancel;
  const promise = new Promise((resolve) => {
    let response = "";
    let responseBytes = 0;
    let finished = false;
    const finish = (ok = false) => {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      socket.destroy();
      resolve(ok);
    };
    cancel = finish;
    // A hard deadline, not an inactivity timeout that a dribbling peer can extend.
    const timer = setTimeout(finish, timeoutMs);
    timer.unref();
    socket.on("error", () => finish());
    socket.on("close", () => finish());
    socket.on("data", (chunk) => {
      responseBytes += chunk.length;
      if (responseBytes > 4096) return finish();
      response += chunk.toString("utf8");
      const newline = response.indexOf("\n");
      if (newline < 0) return;
      try {
        const ack = JSON.parse(response.slice(0, newline));
        finish(ack.schema_version === 1 && ack.ok === true &&
          (ack.outcome === "applied" || ack.outcome === "ignored_stale"));
      } catch { finish(); }
    });
    try {
      socket.connect(path, () => {
        socket.unref();
        socket.write(`${JSON.stringify(message)}\n`);
      });
    } catch { finish(); }
  });
  return { promise, cancel: () => cancel() };
}

export class PiConnector {
  constructor(options = {}) {
    this.path = options.path ?? socketPath();
    this.resolveTarget = options.resolveTarget ?? (() => ({ path: this.path, key: this.path }));
    this.target = undefined;
    this.heartbeatMs = options.heartbeatMs ?? 5000;
    this.timeoutMs = options.timeoutMs ?? 500;
    this.shutdownMs = options.shutdownMs ?? 250;
    this.backoffMinMs = options.backoffMinMs ?? 250;
    this.backoffMaxMs = options.backoffMaxMs ?? 5000;
    this.backoffMs = this.backoffMinMs;
    this.instance = randomUUID();
    this.sequence = 0;
    this.state = "idle";
    this.pendingState = false;
    this.pendingEvent = undefined;
    this.inflight = undefined;
    this.heartbeat = undefined;
    this.retry = undefined;
    this.started = false;
    this.stopped = false;
    this.stopPromise = undefined;
  }

  start() {
    if (this.started || this.stopped) return;
    this.started = true;
    this.heartbeat = setInterval(() => this.publishState(), this.heartbeatMs);
    this.heartbeat.unref();
    this.publishState();
  }

  working() {
    if (!this.started || this.stopped) return;
    this.state = "working";
    // A notification for an earlier run is no longer useful.
    this.pendingEvent = undefined;
    this.publishState();
  }

  settled() {
    if (!this.started || this.stopped || this.state !== "working") return;
    this.state = "idle";
    this.pendingEvent = { deadline: performance.now() + 2000 };
    this.publishState();
  }

  publishState() {
    if (!this.started || this.stopped) return;
    this.pendingState = true; // Coalesce, never queue heartbeats or historical states.
    this.pump();
  }

  message(payload) {
    // Never produce an imprecise integer or reuse a sequence. Lease expiry cleans up
    // in the practically unreachable exhaustion case.
    if (this.sequence >= Number.MAX_SAFE_INTEGER) return undefined;
    return { schema_version: 1, source: "pi", instance: this.instance,
      sequence: ++this.sequence, payload };
  }

  pump() {
    if (this.stopped) return;
    const target = this.currentTarget();
    if (target?.key !== this.target?.key) {
      // Restart/mode changes must not replay a completion from the previous engine.
      this.inflight?.cancel();
      this.inflight = undefined;
      clearTimeout(this.retry);
      this.retry = undefined;
      this.pendingEvent = undefined;
      this.pendingState = true;
      this.target = target;
    }
    if (this.inflight || this.retry) return;
    if (!target) {
      this.pendingEvent = undefined;
      this.defer();
      return;
    }
    let payload;
    if (this.pendingState) {
      this.pendingState = false;
      payload = { kind: "state", state: this.state, ttl_ms: 15000 };
    } else if (this.pendingEvent) {
      const remaining = Math.floor(this.pendingEvent.deadline - performance.now());
      this.pendingEvent = undefined;
      if (remaining <= 0) return;
      payload = { kind: "event", event: "finished", ttl_ms: remaining };
    } else return;
    const message = this.message(payload);
    if (!message) {
      this.stop();
      return;
    }
    const operation = request(target.path, message, this.timeoutMs);
    this.inflight = operation;
    operation.promise.then((ok) => {
      if (this.inflight !== operation) return;
      this.inflight = undefined;
      if (this.stopped) return;
      if (ok) {
        this.backoffMs = this.backoffMinMs;
        this.pump();
      } else {
        // Events are at-most-once attempts. An absent ACK could mean the engine
        // applied one, then restarted; retrying would replay a stale completion.
        this.pendingState = true;
        this.defer();
      }
    });
  }

  currentTarget() {
    try {
      const target = this.resolveTarget();
      if (target && typeof target.path === "string" && target.path &&
          typeof target.key === "string" && target.key) return target;
    } catch { /* Discovery failure is an unavailable engine, never a Pi error. */ }
    return undefined;
  }

  defer() {
    this.pendingState = true;
    this.retry = setTimeout(() => {
      this.retry = undefined;
      this.pump();
    }, this.backoffMs);
    this.retry.unref();
    this.backoffMs = Math.min(this.backoffMs * 2, this.backoffMaxMs);
  }

  stop() {
    if (this.stopPromise) return this.stopPromise;
    this.stopped = true;
    clearInterval(this.heartbeat);
    clearTimeout(this.retry);
    this.heartbeat = this.retry = undefined;
    this.pendingState = false;
    this.pendingEvent = undefined;
    this.inflight?.cancel();
    this.inflight = undefined;
    const message = this.started ? this.message({ kind: "remove" }) : undefined;
    const target = this.currentTarget();
    if (!message || !target || target.key !== this.target?.key) {
      return this.stopPromise = Promise.resolve();
    }
    const operation = request(target.path, message, this.shutdownMs);
    this.inflight = operation;
    this.stopPromise = operation.promise.then(() => { this.inflight = undefined; });
    return this.stopPromise;
  }
}

// Factory registration is inert: no client, timer, socket, or session inspection.
// Ignore event/context arguments deliberately: only lifecycle facts leave Pi.
export function registerPiConnector(pi, options) {
  // APIs are per-extension wrappers, but Pi's event bus is shared and scoped to
  // the runtime. Arbitrate before session_start, including separately loaded
  // copies of this module. Project discovery outranks a generic installed copy.
  const channel = "underglow:pi-connector:claim:v1";
  const priority = options?.resolveTarget ? 1 : 0;
  let enabled = true;
  if (pi.events) {
    const claim = { priority, occupied: false };
    pi.events.emit(channel, claim);
    enabled = !claim.occupied;
    pi.events.on(channel, (other) => {
      if (!enabled) return;
      if (other.priority > priority) enabled = false;
      else other.occupied = true;
    });
  } else {
    // Minimal test hosts may not expose the shared bus.
    const key = Symbol.for("underglow.pi-connector.claim.v1");
    const previous = pi[key];
    if (previous && previous.priority >= priority) enabled = false;
    else {
      previous?.disable();
      pi[key] = { priority, disable: () => { enabled = false; } };
    }
  }
  let client;
  pi.on("session_start", async () => {
    if (!enabled) return;
    await client?.stop();
    client = new PiConnector(options);
    client.start();
  });
  pi.on("agent_start", () => { client?.working(); });
  pi.on("agent_settled", () => { client?.settled(); });
  pi.on("session_shutdown", () => client?.stop());
}
