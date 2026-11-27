//! The selected feature's fields, or an unread line as text to fix.

use bevy_egui::egui::{self, DragValue, RichText};
use lc_galaxy::compiled::{Age, Dispersion, Feh, Imf, OpSpec, Rotation, SystemField};
use lc_galaxy::kinds::{self, BoolOp, Boolean, Contribution, Field, FrameKind, PathPoint, PopulationKind, ShapeKind, ShapeRef, Unit, Variable};
use lc_galaxy::{Compiled, FeatureId, Num, Output, Params, Row, Target, Timeline, Unread, codec};

use crate::doc::Doc;
use crate::palette;

pub fn show(ui: &mut egui::Ui, doc: &mut Doc) {
    ui.heading("Parameters");
    ui.separator();
    let Some(index) = doc.selected.filter(|&i| i < doc.timeline.len()) else { return };
    let refs = Refs::before(&doc.timeline, index, doc.result.as_ref().map(|r| &r.compiled));
    let problem = doc.timeline.rows()[index]
        .id()
        .and_then(|id| doc.result.as_ref()?.compiled.problems.get(&id).cloned());

    let mut reread = None;
    match &doc.timeline.rows()[index] {
        Row::Unread(u) => unread(ui, u, &mut reread),
        Row::Feature(_) => {
            let id = doc.timeline.rows()[index].id().unwrap();
            let f = doc.timeline.feature_mut(id).unwrap();
            egui::Grid::new("feature head").num_columns(2).show(ui, |ui| {
                ui.label("name");
                let mut name = f.name.clone().unwrap_or_default();
                if ui.text_edit_singleline(&mut name).changed() {
                    f.name = (!name.trim().is_empty()).then_some(name);
                }
                ui.end_row();
                ui.label("kind");
                ui.label(format!("{} v{}  {}", f.params.kind(), f.params.version(), f.id));
                ui.end_row();
                ui.label("enabled");
                ui.checkbox(&mut f.enabled, "");
                ui.end_row();
            });
            if let Some(problem) = &problem {
                ui.label(RichText::new(problem.to_string()).color(palette::PROBLEM));
            }
            ui.separator();
            kind_fields(ui, f.params.as_mut(), &refs);
        }
    }
    if let Some(line) = reread {
        let row = codec::read_row(&line, &doc.registry);
        if let Err(e) = doc.timeline.replace(index, row) {
            doc.notice = Some(e);
        }
    }
}

fn unread(ui: &mut egui::Ui, u: &Unread, reread: &mut Option<String>) {
    match u {
        Unread::Unknown { kind, version, .. } => {
            ui.label(format!("This build has no kind {kind:?} at version {version}."));
        }
        Unread::Unsupported { kind, fields, .. } => {
            ui.label(format!("{kind} fields this build does not have:"));
            for f in fields {
                ui.label(RichText::new(f).monospace().color(palette::UNREAD));
            }
        }
        Unread::Damaged { error, .. } => {
            ui.label(RichText::new(error).color(palette::PROBLEM));
        }
    }
    let id = egui::Id::new(("unread line", u.line()));
    let mut text = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| u.line().to_string());
    ui.add(egui::TextEdit::multiline(&mut text).code_editor().desired_width(f32::INFINITY).desired_rows(6));
    let changed = text != u.line();
    ui.data_mut(|d| d.insert_temp(id, text.clone()));
    if ui.add_enabled(changed, egui::Button::new("Read again")).clicked() {
        *reread = Some(text);
    }
}

/// The features before the one being edited, which are all it may reference.
struct Refs {
    frames: Vec<(FeatureId, String)>,
    populations: Vec<(FeatureId, String)>,
    variables: Vec<String>,
    /// Every output of an earlier feature that compiled to a shape.
    shapes: Vec<(Output, String)>,
}

impl Refs {
    fn before(t: &Timeline, index: usize, compiled: Option<&Compiled>) -> Self {
        let mut r = Refs { frames: Vec::new(), populations: Vec::new(), variables: Vec::new(), shapes: Vec::new() };
        for f in t.rows()[..index].iter().filter_map(Row::feature) {
            for o in compiled.into_iter().flat_map(|c| c.shapes.keys()).filter(|o| o.feature == f.id) {
                let name = if o.of == 1 { f.label() } else { format!("{} {} of {}", f.label(), o.index + 1, o.of) };
                r.shapes.push((*o, name));
            }
            match f.params.kind() {
                "frame" => r.frames.push((f.id, f.label())),
                "population" => r.populations.push((f.id, f.label())),
                _ => {}
            }
            if let Some(v) = f.get::<Variable>() {
                r.variables.push(v.name.clone());
            }
        }
        r
    }
}

/// Points a new feature's references at the nearest frame, population and shapes before `at`.
pub fn seed_references(params: &mut dyn Params, t: &Timeline, at: usize, compiled: Option<&Compiled>) {
    let refs = Refs::before(t, at.min(t.len()), compiled);
    let frame = refs.frames.last().map(|f| f.0);
    let pop = refs.populations.last().map(|p| p.0);
    if let Some(p) = params.as_any_mut().downcast_mut::<PopulationKind>() {
        p.frame = frame.unwrap_or(p.frame);
    } else if let Some(b) = params.as_any_mut().downcast_mut::<Boolean>() {
        b.of = refs.shapes.iter().rev().take(2).rev().map(|s| ShapeRef(s.0)).collect();
    } else if let Some(s) = kinds::as_shape(params) {
        *s.frame_mut() = frame.unwrap_or(*s.frame_mut());
        if let Some(pop) = pop {
            *s.contributes_mut() = vec![Contribution(Target::Density(pop), OpSpec::Add(Num::Lit(10.0)))];
        }
    }
}

fn kind_fields(ui: &mut egui::Ui, params: &mut dyn Params, refs: &Refs) {
    let kind = params.kind();
    let any = params.as_any_mut();
    if let Some(v) = any.downcast_mut::<Variable>() {
        variable(ui, v);
    } else if let Some(f) = any.downcast_mut::<FrameKind>() {
        frame(ui, f);
    } else if let Some(p) = any.downcast_mut::<PopulationKind>() {
        population(ui, p, refs);
    } else if let Some(b) = any.downcast_mut::<Boolean>() {
        boolean(ui, b, refs);
    } else if let Some(s) = kinds::as_shape(params) {
        shape(ui, kind, s, refs);
    }
}

fn boolean(ui: &mut egui::Ui, b: &mut Boolean, refs: &Refs) {
    grid(ui, "boolean", |ui| {
        ui.label("operation");
        egui::ComboBox::from_id_salt("bool op").selected_text(op_name(b.op)).show_ui(ui, |ui| {
            for op in [BoolOp::Union, BoolOp::Intersect, BoolOp::Difference] {
                ui.selectable_value(&mut b.op, op, op_name(op));
            }
        });
        ui.end_row();
        ui.label("blend");
        num(ui, "blend", &mut b.blend_ly, &refs.variables, " ly");
        ui.end_row();
    });
    ui.label(RichText::new("of").strong());
    let name = |o: Output| refs.shapes.iter().find(|s| s.0 == o).map_or_else(|| format!("{} (not before this)", o.feature), |s| s.1.clone());
    let mut remove = None;
    for (i, r) in b.of.iter_mut().enumerate() {
        ui.push_id(("operand", i), |ui| {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("operand").selected_text(name(r.0)).show_ui(ui, |ui| {
                    for (o, n) in &refs.shapes {
                        ui.selectable_value(r, ShapeRef(*o), n);
                    }
                });
                if ui.small_button("remove").clicked() {
                    remove = Some(i);
                }
            });
        });
    }
    if let Some(i) = remove {
        b.of.remove(i);
    }
    if let Some((o, _)) = refs.shapes.last()
        && ui.button("+").clicked()
    {
        b.of.push(ShapeRef(*o));
    }
    ui.separator();
    contributions(ui, &mut b.contributes, refs);
}

fn op_name(op: BoolOp) -> &'static str {
    match op {
        BoolOp::Union => "union",
        BoolOp::Intersect => "intersection",
        BoolOp::Difference => "difference",
    }
}

fn grid(ui: &mut egui::Ui, id: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id).num_columns(2).spacing([10.0, 6.0]).show(ui, body);
}

fn number(ui: &mut egui::Ui, label: &str, value: &mut f64, speed: f64, unit: &str) {
    ui.label(label);
    ui.add(DragValue::new(value).speed(speed).suffix(unit));
    ui.end_row();
}

fn vec3(ui: &mut egui::Ui, label: &str, v: &mut (f64, f64, f64), unit: &str) {
    ui.label(label);
    ui.horizontal(|ui| {
        for c in [&mut v.0, &mut v.1, &mut v.2] {
            ui.add(DragValue::new(c).speed(0.05).suffix(unit));
        }
    });
    ui.end_row();
}

fn reference(ui: &mut egui::Ui, label: &str, id: &mut FeatureId, choices: &[(FeatureId, String)]) {
    ui.label(label);
    let current = choices.iter().find(|c| c.0 == *id).map_or_else(|| format!("{id} (not before this)"), |c| c.1.clone());
    egui::ComboBox::from_id_salt(label).selected_text(current).show_ui(ui, |ui| {
        for (cid, name) in choices {
            ui.selectable_value(id, *cid, name);
        }
    });
    ui.end_row();
}

/// A literal, or one of the variables defined before this feature.
fn num(ui: &mut egui::Ui, salt: &str, n: &mut Num, variables: &[String], unit: &str) {
    ui.horizontal(|ui| {
        let shown = match n {
            Num::Lit(_) => "value".to_string(),
            Num::Var(name) => name.clone(),
        };
        egui::ComboBox::from_id_salt(salt).selected_text(shown).width(70.0).show_ui(ui, |ui| {
            if ui.selectable_label(matches!(n, Num::Lit(_)), "value").clicked() && !matches!(n, Num::Lit(_)) {
                *n = Num::Lit(1.0);
            }
            for v in variables {
                if ui.selectable_label(n.variable() == Some(v), v).clicked() {
                    *n = Num::Var(v.clone());
                }
            }
        });
        if let Num::Lit(v) = n {
            ui.add(DragValue::new(v).speed(0.05).suffix(unit));
        }
    });
}

fn variable(ui: &mut egui::Ui, v: &mut Variable) {
    grid(ui, "variable", |ui| {
        ui.label("name");
        ui.text_edit_singleline(&mut v.name);
        ui.end_row();
        number(ui, "value", &mut v.value, 0.1, "");
    });
}

fn frame(ui: &mut egui::Ui, f: &mut FrameKind) {
    grid(ui, "frame", |ui| {
        vec3(ui, "origin", &mut f.origin_ly, " ly");
        number(ui, "tilt", &mut f.tilt_deg, 0.5, "°");
        number(ui, "azimuth", &mut f.azimuth_deg, 0.5, "°");
        ui.label("rotation");
        let flat = matches!(f.rotation, Rotation::Flat { .. });
        egui::ComboBox::from_id_salt("rotation").selected_text(if flat { "flat curve" } else { "static" }).show_ui(ui, |ui| {
            if ui.selectable_label(!flat, "static").clicked() {
                f.rotation = Rotation::Static;
            }
            if ui.selectable_label(flat, "flat curve").clicked() && !flat {
                f.rotation = Rotation::Flat { speed_km_s: 220.0, core_ly: 3000.0 };
            }
        });
        ui.end_row();
        if let Rotation::Flat { speed_km_s, core_ly } = &mut f.rotation {
            number(ui, "speed", speed_km_s, 1.0, " km/s");
            number(ui, "core", core_ly, 10.0, " ly");
        }
    });
}

fn population(ui: &mut egui::Ui, p: &mut PopulationKind, refs: &Refs) {
    grid(ui, "population", |ui| {
        reference(ui, "frame", &mut p.frame, &refs.frames);
        ui.label("IMF");
        egui::ComboBox::from_id_salt("imf").selected_text(format!("{:?}", p.imf)).show_ui(ui, |ui| {
            ui.selectable_value(&mut p.imf, Imf::Kroupa, "Kroupa");
            ui.selectable_value(&mut p.imf, Imf::Salpeter, "Salpeter");
        });
        ui.end_row();
        ui.label("age");
        let burst = matches!(p.age, Age::Burst { .. });
        egui::ComboBox::from_id_salt("age").selected_text(if burst { "burst" } else { "uniform" }).show_ui(ui, |ui| {
            if ui.selectable_label(!burst, "uniform").clicked() && burst {
                p.age = Age::Uniform { min_gyr: 0.0, max_gyr: 10.0 };
            }
            if ui.selectable_label(burst, "burst").clicked() && !burst {
                p.age = Age::Burst { gyr: 1.0, spread_gyr: 0.1 };
            }
        });
        ui.end_row();
        match &mut p.age {
            Age::Uniform { min_gyr, max_gyr } => {
                number(ui, "youngest", min_gyr, 0.05, " Gyr");
                number(ui, "oldest", max_gyr, 0.05, " Gyr");
            }
            Age::Burst { gyr, spread_gyr } => {
                number(ui, "at", gyr, 0.05, " Gyr");
                number(ui, "spread", spread_gyr, 0.01, " Gyr");
            }
        }
        let Feh { zero_age, per_gyr, per_kly, scatter } = &mut p.feh;
        number(ui, "[Fe/H] at birth", zero_age, 0.01, " dex");
        number(ui, "[Fe/H] per Gyr", per_gyr, 0.005, " dex");
        number(ui, "[Fe/H] per kly", per_kly, 0.002, " dex");
        number(ui, "[Fe/H] scatter", scatter, 0.005, " dex");
        let Dispersion { zero_age_km_s, exponent } = &mut p.dispersion;
        number(ui, "σ at birth", zero_age_km_s, 0.1, " km/s");
        number(ui, "σ exponent", exponent, 0.01, "");
    });
}

fn shape(ui: &mut egui::Ui, kind: &str, s: &mut dyn ShapeKind, refs: &Refs) {
    grid(ui, kind, |ui| {
        reference(ui, "frame", s.frame_mut(), &refs.frames);
        for field in s.fields() {
            match field {
                Field::Num(label, n, unit) => {
                    ui.label(label);
                    let suffix = match unit {
                        Unit::Ly => " ly",
                        Unit::Deg => "°",
                        Unit::Plain => "",
                    };
                    num(ui, label, n, &refs.variables, suffix);
                    ui.end_row();
                }
                Field::Vec3(label, v) => vec3(ui, label, v, " ly"),
                Field::Count(label, c) => {
                    ui.label(label);
                    ui.add(DragValue::new(c).range(1..=kinds::MAX_ARMS).clamp_existing_to_range(false));
                    ui.end_row();
                }
            }
        }
    });
    if let Some(points) = s.points_mut() {
        ui.separator();
        path_points(ui, points);
    }
    ui.separator();
    contributions(ui, s.contributes_mut(), refs);
}

fn path_points(ui: &mut egui::Ui, points: &mut Vec<PathPoint>) {
    ui.label(RichText::new("points").strong());
    let mut insert = None;
    let mut remove = None;
    egui::Grid::new("path points").num_columns(3).spacing([6.0, 4.0]).show(ui, |ui| {
        for (i, p) in points.iter_mut().enumerate() {
            ui.label(format!("{}", i + 1));
            ui.horizontal(|ui| {
                for c in [&mut p.at_ly.0, &mut p.at_ly.1, &mut p.at_ly.2] {
                    ui.add(DragValue::new(c).speed(0.1).suffix(" ly"));
                }
                ui.label("r");
                ui.add(DragValue::new(&mut p.radius_ly).speed(0.05).suffix(" ly"));
            });
            ui.horizontal(|ui| {
                if ui.small_button("insert").clicked() {
                    insert = Some(i);
                }
                if ui.small_button("remove").clicked() {
                    remove = Some(i);
                }
            });
            ui.end_row();
        }
    });
    if let Some(i) = insert {
        // Halfway to the next point; after the last, one step on along the last span.
        let (p, at) = (points[i], glam::DVec3::from(points[i].at_ly));
        let point = match (points.get(i + 1), i.checked_sub(1).map(|k| points[k])) {
            (Some(n), _) => PathPoint { at_ly: ((at + glam::DVec3::from(n.at_ly)) * 0.5).into(), radius_ly: 0.5 * (p.radius_ly + n.radius_ly) },
            (None, Some(b)) => PathPoint { at_ly: (at * 2.0 - glam::DVec3::from(b.at_ly)).into(), ..p },
            (None, None) => PathPoint { at_ly: (at + glam::DVec3::X * 5.0).into(), ..p },
        };
        points.insert(i + 1, point);
    }
    if let Some(i) = remove {
        points.remove(i);
    }
}

fn contributions(ui: &mut egui::Ui, list: &mut Vec<Contribution>, refs: &Refs) {
    ui.label(RichText::new("contributes").strong());
    let mut remove = None;
    for (i, Contribution(target, op)) in list.iter_mut().enumerate() {
        ui.push_id(i, |ui| {
            ui.horizontal(|ui| {
                target_picker(ui, target, refs);
                op_picker(ui, op);
                let unit = match (&target, &op) {
                    (Target::Density(_), OpSpec::Add(_)) => " stars",
                    (Target::FeH(_), OpSpec::Add(_) | OpSpec::Set(_)) => " dex",
                    (Target::System(f), OpSpec::Add(_) | OpSpec::Set(_)) if f.in_dex() => " dex",
                    _ => "",
                };
                match op {
                    OpSpec::Add(n) | OpSpec::Scale(n) | OpSpec::Set(n) => num(ui, "amount", n, &refs.variables, unit),
                    OpSpec::Subtract => {}
                }
                if ui.small_button("remove").clicked() {
                    remove = Some(i);
                }
            });
        });
    }
    if let Some(i) = remove {
        list.remove(i);
    }
    if ui.button("+").clicked() {
        let target = refs.populations.last().map_or(Target::Exclusion, |p| Target::Density(p.0));
        list.push(Contribution(target, OpSpec::Add(Num::Lit(10.0))));
    }
}

fn target_picker(ui: &mut egui::Ui, target: &mut Target, refs: &Refs) {
    let name = |id: FeatureId| refs.populations.iter().find(|p| p.0 == id).map_or(format!("{id}"), |p| p.1.clone());
    let shown = match target {
        Target::Density(p) => format!("density of {}", name(*p)),
        Target::FeH(p) => format!("[Fe/H] of {}", name(*p)),
        Target::Exclusion => "packing".into(),
        Target::System(f) => format!("system: {}", f.label()),
    };
    egui::ComboBox::from_id_salt("target").selected_text(shown).show_ui(ui, |ui| {
        for (id, n) in &refs.populations {
            ui.selectable_value(target, Target::Density(*id), format!("density of {n}"));
            ui.selectable_value(target, Target::FeH(*id), format!("[Fe/H] of {n}"));
        }
        ui.selectable_value(target, Target::Exclusion, "packing");
        for f in SystemField::ALL {
            ui.selectable_value(target, Target::System(f), format!("system: {}", f.label()));
        }
    });
}

fn op_picker(ui: &mut egui::Ui, op: &mut OpSpec) {
    let label = match op {
        OpSpec::Add(_) => "add",
        OpSpec::Scale(_) => "scale",
        OpSpec::Subtract => "subtract",
        OpSpec::Set(_) => "set",
    };
    let amount = match op {
        OpSpec::Add(n) | OpSpec::Scale(n) | OpSpec::Set(n) => n.clone(),
        OpSpec::Subtract => Num::Lit(1.0),
    };
    egui::ComboBox::from_id_salt("op").selected_text(label).width(70.0).show_ui(ui, |ui| {
        if ui.selectable_label(label == "add", "add").clicked() {
            *op = OpSpec::Add(amount.clone());
        }
        if ui.selectable_label(label == "scale", "scale").clicked() {
            *op = OpSpec::Scale(amount.clone());
        }
        if ui.selectable_label(label == "subtract", "subtract").clicked() {
            *op = OpSpec::Subtract;
        }
        if ui.selectable_label(label == "set", "set").clicked() {
            *op = OpSpec::Set(amount);
        }
    });
}
