use eframe::egui;
use eframe::egui::Ui;
use crate::interop::horizons::*;

const MAX_SEARCH_RESULTS: usize = 2000;

/// Opaque status the bin passes in; we just need to know loading/ready/failed.
pub enum BodyListStatus {
    Loading,
    Ready,
    Failed(String),
}

pub fn request_ui(
    ui: &mut Ui,
    request: &mut Request,
    body_list: &[MajorBody],
    body_list_status: &BodyListStatus,
    body_search: &mut String,
) {
    egui::ScrollArea::vertical()
        .id_salt("horizons_request")
        .show(ui, |ui| {
            ui.heading("Request");
            command_ui(ui, &mut request.command, body_list, body_list_status, body_search);
            ui.checkbox(&mut request.obj_data, "Include Object Data");

            ui.separator();
            ephemeris_ui(ui, &mut request.ephemeris);

            let warnings = request.validate();
            if !warnings.is_empty() {
                ui.separator();
                for w in &warnings {
                    ui.colored_label(egui::Color32::from_rgb(220, 160, 40), w);
                }
            }
        });
}

// ---------------------------------------------------------------------------
// Target / Command selector with search
// ---------------------------------------------------------------------------

fn command_ui(
    ui: &mut Ui,
    command: &mut Command,
    body_list: &[MajorBody],
    body_list_status: &BodyListStatus,
    body_search: &mut String,
) {
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("command_type")
            .selected_text(command.label())
            .show_ui(ui, |ui| {
                if ui.selectable_label(matches!(command, Command::MajorBody(_)), "Major Body ID").clicked() {
                    *command = Command::MajorBody(399);
                }
                if ui.selectable_label(matches!(command, Command::SmallBodyNumber(_)), "Small Body IAU#").clicked() {
                    *command = Command::SmallBodyNumber(1);
                }
                if ui.selectable_label(matches!(command, Command::SmallBodyDesignation(_)), "Small Body Designation").clicked() {
                    *command = Command::SmallBodyDesignation("1999 AN10".into());
                }
                if ui.selectable_label(matches!(command, Command::SmallBodyName(_)), "Small Body Name").clicked() {
                    *command = Command::SmallBodyName("Ceres".into());
                }
            });

        match command {
            Command::MajorBody(id) => {
                ui.add(egui::DragValue::new(id));
                if let Some(body) = body_list.iter().find(|b| b.id == *id) {
                    let mut hint = body.name.clone();
                    if !body.designation.is_empty() {
                        hint.push_str("  [");
                        hint.push_str(&body.designation);
                        hint.push(']');
                    }
                    ui.label(&hint);
                }
            }
            Command::SmallBodyNumber(n) => { ui.add(egui::DragValue::new(n)); }
            Command::SmallBodyDesignation(s) => { ui.text_edit_singleline(s); }
            Command::SmallBodyName(s) => { ui.text_edit_singleline(s); }
            Command::MajorBodyList => {}
        }
    });

    // Body search widget (only shown for MajorBody)
    if matches!(command, Command::MajorBody(_)) {
        body_search_ui(ui, command, body_list, body_list_status, body_search);
    }
}

fn body_search_ui(
    ui: &mut Ui,
    command: &mut Command,
    body_list: &[MajorBody],
    body_list_status: &BodyListStatus,
    search: &mut String,
) {
    match body_list_status {
        BodyListStatus::Loading => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Loading body list...");
            });
            return;
        }
        BodyListStatus::Failed(err) => {
            ui.colored_label(
                egui::Color32::from_rgb(220, 60, 60),
                format!("Body list failed: {}", err),
            );
            return;
        }
        BodyListStatus::Ready => {}
    }

    ui.horizontal(|ui| {
        ui.label("Search:");
        ui.text_edit_singleline(search);
    });

    let filtered: Vec<&MajorBody> = body_list
        .iter()
        .filter(|b| b.matches(search))
        .take(MAX_SEARCH_RESULTS)
        .collect();

    egui::ScrollArea::vertical()
        .id_salt("body_search_results")
        .max_height(200.0)
        .show(ui, |ui| {
            if filtered.is_empty() {
                ui.label("No matches.");
            }
            for body in &filtered {
                let selected = matches!(command, Command::MajorBody(id) if *id == body.id);
                let label = body.display_label();
                if ui.selectable_label(selected, &label).clicked() {
                    *command = Command::MajorBody(body.id);
                    search.clear();
                }
            }
        });
}

// ---------------------------------------------------------------------------
// Ephemeris UI
// ---------------------------------------------------------------------------

fn ephemeris_ui(ui: &mut Ui, ephemeris: &mut Option<EphemerisRequest>) {
    let mut make_ephem = ephemeris.is_some();
    ui.checkbox(&mut make_ephem, "Make Ephemeris");

    if make_ephem && ephemeris.is_none() {
        *ephemeris = Some(EphemerisRequest::default());
    } else if !make_ephem {
        *ephemeris = None;
    }

    if let Some(eph) = ephemeris {
        let mut tag = eph.tag();
        ui.horizontal(|ui| {
            egui::ComboBox::from_label("Ephemeris Type")
                .selected_text(tag.label())
                .show_ui(ui, |ui| {
                    for t in EphemerisTypeTag::ALL {
                        ui.selectable_value(&mut tag, t, t.label());
                    }
                });
        });
        eph.switch_to(tag);

        ui.separator();

        match eph {
            EphemerisRequest::Observer(p) => observer_ui(ui, p),
            EphemerisRequest::Vectors(p) => vectors_ui(ui, p),
            EphemerisRequest::Elements(p) => elements_ui(ui, p),
            EphemerisRequest::CloseApproach(p) => close_approach_ui(ui, p),
            EphemerisRequest::Spk(p) => spk_ui(ui, p),
        }
    }
}

// ---------------------------------------------------------------------------
// Observer
// ---------------------------------------------------------------------------

fn observer_ui(ui: &mut Ui, p: &mut ObserverParams) {
    ui.push_id("observer", |ui| {
        center_ui(ui, &mut p.center);
        time_config_ui(ui, &mut p.time);

        ui.horizontal(|ui| {
            ui.label("Quantities:");
            ui.text_edit_singleline(&mut p.quantities);
        });

        ui.collapsing("Output Options", |ui| {
            enum_combo(ui, "Ref Plane", &mut p.ref_plane, &[RefPlane::Ecliptic, RefPlane::Frame, RefPlane::BodyEquator], RefPlane::label);
            enum_combo(ui, "Ref System", &mut p.ref_system, &[RefSystem::Icrf, RefSystem::B1950], RefSystem::label);
            enum_combo(ui, "Angle Format", &mut p.ang_format, &[AngFormat::Hms, AngFormat::Deg], AngFormat::label);
            enum_combo(ui, "Calendar Format", &mut p.cal_format, &[CalFormat::Cal, CalFormat::Jd, CalFormat::Both], CalFormat::label);
            enum_combo(ui, "Calendar Type", &mut p.cal_type, &[CalType::Mixed, CalType::Gregorian], CalType::label);
            enum_combo(ui, "Range Units", &mut p.range_units, &[RangeUnits::Au, RangeUnits::Km], RangeUnits::label);
            enum_combo(ui, "Apparent", &mut p.apparent, &[Apparent::Airless, Apparent::Refracted], Apparent::label);
            ui.checkbox(&mut p.suppress_range_rate, "Suppress range rate");
            ui.checkbox(&mut p.extra_prec, "Extra precision");
            ui.checkbox(&mut p.csv_format, "CSV format");
            ui.checkbox(&mut p.r_t_s_only, "Rise/Transit/Set only");
        });

        ui.collapsing("Time Options", |ui| {
            enum_combo(ui, "Time Precision", &mut p.time_digits, &[TimeDigits::Minutes, TimeDigits::Seconds, TimeDigits::FracSec], TimeDigits::label);
            enum_combo(ui, "Time Type", &mut p.time_type, &[ObserverTimeType::Ut, ObserverTimeType::Tt], ObserverTimeType::label);
        });

        ui.collapsing("Filters", |ui| {
            ui.checkbox(&mut p.skip_daylight, "Skip daylight");
        });
    });
}

// ---------------------------------------------------------------------------
// Vectors
// ---------------------------------------------------------------------------

fn vectors_ui(ui: &mut Ui, p: &mut VectorsParams) {
    ui.push_id("vectors", |ui| {
        center_ui(ui, &mut p.center);
        time_config_ui(ui, &mut p.time);

        ui.collapsing("Output Options", |ui| {
            enum_combo(ui, "Ref Plane", &mut p.ref_plane, &[RefPlane::Ecliptic, RefPlane::Frame, RefPlane::BodyEquator], RefPlane::label);
            enum_combo(ui, "Ref System", &mut p.ref_system, &[RefSystem::Icrf, RefSystem::B1950], RefSystem::label);
            enum_combo(ui, "Units", &mut p.out_units, &[OutUnits::KmS, OutUnits::AuD, OutUnits::KmD], OutUnits::label);
            enum_combo(ui, "Table Format", &mut p.vec_table, &[
                VecTable::Position, VecTable::State, VecTable::StateRangeRate,
                VecTable::PositionRangeRate, VecTable::Velocity, VecTable::RangeRateOnly,
            ], VecTable::label);
            enum_combo(ui, "Correction", &mut p.vec_corr, &[VecCorr::None, VecCorr::LightTime, VecCorr::LightTimeStellarAberr], VecCorr::label);
            ui.checkbox(&mut p.vec_labels, "Vector labels");
            ui.checkbox(&mut p.vec_delta_t, "Delta-T (TDB-UT)");
            ui.checkbox(&mut p.csv_format, "CSV format");
        });

        ui.collapsing("Time Options", |ui| {
            enum_combo(ui, "Calendar Type", &mut p.cal_type, &[CalType::Mixed, CalType::Gregorian], CalType::label);
            enum_combo(ui, "Time Precision", &mut p.time_digits, &[TimeDigits::Minutes, TimeDigits::Seconds, TimeDigits::FracSec], TimeDigits::label);
            enum_combo(ui, "Time Type", &mut p.time_type, &[VectorTimeType::Tdb, VectorTimeType::Ut], VectorTimeType::label);
        });
    });
}

// ---------------------------------------------------------------------------
// Elements
// ---------------------------------------------------------------------------

fn elements_ui(ui: &mut Ui, p: &mut ElementsParams) {
    ui.push_id("elements", |ui| {
        center_ui(ui, &mut p.center);
        time_config_ui(ui, &mut p.time);

        ui.collapsing("Output Options", |ui| {
            enum_combo(ui, "Ref System", &mut p.ref_system, &[RefSystem::Icrf, RefSystem::B1950], RefSystem::label);
            enum_combo(ui, "Units", &mut p.out_units, &[OutUnits::KmS, OutUnits::AuD, OutUnits::KmD], OutUnits::label);
            enum_combo(ui, "Tp Type", &mut p.tp_type, &[TpType::Absolute, TpType::Relative], TpType::label);
            ui.checkbox(&mut p.elm_labels, "Element labels");
            ui.checkbox(&mut p.csv_format, "CSV format");
        });

        ui.collapsing("Time Options", |ui| {
            enum_combo(ui, "Calendar Type", &mut p.cal_type, &[CalType::Mixed, CalType::Gregorian], CalType::label);
            enum_combo(ui, "Time Precision", &mut p.time_digits, &[TimeDigits::Minutes, TimeDigits::Seconds, TimeDigits::FracSec], TimeDigits::label);
        });
    });
}

// ---------------------------------------------------------------------------
// Close Approach
// ---------------------------------------------------------------------------

fn close_approach_ui(ui: &mut Ui, p: &mut CloseApproachParams) {
    ui.push_id("close_approach", |ui| {
        ui.label("Close-approach tables are generated by the system for small-body targets.");
        enum_combo(ui, "Table Type", &mut p.ca_table_type, &[CaTableType::Standard, CaTableType::Extended], CaTableType::label);
        enum_combo(ui, "Calendar Type", &mut p.cal_type, &[CalType::Mixed, CalType::Gregorian], CalType::label);
    });
}

// ---------------------------------------------------------------------------
// SPK
// ---------------------------------------------------------------------------

fn spk_ui(ui: &mut Ui, p: &mut SpkParams) {
    ui.push_id("spk", |ui| {
        ui.label("SPK binary files — small bodies only.");
        ui.horizontal(|ui| {
            ui.label("Start:");
            ui.text_edit_singleline(&mut p.start_time);
        });
        ui.horizontal(|ui| {
            ui.label("Stop:");
            ui.text_edit_singleline(&mut p.stop_time);
        });
    });
}

// ---------------------------------------------------------------------------
// Shared sub-widgets
// ---------------------------------------------------------------------------

fn center_ui(ui: &mut Ui, center: &mut Center) {
    ui.horizontal(|ui| {
        egui::ComboBox::from_label("Center")
            .selected_text(center.label())
            .show_ui(ui, |ui| {
                if ui.selectable_label(matches!(center, Center::Geocentric), "Geocentric").clicked() {
                    *center = Center::Geocentric;
                }
                if ui.selectable_label(matches!(center, Center::BodyCenter(_)), "Body Center").clicked() {
                    *center = Center::BodyCenter(10);
                }
                if ui.selectable_label(matches!(center, Center::SiteOnBody { .. }), "Site on Body").clicked() {
                    *center = Center::SiteOnBody { site: 500, body: 399 };
                }
                if ui.selectable_label(matches!(center, Center::Coordinate { .. }), "Coordinates").clicked() {
                    *center = Center::Coordinate {
                        body: 399,
                        coord_type: CoordType::Geodetic,
                        lon: 0.0, lat: 0.0, alt: 0.0,
                    };
                }
            });
    });

    match center {
        Center::BodyCenter(body) => {
            ui.horizontal(|ui| {
                ui.label("Body ID:");
                ui.add(egui::DragValue::new(body));
            });
        }
        Center::SiteOnBody { site, body } => {
            ui.horizontal(|ui| {
                ui.label("Site ID:");
                ui.add(egui::DragValue::new(site));
                ui.label("Body ID:");
                ui.add(egui::DragValue::new(body));
            });
        }
        Center::Coordinate { body, coord_type, lon, lat, alt } => {
            ui.horizontal(|ui| {
                ui.label("Body ID:");
                ui.add(egui::DragValue::new(body));
            });
            enum_combo(ui, "Coord Type", coord_type, &[CoordType::Geodetic, CoordType::Cylindrical], CoordType::label);
            ui.horizontal(|ui| {
                ui.label("Lon:");
                ui.add(egui::DragValue::new(lon).speed(0.1));
                ui.label("Lat:");
                ui.add(egui::DragValue::new(lat).speed(0.1));
                ui.label("Alt (km):");
                ui.add(egui::DragValue::new(alt).speed(0.1));
            });
        }
        Center::Geocentric => {}
    }
}

fn time_config_ui(ui: &mut Ui, tc: &mut TimeConfig) {
    ui.horizontal(|ui| {
        ui.label("Time:");
        ui.radio_value(&mut tc.spec, TimeSpec::Span, "Span");
        ui.radio_value(&mut tc.spec, TimeSpec::List, "List");
    });

    match tc.spec {
        TimeSpec::Span => {
            ui.horizontal(|ui| {
                ui.label("Start:");
                ui.text_edit_singleline(&mut tc.span.start);
            });
            ui.horizontal(|ui| {
                ui.label("Stop:");
                ui.text_edit_singleline(&mut tc.span.stop);
            });
            ui.horizontal(|ui| {
                ui.label("Step:");
                ui.text_edit_singleline(&mut tc.span.step);
            });
        }
        TimeSpec::List => {
            ui.horizontal(|ui| {
                ui.label("Times:");
                ui.text_edit_singleline(&mut tc.list.times);
            });
            let mut lt = tc.list.list_type.unwrap_or_default();
            enum_combo(ui, "List Type", &mut lt, &[TListType::Jd, TListType::Mjd, TListType::Cal], TListType::label);
            tc.list.list_type = Some(lt);
        }
    }
}

fn enum_combo<T: PartialEq + Copy>(
    ui: &mut Ui,
    label: &str,
    value: &mut T,
    options: &[T],
    label_fn: fn(&T) -> &'static str,
) {
    ui.horizontal(|ui| {
        egui::ComboBox::from_label(label)
            .selected_text(label_fn(value))
            .show_ui(ui, |ui| {
                for opt in options {
                    ui.selectable_value(value, *opt, label_fn(opt));
                }
            });
    });
}
