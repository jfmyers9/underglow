// Real connector + real checkout discovery; only Pi's lifecycle host is fake.
import { createInterface } from "node:readline";
import { pathToFileURL } from "node:url";
import { join } from "node:path";

const [repository, checkout] = process.argv.slice(2);
const { registerPiConnector } = await import(pathToFileURL(join(repository, "connectors/pi/connector.mjs")));
const { projectTargetResolver } = await import(pathToFileURL(join(repository, "connectors/pi/discovery.mjs")));
delete process.env.WOOTING_PI_SOCKET;
delete process.env.WOOTING_STATE_DIR;
const resolveTarget = projectTargetResolver(checkout);
const hooks = new Map();
registerPiConnector({ on: (name, callback) => hooks.set(name, callback) }, {
  resolveTarget,
  heartbeatMs: 40,
  backoffMinMs: 20,
  backoffMaxMs: 80,
});
const reply = (value) => console.log(JSON.stringify(value));
reply({ ready: true, target: resolveTarget() ?? null });
try {
  for await (const line of createInterface({ input: process.stdin })) {
    const { action } = JSON.parse(line);
    if (action !== "target") await hooks.get(action)?.();
    reply({ target: resolveTarget() ?? null });
  }
} finally {
  await hooks.get("session_shutdown")?.();
}
