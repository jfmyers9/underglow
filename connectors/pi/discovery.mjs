import { createHash } from "node:crypto";
import { closeSync, constants, fstatSync, lstatSync, openSync, readSync, realpathSync } from "node:fs";
import { join } from "node:path";
import { socketPath } from "./connector.mjs";

export function developmentRuntime(checkout, uid = process.getuid()) {
  const root = realpathSync(checkout);
  const digest = createHash("sha256").update(root, "utf8").digest("hex").slice(0, 12);
  return join("/tmp", `wsdev-${uid}-${digest}`);
}

function privateDirectory(path, uid) {
  const stat = lstatSync(path);
  return stat.isDirectory() && stat.uid === uid && (stat.mode & 0o777) === 0o700;
}

function alive(pid) {
  if (!Number.isSafeInteger(pid) || pid <= 0) return false;
  try { process.kill(pid, 0); return true; } catch { return false; }
}

// Creating a resolver is inert; inspect only when a session delivers/renews state.
// An absent/invalid descriptor NEVER falls back to the installed engine.
export function projectTargetResolver(checkout, env = process.env) {
  return () => {
    if (env.WOOTING_PI_SOCKET || env.WOOTING_STATE_DIR) {
      const path = socketPath(env);
      return { path, key: path };
    }
    let fd;
    try {
      const uid = process.getuid();
      const root = realpathSync(checkout);
      const runtimePath = developmentRuntime(root, uid);
      if (!privateDirectory(runtimePath, uid)) return undefined;
      // /tmp is /private/tmp on macOS; the supervisor publishes resolved paths.
      const runtime = realpathSync(runtimePath);
      const descriptor = join(runtime, "current.json");
      // NOFOLLOW plus fstat avoids following a replaced descriptor or reading a
      // device/FIFO. A bounded read also handles growth after the size check.
      fd = openSync(descriptor, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
      const stat = fstatSync(fd);
      if (!stat.isFile() || stat.uid !== uid || (stat.mode & 0o777) !== 0o600 || stat.size > 4096) return undefined;
      const bytes = Buffer.alloc(4097);
      const count = readSync(fd, bytes, 0, bytes.length, 0);
      if (count > 4096) return undefined;
      const data = JSON.parse(bytes.subarray(0, count).toString("utf8"));
      if (data.schema_version !== 1 || data.checkout !== root || realpathSync(data.checkout) !== root ||
          !["simulation", "hardware"].includes(data.mode) ||
          typeof data.nonce !== "string" || !data.nonce || data.nonce.length > 256 ||
          !alive(data.supervisor_pid) || !alive(data.engine_pid)) return undefined;
      const state = join(runtime, data.mode);
      const path = join(state, "events.sock");
      if (data.state_dir !== state || data.socket !== path || !privateDirectory(state, uid)) return undefined;
      const socket = lstatSync(path);
      if (!socket.isSocket() || socket.uid !== uid || (socket.mode & 0o077) !== 0) return undefined;
      return { path, key: `${path}\0${data.engine_pid}\0${data.nonce}` };
    } catch { return undefined; }
    finally { if (fd !== undefined) closeSync(fd); }
  };
}
