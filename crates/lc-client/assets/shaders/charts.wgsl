// A part's chart (crate::charts in lc-client) as a look with a roof and decks reads it. A round
// part lying on its side has no end facing up, so its band, marked 2 in the chart's `w`, is split
// here into a roof along its top, down to `ROOF_ELEVATION` above level, and a side down each
// flank, each a rectangle whose v runs up, so decks stand level. Any other chart is as it is.
// Shared by hull.wgsl, aztec.wgsl and racks.wgsl.

/// Where a lying part's roof stops, as an ovoid's does: crate::charts::BAND_LATITUDE, which a test
/// there holds this to.
const ROOF_ELEVATION: f32 = 0.34906585;

struct ChartView {
    /// Meters in the chart as seen.
    p: vec2<f32>,
    kind: u32,
    bounds: vec4<f32>,
    /// u, v, and the surface's normal.
    frame: mat3x3<f32>,
    end: bool,
}

/// `chart`, `bounds` and `frame` as the vertex carries them, seen as a look with a roof wants.
fn chart_view(chart: vec4<f32>, bounds: vec4<f32>, frame: mat3x3<f32>) -> ChartView {
    let kind = u32(round(chart.z));
    if (chart.w < 1.5) {
        return ChartView(chart.xy, kind, bounds, frame, chart.w > 0.5);
    }
    let period = bounds.x;
    let roof = (1.5707963 - ROOF_ELEVATION) * period / 6.2831853;
    // The band's u is 0 along the part's top; taken into a half turn either way.
    let u = chart.x - period * round(chart.x / period);
    if (abs(u) < roof) {
        return ChartView(vec2<f32>(u, chart.y), 0u, vec4<f32>(-roof, roof, bounds.z, bounds.w), frame, true);
    }
    // A flank: v up it, from the keel to the roof's edge, and u along the part, the same way on
    // both flanks so they meet at the keel without a seam. The far flank's chart is mirrored, which
    // nothing here minds: a look reads u and v along `frame`'s first two columns.
    let up = select(frame[0], -frame[0], u > 0.0);
    let turned = mat3x3<f32>(frame[1], up, frame[2]);
    return ChartView(vec2<f32>(chart.y, 0.5 * period - abs(u)), 0u, vec4<f32>(bounds.zw, 0.0, 0.5 * period - roof), turned, false);
}
