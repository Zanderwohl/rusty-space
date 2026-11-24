// A small spinning galaxy, drawn into whatever box `#micro-galaxy` is given.
//
// The host sets its size with `width` and `height` attributes, in any CSS length; a bare number
// is pixels. Without `height` the box is 16:9 of its width. With JavaScript off the div stays
// empty and takes no room, so the page around it is unchanged.
//
// Two phases. Init bakes every texture once from a seeded noise table: a face-on disc, a bulge
// and one glow sprite per star color. Each frame after that is two transformed image draws and
// one sprite per star, with no per-pixel work.
//
// The "3D" is an orthographic camera ELEVATION above the plane. A disc point at (r, θ) lands at
// (r cos θ, r sin θ · sin ELEVATION) on screen, so the face-on texture rotated by θ and squashed
// vertically is the tilted disc. Positive screen y is the near side.
//
// `data-debug` on the host outlines the disc ellipse and the bulge, and marks θ = 0.

const ELEVATION = (30 * Math.PI) / 180;
const SQUASH = Math.sin(ELEVATION);
const LIFT = Math.cos(ELEVATION);
const PERIOD_MS = 60_000;

// Disc radius is 1. Everything below is in that unit.
const ARMS = 2;
const PITCH = (13 * Math.PI) / 180;
const WIND = 1 / Math.tan(PITCH);
const BULGE_RADIUS = 0.3;
// Polar axis over equatorial axis of the bulge spheroid. 1 would draw a circle.
const BULGE_FLATTENING = 0.65;
const BULGE_SQUASH = Math.hypot(BULGE_FLATTENING * LIFT, SQUASH);

const SEED = 0x1c0e;
const NOISE_SIZE = 64; // power of two: lookups wrap with a mask
const DISC_TEXELS = 512;
const BULGE_TEXELS = 128;
const SPRITE_TEXELS = 32;
const STAR_COUNT = 280;
// The largest stars twinkle by walking this loop of brightness factors, each at its own rate
// and starting point. Cosine-eased between keys, so a fixed curve rather than per-frame noise.
const TWINKLERS = 20;
const TWINKLE_CURVE = [1, 0.72, 0.9, 0.5, 0.95, 0.62, 0.84, 0.45, 0.78, 0.98];

// Kept dark in the light theme too: the galaxy is drawn additively and needs a dark field.
const SPACE = "#0b0d12";

// Approximate blackbody colors, hot to cool.
const STAR_COLORS = [
  [155, 176, 255],
  [202, 215, 255],
  [248, 247, 255],
  [255, 236, 214],
  [255, 210, 161],
  [255, 178, 120],
];

function mulberry32(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function gaussian(rand) {
  return Math.sqrt(-2 * Math.log(1 - rand())) * Math.cos(2 * Math.PI * rand());
}

function smoothstep(lo, hi, x) {
  const t = Math.min(1, Math.max(0, (x - lo) / (hi - lo)));
  return t * t * (3 - 2 * t);
}

function mix(a, b, t) {
  return a + (b - a) * t;
}

// Tileable value noise over a table filled once. Returns fractal noise in [0, 1].
function makeNoise(rand) {
  const lut = new Float32Array(NOISE_SIZE * NOISE_SIZE);
  for (let i = 0; i < lut.length; i++) lut[i] = rand();
  const mask = NOISE_SIZE - 1;

  function sample(x, y) {
    const xi = Math.floor(x);
    const yi = Math.floor(y);
    let fx = x - xi;
    let fy = y - yi;
    fx = fx * fx * (3 - 2 * fx);
    fy = fy * fy * (3 - 2 * fy);
    const x0 = xi & mask;
    const x1 = (xi + 1) & mask;
    const y0 = (yi & mask) * NOISE_SIZE;
    const y1 = ((yi + 1) & mask) * NOISE_SIZE;
    const a = lut[y0 + x0];
    const b = lut[y0 + x1];
    const c = lut[y1 + x0];
    const d = lut[y1 + x1];
    return a + (b - a) * fx + (c - a) * fy + (a - b - c + d) * fx * fy;
  }

  return (x, y, octaves) => {
    let sum = 0;
    let amp = 0.5;
    let norm = 0;
    for (let o = 0; o < octaves; o++) {
      sum += amp * sample(x, y);
      norm += amp;
      // The offset keeps octaves from sharing a lattice origin, which shows as a cross.
      x = x * 2 + 17.3;
      y = y * 2 + 31.7;
      amp *= 0.5;
    }
    return sum / norm;
  };
}

// Fractal value noise bunches around 0.5; this spreads it back over [0, 1].
function contrast(n, gain) {
  return Math.min(1, Math.max(0, (n - 0.5) * gain + 0.5));
}

function canvasOf(size) {
  const c = document.createElement("canvas");
  c.width = size;
  c.height = size;
  return c;
}

// The face-on disc: exponential falloff, two trailing log-spiral arms broken into clouds, faint
// filaments between them, a dust lane along each arm's inner edge, and pink knots where the arm
// is densest. RGB is the color and alpha the brightness, since it is drawn with "lighter".
function bakeDisc(fbm) {
  const canvas = canvasOf(DISC_TEXELS);
  const ctx = canvas.getContext("2d");
  const image = ctx.createImageData(DISC_TEXELS, DISC_TEXELS);
  const px = image.data;

  for (let j = 0; j < DISC_TEXELS; j++) {
    const v = ((j + 0.5) / DISC_TEXELS) * 2 - 1;
    for (let i = 0; i < DISC_TEXELS; i++) {
      const u = ((i + 0.5) / DISC_TEXELS) * 2 - 1;
      const r = Math.hypot(u, v);
      if (r >= 1) continue;

      const warp = fbm(u * 2.5 + 5, v * 2.5 + 5, 3) - 0.5;
      // Trailing: the arm sits at θ = -WIND·ln r, so it falls behind as r grows. The small
      // offset keeps ln finite at the center texel.
      const phase = ARMS * (Math.atan2(v, u) + WIND * Math.log(r + 0.02)) + warp * 3;
      const arm = Math.pow(0.5 + 0.5 * Math.cos(phase), 2.5);
      const lane = Math.pow(0.5 + 0.5 * Math.cos(phase + 0.75), 10);
      const reach = smoothstep(0.1, 0.35, r);
      const cloud = contrast(fbm(u * 7, v * 7, 4), 2.4);
      const dust = contrast(fbm(u * 11 + 3, v * 11 + 9, 3), 2);

      const falloff = Math.exp(-r / 0.42) * (1 - smoothstep(0.6, 1, r));
      // Soft clouds of gas between the arms. Ridged noise drew sharp veins here.
      const haze = 0.2 + 0.2 * contrast(fbm(u * 3.5 + 21, v * 3.5 + 13, 4), 1.8);
      const structure = mix(0.55, haze + arm * (0.35 + 1.1 * cloud), reach);
      const shadow = 1 - 0.7 * lane * reach * dust;
      const bright = falloff * structure * shadow;

      const young = reach * arm;
      const knot = young * Math.max(0, cloud - 0.72) * 3.5;
      const at = (j * DISC_TEXELS + i) * 4;
      px[at] = Math.min(255, mix(255, 175, young) + 80 * knot);
      px[at + 1] = Math.min(255, mix(214, 200, young) + 10 * knot);
      px[at + 2] = Math.min(255, mix(160, 255, young) + 40 * knot);
      px[at + 3] = Math.min(255, 255 * 2.4 * bright * (1 + knot));
    }
  }
  ctx.putImageData(image, 0, 0);
  return canvas;
}

function bakeBulge(fbm) {
  const canvas = canvasOf(BULGE_TEXELS);
  const ctx = canvas.getContext("2d");
  const image = ctx.createImageData(BULGE_TEXELS, BULGE_TEXELS);
  const px = image.data;

  for (let j = 0; j < BULGE_TEXELS; j++) {
    const v = ((j + 0.5) / BULGE_TEXELS) * 2 - 1;
    for (let i = 0; i < BULGE_TEXELS; i++) {
      const u = ((i + 0.5) / BULGE_TEXELS) * 2 - 1;
      const r = Math.hypot(u, v);
      if (r >= 1) continue;
      const mottle = 0.85 + 0.3 * fbm(u * 5 + 40, v * 5 + 40, 3);
      const core = Math.exp((-r * r) / 0.02);
      const halo = Math.exp(-r / 0.22) * (1 - smoothstep(0.6, 1, r));
      const at = (j * BULGE_TEXELS + i) * 4;
      px[at] = 255;
      px[at + 1] = mix(196, 248, core);
      px[at + 2] = mix(130, 235, core);
      px[at + 3] = Math.min(255, 255 * (core + 0.55 * halo * mottle));
    }
  }
  ctx.putImageData(image, 0, 0);
  return canvas;
}

// White in the middle, the star's color at the rim, fading to nothing.
function bakeSprite([r, g, b]) {
  const canvas = canvasOf(SPRITE_TEXELS);
  const ctx = canvas.getContext("2d");
  const h = SPRITE_TEXELS / 2;
  const grad = ctx.createRadialGradient(h, h, 0, h, h, h);
  grad.addColorStop(0, "rgba(255,255,255,1)");
  grad.addColorStop(0.12, "rgba(255,255,255,0.95)");
  grad.addColorStop(0.3, `rgba(${r},${g},${b},0.55)`);
  grad.addColorStop(0.6, `rgba(${r},${g},${b},0.12)`);
  grad.addColorStop(1, `rgba(${r},${g},${b},0)`);
  ctx.fillStyle = grad;
  ctx.fillRect(0, 0, SPRITE_TEXELS, SPRITE_TEXELS);
  return canvas;
}

// Most bright stars are young and sit in the arms; the rest are older field stars.
function scatterStars(rand) {
  const stars = [];
  for (let k = 0; k < STAR_COUNT; k++) {
    let r;
    let theta;
    let color;
    let size;
    if (rand() < 0.65) {
      r = 0.18 + 0.75 * Math.pow(rand(), 0.8);
      const arm = Math.floor(rand() * ARMS);
      theta = (2 * Math.PI * arm) / ARMS - WIND * Math.log(r) + 0.35 * gaussian(rand);
      color = Math.floor(rand() * 3);
      size = 0.7 + 0.8 * rand();
    } else {
      r = 0.08 + 0.85 * Math.pow(rand(), 1.4);
      theta = 2 * Math.PI * rand();
      color = 2 + Math.floor(rand() * 4);
      size = 0.45 + 0.5 * rand();
    }
    stars.push({
      r,
      theta,
      z: 0.015 * gaussian(rand),
      sprite: color,
      size,
      alpha: 0.35 + 0.6 * rand(),
      twinkle: null,
    });
  }
  const largest = [...stars].sort((a, b) => b.size - a.size).slice(0, TWINKLERS);
  for (const star of largest) {
    star.alpha = 1;
    // Keys per second, so one pass of the curve takes about 11 to 25 s.
    star.twinkle = { rate: 0.4 + 0.5 * rand(), offset: rand() * TWINKLE_CURVE.length };
  }
  return stars;
}

function initResources() {
  const rand = mulberry32(SEED);
  const fbm = makeNoise(rand);
  return {
    disc: bakeDisc(fbm),
    bulge: bakeBulge(fbm),
    sprites: STAR_COLORS.map(bakeSprite),
    stars: scatterStars(rand),
  };
}

function cssLength(value) {
  return /^\d+(\.\d+)?$/.test(value.trim()) ? `${value.trim()}px` : value;
}

function sizeHost(host) {
  const width = host.getAttribute("width");
  const height = host.getAttribute("height");
  if (width) host.style.width = cssLength(width);
  if (height) host.style.height = cssLength(height);
  else host.style.aspectRatio = "16 / 9";
}

// `t` in [0, 1) is the fraction of one turn; `seconds` is running time, for twinkling.
function draw(ctx, res, t, seconds, debug) {
  const { width: w, height: h } = ctx.canvas;
  const cx = w / 2;
  const cy = h / 2;
  const radius = 0.47 * Math.min(w, h / SQUASH);
  const spin = 2 * Math.PI * t;

  ctx.globalCompositeOperation = "source-over";
  ctx.globalAlpha = 1;
  ctx.fillStyle = SPACE;
  ctx.fillRect(0, 0, w, h);
  ctx.globalCompositeOperation = "lighter";

  ctx.save();
  ctx.translate(cx, cy);
  ctx.scale(1, SQUASH);
  ctx.rotate(spin);
  ctx.drawImage(res.disc, -radius, -radius, 2 * radius, 2 * radius);
  ctx.restore();

  const bx = radius * BULGE_RADIUS;
  const by = bx * BULGE_SQUASH;
  const starPx = radius * 0.018;

  // Far-side stars first, then the bulge, then the near side. Additive drawing ignores order,
  // so a far star behind the bulge is dimmed by hand.
  const near = [];
  for (const star of res.stars) {
    const a = star.theta + spin;
    const sin = Math.sin(a);
    const x = cx + radius * star.r * Math.cos(a);
    const y = cy + radius * (star.r * sin * SQUASH - star.z * LIFT);
    if (sin > 0) {
      near.push(star, x, y);
      continue;
    }
    const inside = Math.hypot((x - cx) / bx, (y - cy) / by);
    drawStar(ctx, res, star, x, y, starPx, smoothstep(0.25, 0.9, inside), seconds);
  }

  ctx.globalAlpha = 1;
  ctx.drawImage(res.bulge, cx - bx, cy - by, 2 * bx, 2 * by);

  for (let i = 0; i < near.length; i += 3) {
    drawStar(ctx, res, near[i], near[i + 1], near[i + 2], starPx, 1, seconds);
  }

  if (debug) drawDebug(ctx, cx, cy, radius, bx, by, spin, t);
}

function twinkle(star, seconds) {
  if (!star.twinkle) return 1;
  const n = TWINKLE_CURVE.length;
  const at = (star.twinkle.offset + seconds * star.twinkle.rate) % n;
  const i = Math.floor(at);
  const ease = 0.5 - 0.5 * Math.cos(Math.PI * (at - i));
  return mix(TWINKLE_CURVE[i], TWINKLE_CURVE[(i + 1) % n], ease);
}

function drawStar(ctx, res, star, x, y, starPx, visible, seconds) {
  if (visible <= 0) return;
  const glow = twinkle(star, seconds);
  const s = starPx * star.size * (0.85 + 0.15 * glow);
  ctx.globalAlpha = star.alpha * visible * glow;
  ctx.drawImage(res.sprites[star.sprite], x - s, y - s, 2 * s, 2 * s);
}

function drawDebug(ctx, cx, cy, radius, bx, by, spin, t) {
  ctx.globalCompositeOperation = "source-over";
  ctx.globalAlpha = 1;
  ctx.lineWidth = Math.max(1, radius / 300);
  ctx.strokeStyle = "#7fd4d8";
  ctx.beginPath();
  ctx.ellipse(cx, cy, radius, radius * SQUASH, 0, 0, 2 * Math.PI);
  ctx.stroke();
  ctx.strokeStyle = "#ffd27f";
  ctx.beginPath();
  ctx.ellipse(cx, cy, bx, by, 0, 0, 2 * Math.PI);
  ctx.stroke();

  const mx = cx + radius * Math.cos(spin);
  const my = cy + radius * Math.sin(spin) * SQUASH;
  ctx.strokeStyle = "#ff7f9f";
  ctx.beginPath();
  ctx.moveTo(cx, cy);
  ctx.lineTo(mx, my);
  ctx.stroke();

  ctx.fillStyle = "#dfe3ec";
  ctx.font = `${Math.round(radius / 18)}px ui-monospace, monospace`;
  ctx.fillText(`t = ${t.toFixed(3)}`, ctx.lineWidth * 8, radius / 12);
}

function start(host) {
  sizeHost(host);
  const canvas = document.createElement("canvas");
  canvas.style.display = "block";
  canvas.style.width = "100%";
  canvas.style.height = "100%";
  canvas.setAttribute("aria-hidden", "true");
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  host.replaceChildren(canvas);

  const res = initResources();
  const debug = host.hasAttribute("data-debug");
  const still = matchMedia("(prefers-reduced-motion: reduce)");
  const epoch = performance.now();
  let visible = true;
  let pending = 0;

  const render = (now) => {
    pending = 0;
    const elapsed = still.matches ? 0 : now - epoch;
    draw(ctx, res, (elapsed % PERIOD_MS) / PERIOD_MS, elapsed / 1000, debug);
    if (visible && !still.matches) pending = requestAnimationFrame(render);
  };
  const resume = () => {
    if (!pending) pending = requestAnimationFrame(render);
  };

  new ResizeObserver(([entry]) => {
    const box = entry.devicePixelContentBoxSize?.[0];
    const dpr = devicePixelRatio || 1;
    canvas.width = Math.max(1, box ? box.inlineSize : Math.round(entry.contentRect.width * dpr));
    canvas.height = Math.max(1, box ? box.blockSize : Math.round(entry.contentRect.height * dpr));
    resume();
  }).observe(host);

  new IntersectionObserver(([entry]) => {
    visible = entry.isIntersecting;
    if (visible) resume();
  }).observe(host);

  still.addEventListener("change", resume);
}

const host = document.getElementById("micro-galaxy");
if (host) start(host);
