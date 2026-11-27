//! Pictures of the Phase III draft: the light a volume holds is the light the graph's stars
//! shine with, a picture catches all of it, and threads change nothing.

use lc_galaxy::light::mean_light;
use lc_galaxy::picture::{Progress, Side, View, Volume, lights, march, march_exact};
use lc_galaxy::{Compiled, Registry, codec, stats};

fn phase_three() -> Compiled {
    let t = codec::read(include_str!("../galaxies/phase-3.ron"), &Registry::standard()).unwrap();
    t.compile(t.len(), None)
}

fn close(a: [f64; 3], b: [f64; 3], tolerance: f64) -> bool {
    a.iter().zip(&b).all(|(x, y)| (x / y - 1.0).abs() < tolerance)
}

#[test]
fn a_volume_holds_the_light_of_the_stars_the_graph_expects() {
    let c = phase_three();
    let volume = Volume::of(&c, 64, 4);
    let mut want = [0.0; 3];
    for ((id, pop), (_, n)) in c.populations.iter().zip(stats::expected(&c, c.bounds(), 100)) {
        let l = mean_light(pop, u64::from(id.0));
        for k in 0..3 {
            want[k] += n * l[k];
        }
    }
    let got = volume.total();
    assert!(close(got, want, 0.05), "volume {got:?}, stars {want:?}");
}

#[test]
fn a_picture_from_any_side_catches_all_the_light() {
    let volume = Volume::of(&phase_three(), 48, 4);
    let total = volume.total();
    for side in Side::ALL {
        let view = View::fitting(side, volume.bounds(), 160, 120);
        let got = march(&volume, &view, 160, 120, 4).total(view.ly_per_px);
        assert!(close(got, total, 0.04), "{}: picture {got:?}, volume {total:?}", side.label());
    }
}

#[test]
fn threads_change_nothing() {
    let c = phase_three();
    let (one, many) = (Volume::of(&c, 32, 1), Volume::of(&c, 32, 7));
    assert_eq!(one.emission, many.emission);
    let view = View::fitting(Side::ALL[2], one.bounds(), 64, 48);
    assert_eq!(march(&one, &view, 64, 48, 1).pixels, march(&many, &view, 64, 48, 5).pixels);
}


/// Marching the graph itself catches the same light as marching its volume.
#[test]
fn an_exact_picture_catches_the_light_a_volume_picture_does() {
    let c = phase_three();
    let volume = Volume::of(&c, 64, 4);
    let view = View::fitting(Side::ALL[2], volume.bounds(), 120, 120);
    let from_volume = march(&volume, &view, 120, 120, 4).total(view.ly_per_px);
    let exact = march_exact(&c, &lights(&c), &view, 120, 120, volume.voxel_ly, 4, &Progress::default()).unwrap().total(view.ly_per_px);
    assert!(close(exact, from_volume, 0.05), "exact {exact:?}, volume {from_volume:?}");
}
