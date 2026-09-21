//! The shader's uniform and the host's struct are one layout, written twice.
//!
//! `ShaderType` lays a uniform out by declaration order, so a field inserted in one and appended
//! to the other shifts every value after it. Nothing catches that: the shader still compiles,
//! the sizes still match, and the only symptom is a rendering that looks merely wrong. This
//! caught exactly that while the corona's reach was being added.

/// Field names of a struct, in declaration order, from either language's source.
fn fields(source: &str, header: &str) -> Vec<String> {
    let body = source
        .split_once(header)
        .unwrap_or_else(|| panic!("no {header} in that source"))
        .1;
    let body = body.split_once("\n}").expect("an unterminated struct").0;
    body.lines()
        .map(str::trim)
        // Comments and attributes are not fields; a `//`-prefixed line may itself contain a
        // colon, which is why this comes before the split.
        .filter(|line| !line.starts_with("//") && !line.starts_with('#'))
        .filter_map(|line| line.split_once(':'))
        .map(|(name, _)| name.trim_start_matches("pub ").trim().to_owned())
        .collect()
}

#[test]
fn the_starfield_uniform_is_declared_in_one_order() {
    let shader = include_str!("../assets/shaders/starfield.wgsl");
    let host = include_str!("../../em-render/src/relativistic_starfield_material.rs");

    let in_shader = fields(shader, "struct StarfieldUniform {");
    let in_host = fields(host, "pub struct RelativisticStarfieldUniform {");

    assert!(in_shader.len() > 10, "the parse found almost nothing: {in_shader:?}");
    assert_eq!(
        in_shader, in_host,
        "the shader and the host disagree about the uniform's layout",
    );
}

/// The population material grew from six fields to eleven when the envelope became a volume,
/// and three of the new ones are geometry the march cannot do without. A shift here would
/// render a belt with a cloud's slab and no error anywhere.
#[test]
fn the_population_uniform_is_declared_in_one_order() {
    let shader = include_str!("../assets/shaders/population.wgsl");
    let host = include_str!("../../em-render/src/population_material.rs");

    let in_shader = fields(shader, "struct PopulationUniform {");
    let in_host = fields(host, "pub struct PopulationUniform {");

    assert!(in_shader.len() > 8, "the parse found almost nothing: {in_shader:?}");
    assert_eq!(
        in_shader, in_host,
        "the shader and the host disagree about the uniform's layout",
    );
}

/// The body surface uniform grew an emission term, and it is four floats at the end of a struct
/// the shader reads positionally.
#[test]
fn the_body_surface_uniform_is_declared_in_one_order() {
    let shader = include_str!("../assets/shaders/body_surface.wgsl");
    let host = include_str!("../../em-render/src/body_surface_material.rs");

    let in_shader = fields(shader, "struct BodySurfaceUniform {");
    let in_host = fields(host, "pub struct BodySurfaceUniform {");

    assert!(in_shader.len() >= 5, "the parse found almost nothing: {in_shader:?}");
    assert_eq!(
        in_shader, in_host,
        "the shader and the host disagree about the uniform's layout",
    );
}

/// The plume's uniform grew a second color and a churn, appended to both — but it is six vectors
/// the shader reads positionally, and a march that takes the soot for the exposure draws nothing
/// at all.
#[test]
fn the_plume_uniform_is_declared_in_one_order() {
    let shader = include_str!("../assets/shaders/plume.wgsl");
    let host = include_str!("../../em-render/src/plume_material.rs");

    let in_shader = fields(shader, "struct PlumeUniform {");
    let in_host = fields(host, "pub struct PlumeUniform {");

    assert!(in_shader.len() >= 6, "the parse found almost nothing: {in_shader:?}");
    assert_eq!(
        in_shader, in_host,
        "the shader and the host disagree about the uniform's layout",
    );
}
