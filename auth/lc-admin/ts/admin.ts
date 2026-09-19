/**
 * Everything the console asks a browser to do.
 *
 * Two modules and no framework. The pages are HTML the server rendered; htmx swaps parts of
 * them; this keeps the address bar honest and replaces one dialog. If it failed to load, the
 * console would be slower and entirely usable — which is the standard the rest of the site
 * holds itself to and there is no reason for an administration page to hold itself to a
 * lower one.
 */

import * as confirm from "./confirm.js";
import * as params from "./params.js";

function start(): void {
  params.install();
  confirm.install();
}

// `defer` on the tag would do this for a classic script; a module is deferred already, but it
// can also be loaded after `DOMContentLoaded` has fired — by a bundler, or by a browser
// extension — and then the listener never runs.
if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", start, { once: true });
} else {
  start();
}
