import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { fileURLToPath } from "node:url";
import { registerPiConnector } from "../../../connectors/pi/connector.mjs";
import { projectTargetResolver } from "../../../connectors/pi/discovery.mjs";

export default function (pi: ExtensionAPI) {
  const checkout = fileURLToPath(new URL("../../../", import.meta.url));
  registerPiConnector(pi, { resolveTarget: projectTargetResolver(checkout) });
}
