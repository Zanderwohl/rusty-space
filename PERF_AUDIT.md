# Performance audit — last 8h of work

Commits reviewed: `04eae38` → `37b8588` (kepler caching, settings tabs, starfield radius/brightness, body-point AA & energy spread, fade model).

Scope you flagged: the **starfield/Distant Object shader** and the **debug-ball distance fade**. Findings below, ordered by impact.

---

## 1. Starfield fragment shader — the dominant cost (fix this first)

`assets/shaders/starfield.wgsl` renders the sky as a full-screen sphere and, **for every fragment, loops over every star**:

```wgsl
for (var i: u32 = 0u; i < material.star_count; i = i + 1u) {
    let star = stars[i];
    ...
    let size_t = clamp((MAG_FAINT - mag) / (MAG_FAINT - MAG_BRIGHT), 0.0, 1.0);
    let radius_arcmin = mix(material.star_radius_min, material.star_radius_max, size_t);
    let threshold = cos(radius_arcmin * ARCMIN_TO_RAD);   // per pixel, per star
    if alignment > threshold {
        let brightness = MAG_ZERO_BRIGHTNESS * pow(2.512, -mag) * BRIGHTNESS_SCALE; // per pixel, per star
        ...
    }
}
```

Catalog size is ~700 stars (`hygdata_v42_dist_sort.csv` 200 + `..._mag_sort.csv` 500, deduped). At 1080p that's ~2M fragments × ~700 = **~1.4 billion iterations/frame**. Commit `1e453b1` made each iteration *heavier* by adding a per-pixel `cos()`, `mix()` and `pow()` — none of which depend on the fragment.

### 1a. Cheap win — hoist per-star constants out of the inner loop
`brightness` depends only on `mag` (fully static). `threshold` depends on `mag` + the two radius uniforms, which only change when the user drags a settings slider. Precompute both into a GPU buffer and the inner loop collapses to `dot → compare → falloff → accumulate` (~3 ops, no transcendentals).

- `brightness`: bake once at spawn. You already build `color_data`; multiply color by brightness there, or store brightness in an unused channel.
- `threshold`: recompute the buffer only inside `update_starfield_brightness` when `star_radius_min/max` actually change (you already have that change-detection guard — extend it to rewrite a `thresholds` storage buffer instead of just the uniform).

Expected: roughly 2–4× on the starfield pass for a one-evening change, no architecture shift.

### 1b. Real fix — stop scanning all stars per pixel
The loop is `O(pixels × stars)`. The correct architecture is `O(stars)`: render each star as an **instanced billboarded quad / point sprite** (one instance per star, ~700 instances), sized by the same arcmin→radius math in the *vertex* shader, with a cheap radial falloff in the fragment shader. The full-screen sphere scan goes away entirely. This is the high-leverage change if the starfield is a measured bottleneck; 1a is the stopgap.

---

## 2. `update_body_points` — redundant work inside the star loop

`src/presentation/body_point.rs`, per body per frame:

```rust
for star in &star_cache.stars {
    let to_camera = (-body_center).normalize();          // loop-invariant — hoist out
    let to_star = (star.bevy_position - body_center).normalize();
    ...
    let body_to_star_dist = (star.bevy_position - body_center).length(); // delta recomputed
}
```

- `to_camera` is constant for the body — compute it once before the loop.
- `star.bevy_position - body_center` is computed twice (normalize + length). Compute the delta once, take `length()` from it, normalize by dividing.

Impact is small in practice — `star_cache.stars` only holds `Appearance::Star` entities (the sim's suns, usually 1–few), not the 700-star catalog — but it's a trivial, free cleanup. **Note:** the catalog scan in §1 and this lighting loop are different star sets; don't conflate them.

Your GPU-upload hygiene here is actually good: every material mutation (`brightness`, wireframe `emission_strength`, occluder `alpha`) is guarded by a `> 0.001` change check before `get_mut`, so you're *not* re-uploading uniforms that didn't change. That's the right pattern — keep it.

---

## 3. Fade model mesh churn — the "copying over and over" you suspected

`update_wireframe_thickness` and `update_terminator_meshes` (`src/presentation/body_mesh.rs`) **regenerate the entire mesh** (rebuild every tube ring with `sin_cos`, allocate fresh `Vec`s, re-upload vertex/index buffers) whenever the distance-driven tube radius crosses a ±5% band:

```rust
let ratio = required_radius / wireframe.current_tube_radius;
if ratio < 0.95 || ratio > 1.05 {
    let new_mesh = generate_latlon_sphere(&wireframe.highlight_latitudes, required_radius, TUBE_SIDES);
    if let Some(mesh_asset) = meshes.get_mut(&mesh3d.0) { *mesh_asset = new_mesh; }
}
```

During a continuous zoom the 5% band is crossed repeatedly, so you rebuild + reallocate + re-upload geometry many frames in a row, for every debug ball. The 5% hysteresis bounds it but doesn't stop the churn while zooming.

Options, cheapest first:
- **Scale instead of rebuild.** Tube *thickness* scaling is a uniform geometric scale — drive it with a material uniform (line width) or a child `Transform` scale rather than regenerating vertices. This eliminates per-frame mesh rebuilds entirely.
- **Quantize the radius** to discrete steps (e.g. powers of ~1.25) so a given zoom level maps to a stable mesh and you rebuild only when crossing a step, not continuously.
- If you keep rebuilds, **reuse the buffers**: `generate_*` allocates new `Vec`s every call; writing into the existing mesh's attribute vecs avoids the allocation per rebuild.

`update_occluder_scale` writes `occluder_transform.scale` every frame unconditionally — harmless (Transforms re-upload anyway) but you could guard it for consistency.

---

## 4. What's already good (don't touch)

- **Trajectory brightness** (`1e453b1`): moving the front/back lerp + exposure into shader uniforms so the mesh no longer rebuilds on brightness tweaks is exactly right.
- **Starfield uniform sync** (`update_starfield_brightness`): correctly diffs before mutating — no per-frame buffer re-upload.
- **Star lighting cache** (`star_cache.rs`): building the snapshot once per frame and sharing it across `update_body_points` / `update_wireframe_lighting` / `update_occluder_lighting` is the right call.
- **Kepler param caching** (`04eae38`) and material change-guards throughout.

---

## Suggested order of attack
1. **§1a** — hoist starfield per-star constants to a buffer (biggest win / effort ratio).
2. **§3** — stop regenerating fade meshes during zoom (scale or quantize).
3. **§1b** — re-architect starfield to instanced sprites (if profiling still shows it hot).
4. **§2** — trivial loop cleanup.

Before/after, profile the starfield pass specifically (e.g. a GPU timestamp around that draw) — §1 is almost certainly your frame-time sink, and you want the number to point at, not a guess.
