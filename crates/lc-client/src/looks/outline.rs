//! A roof's outline seen from above, as a map of signed meters to its edge, so a habitat's shader
//! can stop a ray through the glass at a wall dropping from the rim, whatever the hull's shape.
//!
//! The roof is where the normal faces more up than sideways, as `aztec.wgsl` draws it from the
//! normal it interpolates, so each triangle is clipped where that changes sign between its
//! corners; its shadow straight down is the outline. A look's is taken once in world space, with Y
//! up, so the entity must not move after; a hull's in the ship's frame, with z up, where the roof is
//! also only where its region leads, so it follows the hull wherever it goes.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use em_render::hull_material::{ATTRIBUTE_HULL_REGIONS, HullMaterial, RoofOutline as HullRoofOutline};

use super::aztec::AztecMaterial;

/// Texels along the outline's longer side.
const TEXELS: usize = 256;

/// Empty texels around the outline, so the map's edge is never the roof's.
const MARGIN: usize = 4;

/// Derive this entity's roof outline from its mesh, once the mesh is there, and give it a copy of
/// its aztec material carrying it.
#[derive(Component)]
pub struct RoofOutline;

/// The mesh the outline was taken from; a new mesh is outlined again.
#[derive(Component)]
pub struct Outlined(AssetId<Mesh>);

/// In `PostUpdate`, after transforms propagate, so a mesh spawned this frame is where it will be.
pub fn outline_roofs(
    mut commands: Commands,
    roofs: Query<(Entity, &Mesh3d, &MeshMaterial3d<AztecMaterial>, &GlobalTransform, Option<&Outlined>), With<RoofOutline>>,
    meshes: Res<Assets<Mesh>>,
    mut materials: ResMut<Assets<AztecMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    for (entity, mesh, material, transform, outlined) in &roofs {
        if outlined.is_some_and(|o| o.0 == mesh.0.id()) {
            continue;
        }
        let (Some(shape), Some(base)) = (meshes.get(&mesh.0), materials.get(&material.0)) else { continue };
        let mut base = base.clone();
        let Some((image, bounds, width)) = outline(shape, transform) else {
            warn!("looks: a roof outline was asked of a mesh with no roof");
            commands.entity(entity).insert(Outlined(mesh.0.id()));
            continue;
        };
        base.set_outline(images.add(image), bounds, width);
        commands.entity(entity).insert((MeshMaterial3d(materials.add(base)), Outlined(mesh.0.id())));
    }
}

/// Derive this hull's habitat roof outline from its mesh, once the mesh is there, and give it a
/// copy of its hull material carrying it.
#[derive(Component)]
pub struct LivingRoof(pub u32);

/// In `PostUpdate`, so a mesh the mesher landed this frame is outlined before it is drawn.
pub fn outline_hull_roofs(
    mut commands: Commands,
    roofs: Query<(Entity, &LivingRoof, &Mesh3d, &MeshMaterial3d<HullMaterial>, Option<&Outlined>)>,
    meshes: Res<Assets<Mesh>>,
    mut materials: ResMut<Assets<HullMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    for (entity, roof, mesh, material, outlined) in &roofs {
        if outlined.is_some_and(|o| o.0 == mesh.0.id()) {
            continue;
        }
        let (Some(shape), Some(base)) = (meshes.get(&mesh.0), materials.get(&material.0)) else { continue };
        let mut base = base.clone();
        // A hull with no roof of that region still takes a copy, so a later remesh with one does
        // not write into a material another hull shares.
        let (map, outline) = match hull_roof(shape, roof.0) {
            Some((image, bounds, width)) => (Some(images.add(image)), HullRoofOutline { bounds, width: Vec4::new(width, 0.0, 0.0, 0.0) }),
            None => (None, HullRoofOutline::default()),
        };
        base.roof_map = map;
        base.roof_outline = outline;
        commands.entity(entity).insert((MeshMaterial3d(materials.add(base)), Outlined(mesh.0.id())));
    }
}

/// [`outline`] for a hull's habitat roof, in the ship's frame with z up: where the normal faces more
/// up than sideways and `region` leads the others.
pub fn hull_roof(mesh: &Mesh, region: u32) -> Option<(Image, Vec4, f32)> {
    let Some(VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { return None };
    let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else { return None };
    let attribute = &ATTRIBUTE_HULL_REGIONS[(region / 4) as usize];
    let Some(VertexAttributeValues::Unorm8x4(weights)) = mesh.attribute(attribute.id) else { return None };
    let at: Vec<Vec2> = positions.iter().map(|p| Vec2::new(p[0], p[1])).collect();
    let lead: Vec<f32> = normals
        .iter()
        .map(|n| {
            let n = Vec3::from(*n).normalize_or_zero();
            n.z - n.x.abs().max(n.y.abs())
        })
        .collect();
    // The region's weight over a half, as the shader draws whichever region is heavier.
    let ours: Vec<f32> = weights.iter().map(|w| w[(region % 4) as usize] as f32 / 255.0 - 0.5).collect();
    let roofed: Vec<[Vec2; 3]> = triangles(mesh, positions.len())
        .iter()
        .flat_map(|t| clipped_twice(t.map(|i| (at[i], lead[i], ours[i]))))
        .collect();
    rasterized(&roofed)
}

/// The part of a triangle where both of its corners' values are positive, each linear across it.
fn clipped_twice(corners: [(Vec2, f32, f32); 3]) -> Vec<[Vec2; 3]> {
    let mut polygon: Vec<(Vec2, f32, f32)> = corners.to_vec();
    for pick in [|c: &(Vec2, f32, f32)| c.1, |c: &(Vec2, f32, f32)| c.2] {
        let mut kept = Vec::with_capacity(polygon.len() + 1);
        for i in 0..polygon.len() {
            let a = polygon[i];
            let b = polygon[(i + 1) % polygon.len()];
            let (va, vb) = (pick(&a), pick(&b));
            if va > 0.0 {
                kept.push(a);
            }
            if (va > 0.0) != (vb > 0.0) {
                let s = va / (va - vb);
                kept.push((a.0.lerp(b.0, s), a.1 + (b.1 - a.1) * s, a.2 + (b.2 - a.2) * s));
            }
        }
        polygon = kept;
        if polygon.len() < 3 {
            return Vec::new();
        }
    }
    (1..polygon.len() - 1).map(|i| [polygon[0].0, polygon[i].0, polygon[i + 1].0]).collect()
}

fn triangles(mesh: &Mesh, vertices: usize) -> Vec<[usize; 3]> {
    match mesh.indices() {
        Some(Indices::U16(i)) => i.chunks_exact(3).map(|t| [t[0] as usize, t[1] as usize, t[2] as usize]).collect(),
        Some(Indices::U32(i)) => i.chunks_exact(3).map(|t| [t[0] as usize, t[1] as usize, t[2] as usize]).collect(),
        None => (0..vertices / 3).map(|t| [3 * t, 3 * t + 1, 3 * t + 2]).collect(),
    }
}

/// The map, R16Float signed meters, positive inside; the world XZ it spans as `(x0, z0, x1, z1)`;
/// and the widest circle it holds, meters across. `None` where no triangle is roof.
pub fn outline(mesh: &Mesh, transform: &GlobalTransform) -> Option<(Image, Vec4, f32)> {
    let Some(VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { return None };
    let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else { return None };
    let affine = transform.affine();
    let normal_matrix = affine.matrix3.inverse().transpose();
    let at: Vec<Vec2> = positions.iter().map(|p| affine.transform_point3(Vec3::from(*p)).xz()).collect();
    // Positive on the roof. Linear across a triangle only roughly, as the shader's own test is.
    let lead: Vec<f32> = normals
        .iter()
        .map(|n| {
            let n = (normal_matrix * Vec3::from(*n)).normalize_or_zero();
            n.y - n.x.abs().max(n.z.abs())
        })
        .collect();
    let roofed: Vec<[Vec2; 3]> = triangles(mesh, positions.len()).iter().flat_map(|t| clipped(t.map(|i| (at[i], lead[i])))).collect();
    rasterized(&roofed)
}

/// The signed-distance map of the union of `roofed`, as [`outline`] returns it.
fn rasterized(roofed: &[[Vec2; 3]]) -> Option<(Image, Vec4, f32)> {
    if roofed.is_empty() {
        return None;
    }

    let (mut lo, mut hi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
    for p in roofed.iter().flatten() {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let texel = (hi - lo).max_element() / (TEXELS - 2 * MARGIN) as f32;
    lo -= Vec2::splat(MARGIN as f32 * texel);
    let size = ((hi - lo) / texel).ceil().as_uvec2() + UVec2::splat(MARGIN as u32);
    let (w, h) = (size.x as usize, size.y as usize);
    let hi = lo + size.as_vec2() * texel;

    let mut inside = vec![false; w * h];
    for [a, b, c] in roofed {
        let (a, b, c) = ((*a - lo) / texel, (*b - lo) / texel, (*c - lo) / texel);
        let area = (b - a).perp_dot(c - a);
        if area.abs() < 1.0e-9 {
            continue;
        }
        let (min, max) = (a.min(b).min(c).floor().max(Vec2::ZERO), a.max(b).max(c).ceil().min(size.as_vec2() - 1.0));
        for y in min.y as usize..=max.y as usize {
            for x in min.x as usize..=max.x as usize {
                let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let (u, v) = ((p - a).perp_dot(c - a) / area, (b - a).perp_dot(p - a) / area);
                if u >= 0.0 && v >= 0.0 && u + v <= 1.0 {
                    inside[y * w + x] = true;
                }
            }
        }
    }

    let to_outside = distance(&inside, w, h, false);
    let to_inside = distance(&inside, w, h, true);
    // Half a texel off each, so the zero falls on the boundary between texels.
    let meters: Vec<f32> = (0..w * h).map(|i| texel * if inside[i] { to_outside[i] - 0.5 } else { 0.5 - to_inside[i] }).collect();
    let width = 2.0 * meters.iter().copied().fold(0.0, f32::max);
    let bytes: Vec<u8> = meters.iter().flat_map(|m| half::f16::from_f32(*m).to_le_bytes()).collect();
    let mut image = Image::new(
        Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        bytes,
        TextureFormat::R16Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    Some((image, Vec4::new(lo.x, lo.y, hi.x, hi.y), width))
}

/// The part of a triangle where `lead` is positive, as up to two triangles.
fn clipped(corners: [(Vec2, f32); 3]) -> Vec<[Vec2; 3]> {
    let mut kept = Vec::with_capacity(4);
    for i in 0..3 {
        let (a, la) = corners[i];
        let (b, lb) = corners[(i + 1) % 3];
        if la > 0.0 {
            kept.push(a);
        }
        if (la > 0.0) != (lb > 0.0) {
            kept.push(a.lerp(b, la / (la - lb)));
        }
    }
    (1..kept.len().saturating_sub(1)).map(|i| [kept[0], kept[i], kept[i + 1]]).collect()
}

/// Each texel's distance in texels to the nearest whose `inside` is `target`: Felzenszwalb and
/// Huttenlocher's exact transform, a column pass and a row pass. In f64, where "none yet" can be
/// far enough that `q²` is not lost against it.
fn distance(inside: &[bool], w: usize, h: usize, target: bool) -> Vec<f32> {
    const FAR: f64 = 1.0e10;
    let mut grid: Vec<f64> = inside.iter().map(|&i| if i == target { 0.0 } else { FAR }).collect();
    let mut line = Vec::new();
    for x in 0..w {
        line.clear();
        line.extend((0..h).map(|y| grid[y * w + x]));
        for (y, d) in squared_1d(&line).into_iter().enumerate() {
            grid[y * w + x] = d;
        }
    }
    for y in 0..h {
        let row = squared_1d(&grid[y * w..(y + 1) * w]);
        grid[y * w..(y + 1) * w].copy_from_slice(&row);
    }
    grid.into_iter().map(|d| d.sqrt() as f32).collect()
}

/// The lower envelope of parabolas rooted at each sample's height.
fn squared_1d(f: &[f64]) -> Vec<f64> {
    let n = f.len();
    let mut v = vec![0usize; n];
    let mut z = vec![0.0f64; n + 1];
    let mut k = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    let root = |q: usize| f[q] + (q * q) as f64;
    for q in 1..n {
        let mut s = (root(q) - root(v[k])) / (2.0 * (q - v[k]) as f64);
        // z[0] is minus infinity, so this stops at k = 0.
        while s <= z[k] {
            k -= 1;
            s = (root(q) - root(v[k])) / (2.0 * (q - v[k]) as f64);
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    let mut out = vec![0.0; n];
    k = 0;
    for (q, slot) in out.iter_mut().enumerate() {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        *slot = (q as f64 - v[k] as f64).powi(2) + f[v[k]];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances_are_exact_on_a_line() {
        let inside = [false, false, true, false, false, false];
        let d = distance(&inside, 6, 1, true);
        assert_eq!(d, vec![2.0, 1.0, 0.0, 1.0, 2.0, 3.0]);
    }

    #[test]
    fn a_unit_sphere_is_a_disc_from_above() {
        let mesh = Sphere::new(1.0).mesh().uv(64, 32);
        let (image, bounds, width) = outline(&mesh, &GlobalTransform::IDENTITY).expect("a sphere has a top");
        // The roof stops where the normal is 45 degrees from up: a disc of radius sin 45.
        let r = std::f32::consts::FRAC_1_SQRT_2;
        let (w, h) = (image.width() as usize, image.height() as usize);
        let value = |x: f32, z: f32| {
            let u = ((x - bounds.x) / (bounds.z - bounds.x) * w as f32) as usize;
            let v = ((z - bounds.y) / (bounds.w - bounds.y) * h as f32) as usize;
            let i = 2 * (v * w + u);
            let data = image.data.as_ref().expect("made on the CPU");
            half::f16::from_le_bytes([data[i], data[i + 1]]).to_f32()
        };
        assert!((value(0.0, 0.0) - r).abs() < 0.03, "center: {}", value(0.0, 0.0));
        assert!(value(r - 0.05, 0.0).abs() < 0.05);
        assert!(value(r + 0.02, 0.0) < 0.0);
        assert!((width - 2.0 * r).abs() < 0.06, "width: {width}");
    }
}
