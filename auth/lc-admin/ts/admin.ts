/**
 * Everything the console asks a browser to do: keep the address bar honest, and replace one
 * dialog. If it failed to load the console would be slower and entirely usable.
 */

import * as confirm from "./confirm.js";
import * as params from "./params.js";

function start(): void {
  params.install();
  confirm.install();
}

// A module is deferred, but can still load after `DOMContentLoaded` has fired.
if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", start, { once: true });
} else {
  start();
}
