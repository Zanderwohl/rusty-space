// The handover from the site to the game.
//
// The only script this site loads, and it is here because the alternative is shipping a 27 MB
// download to someone whose browser cannot use it. Everything it does before fetching is a
// reason not to fetch.
//
// It takes its configuration from `#boot`'s data attributes rather than from an inline script,
// so the page needs no `script-src` nonce.

const boot = document.getElementById('boot');
const stage = document.getElementById('stage');
const detail = document.getElementById('detail');
const bar = document.getElementById('bar');

const base = boot.dataset.base.replace(/\/$/, '');
const at = (path) => `${base}/${path}`;

const say = (s, d) => { stage.textContent = s; detail.innerHTML = d; };
const fail = (s, d) => {
  say(s, d);
  bar.hidden = true;
  boot.classList.add('boot-failed');
  throw new Error(s);
};

// `isSecureContext` first, and separately from WebGPU. They fail together — WebGPU is not
// exposed outside a secure context — and blaming the browser for what is actually the page's
// own URL sends people to download a browser they already have.
if (!window.isSecureContext) {
  fail('Needs a secure connection', 'WebGPU is only available over HTTPS, or from ' +
    '<code>localhost</code>. This page was served over plain HTTP, so the browser will not ' +
    'expose a GPU to it however capable it is.');
}
if (!navigator.gpu) {
  fail('WebGPU required', 'This build renders through WebGPU and has no WebGL fallback, ' +
    'because the sky needs storage buffers and a silently degraded one would be worse than ' +
    'this message. Try a current Chrome, Edge, or Safari 26+.');
}
if (!(await navigator.gpu.requestAdapter().catch(() => null))) {
  fail('No GPU adapter', 'The browser reports WebGPU but would not give this page an ' +
    'adapter. That is usually a disabled or blocklisted GPU driver.');
}

say('Loading', 'Fetching the manifest.');
const manifest = await fetch(at('manifest.json')).then((r) => r.json()).catch(() => null);
if (!manifest) {
  fail('Build not found', `Nothing answered at <code>${base}</code>. The site is pointing at ` +
    'a build that is not on the CDN.');
}

// The client resolves its own asset paths against this, and has no idea which page loaded it,
// so it has to be absolute. It reaches the client through the query string because that is
// the only argument vector a browser has.
const params = new URLSearchParams(location.search);
params.set('assets', new URL(at(manifest.asset_base || 'assets'), location.href).href);
history.replaceState(null, '', `${location.pathname}?${params}`);

say('Loading', `Downloading the client (${(manifest.bytes.wasm / 1e6).toFixed(0)} MB).`);
bar.hidden = false;
const response = await fetch(at(manifest.wasm));
if (!response.ok) fail('Download failed', `The client returned ${response.status}.`);

// A progress bar and streaming compilation are not a choice between two things. Tee the body
// through a counting transform and rebuild a Response around it -- carrying the original
// headers, or `application/wasm` goes with them and the browser stops streaming.
//
// `content-length` is the compressed length when the body is compressed, and the chunks
// arriving here are decompressed, so the two do not agree. Prefer the manifest's own figure.
const total = manifest.bytes.wasm || Number(response.headers.get('content-length')) || 0;
let seen = 0;
const counted = new Response(
  response.body.pipeThrough(new TransformStream({
    transform(chunk, controller) {
      seen += chunk.byteLength;
      if (total) bar.value = Math.min(100, (seen / total) * 100);
      controller.enqueue(chunk);
    },
  })),
  { headers: response.headers },
);

const init = (await import(new URL(at(manifest.entry), location.href).href)).default;
say('Starting', 'Compiling and handing over.');
try {
  await init({ module_or_path: counted });
} catch (e) {
  // Bevy's winit loop reports its exit by throwing. That is not a failure.
  if (!String(e).includes('Using exceptions for control flow')) {
    fail('Failed to start', `<code>${String(e).slice(0, 300)}</code>`);
  }
}
// Removed, not hidden. `#boot` is styled by an id selector, so `display: grid` outranks the
// `[hidden] { display: none }` the `hidden` property relies on, and the overlay would sit over
// a running game looking like it had never finished loading. Taking it out of the document
// settles that without a specificity argument, and drops it from the accessibility tree too.
boot.remove();
document.getElementById('lightcone').focus();
