// Fake only Pi's hook host; use the unmodified production connector and socket.
import { createInterface } from "node:readline";
import { registerPiConnector } from "../../connectors/pi/connector.mjs";

const hooks = new Map();
registerPiConnector({ on: (name, handler) => hooks.set(name, handler) }, {
  // Faster recovery for tests; wire leases remain the production 15 seconds.
  heartbeatMs: 100,
  backoffMinMs: 25,
  backoffMaxMs: 100,
});
for await (const hook of createInterface({ input: process.stdin })) {
  if (!hooks.has(hook)) throw new Error(`Unknown lifecycle hook: ${hook}`);
  await hooks.get(hook)();
  console.log(hook);
}
await hooks.get("session_shutdown")();
