// The sprue's tiles as a hull reads them: which of its sixteen edge-matched variants a tile cell
// is, by hashing each of its edges, so the light and the pipes are continuous across a seam
// whatever a neighbor picks. Shared by hull.wgsl; sprue.wgsl is its development copy.

/// Where a face starts its tiles, in tiles, so neighbors do not repeat edge to edge.
fn tile_start(id: f32) -> vec2<f32> {
    var p3 = fract(vec3<f32>(id) * vec3<f32>(0.1031, 0.1030, 0.0973));
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.xx + p3.yz) * p3.zy);
}

/// Whether the edge of tile cells at `at` on `axis` (0 vertical, 1 horizontal) takes its second
/// choice.
fn tile_edge(at: vec2<f32>, axis: f32, id: f32) -> u32 {
    var p3 = fract(vec3<f32>(at, id * 7.0 + axis) * vec3<f32>(0.1031, 0.1030, 0.0973));
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return u32(fract((p3.x + p3.y) * p3.z) < 0.5);
}

/// The array layer of layer `k` of the variant under tile coordinate `uv`: sixteen variants of
/// three layers, as looks::sprue::VARIANTS.
fn variant_layer(uv: vec2<f32>, k: u32, id: f32) -> u32 {
    let c = floor(uv);
    let variant = tile_edge(c, 0.0, id)
        + 2u * tile_edge(c + vec2<f32>(1.0, 0.0), 0.0, id)
        + 4u * tile_edge(c, 1.0, id)
        + 8u * tile_edge(c + vec2<f32>(0.0, 1.0), 1.0, id);
    return variant * 3u + k;
}
