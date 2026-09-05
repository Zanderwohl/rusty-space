#define_import_path exotic_matters::soi_shape

// The shape of a sphere of influence, shared by the limb ring and the point cloud.
//
// The CPU collapses every model in `em_sim::influence::SoiModel` to this parameter set,
// so all but the genuinely anisotropic ones arrive pre-evaluated as a single radius. That
// is what keeps `soi_radius` one branch instead of eight: adding an isotropic model needs
// no change here at all.
//
// Radii are in RENDER units (metres times the view's distance factor), and `primary_dir`
// is in render space (Y-up), converted on the CPU through `render_space::ToRender`.

struct SoiShape {
    // --- 16-byte block 0 ---
    primary_dir: vec4<f32>,   // xyz: unit body -> primary; w unused
    // --- block 1 ---
    model: u32,               // matches SoiModel's declaration order; see MODEL_* below
    radius: f32,              // the widest radius: what an isotropic model returns outright
    bounding_radius: f32,
    min_radius: f32,
    // --- block 2 ---
    anisotropic: f32,         // > 0.5 -> the limb needs the iterative solve
    fade: f32,                // screen-size LOD fade, 0..1
    // No explicit tail padding: naga_oil rejects identifiers a composable module would
    // have to rewrite, and both encase and naga round this struct up to its 16-byte
    // alignment on their own.
}

const MODEL_LAPLACE_ANGLED: u32 = 5u;

/// Radius toward `dir` (a unit vector from the body's centre), in render units.
///
/// Mirrors `Soi::radius_at_cos` on the CPU. The two must agree: the CPU decides
/// visibility and LOD from it, the GPU draws from it.
fn soi_radius(shape: SoiShape, dir: vec3<f32>) -> f32 {
    if (shape.model == MODEL_LAPLACE_ANGLED) {
        // A surface of revolution about the body-primary axis: narrowest along it.
        let c = dot(normalize(dir), shape.primary_dir.xyz);
        return shape.radius / pow(1.0 + 3.0 * c * c, 0.1);
    }
    return shape.radius;
}
