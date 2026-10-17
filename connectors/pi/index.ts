import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { registerPiConnector } from "./connector.mjs";

export default function (pi: ExtensionAPI) {
  registerPiConnector(pi);
}
