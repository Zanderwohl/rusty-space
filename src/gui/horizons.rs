use std::any::Any;
use eframe::egui;
use eframe::egui::Ui;
use crate::interop::horizons::*;

pub fn request_ui(ui: &mut Ui, request: &mut Request) {
    ui.heading("Request");
    format_ui(ui, &mut request.format);
    object_ui(ui, &mut request.object);
    object_data_ui(ui, &mut request.object_data);
    ephemeris_ui(ui, &mut request.ephemeris);
}

pub fn format_ui(ui: &mut Ui, format: &mut Format) {
    ui.label("Format:");
    ui.radio_value(format, Format::JSON, "JSON");
    ui.radio_value(format, Format::Text, "Text");
}

pub fn object_ui(ui: &mut Ui, object: &mut Object) {
    ui.horizontal(|ui| {
        let object_variant_text = match object {
            Object::Number(_) => "Number",
            Object::Name(_) => "Name",
            Object::IauNumber(_) => "IAU Number",
        };

        egui::ComboBox::from_label("Object")
            .selected_text(object_variant_text)
            .show_ui(ui, |ui| {
                if ui.selectable_label(matches!(object, Object::Number(_)), "Number").clicked() {
                    *object = Object::Number(399);
                }
                if ui.selectable_label(matches!(object, Object::Name(_)), "Name").clicked() {
                    *object = Object::Name("Earth".to_string());
                }
                if ui.selectable_label(matches!(object, Object::IauNumber(_)), "IAU Number").clicked() {
                    *object = Object::IauNumber(399);
                }
            });

        match object {
            Object::Number(n) => {
                ui.add(egui::DragValue::new(n));
            }
            Object::Name(s) => {
                ui.text_edit_singleline(s);
            }
            Object::IauNumber(n) => {
                ui.add(egui::DragValue::new(n));
            }
        }
    });
}

pub fn object_data_ui(ui: &mut Ui, obj_data: &mut ObjectData) {
    ui.horizontal(|ui| {
        ui.label("Object Data:");
        ui.radio_value(obj_data, ObjectData::YES, "Yes");
        ui.radio_value(obj_data, ObjectData::NO, "No");
    });
}

pub fn ephemeris_ui(ui: &mut Ui, ephemeris: &mut Ephemeris) {
    ui.collapsing("Ephemeris", |ui| {
        let mut make_ephem = ephemeris.is_yes();
        ui.checkbox(&mut make_ephem, "Make Ephemeris");

        if make_ephem && !ephemeris.is_yes() {
            *ephemeris = Ephemeris::YES(Default::default());
        } else if !make_ephem && ephemeris.is_yes() {
            *ephemeris = Ephemeris::No;
        }

        if let Ephemeris::YES(params) = ephemeris {
            ephemeris_params_ui(ui, params);
        }
    });
}

pub fn ephemeris_params_ui(ui: &mut Ui, params: &mut EphemerisParams) {
    ephemeris_type_ui(ui, &mut params.ephemeris_type);
    if let Some(center) = &mut params.center {
        center_ui(ui, center);
    }
}

pub fn ephemeris_type_ui(ui: &mut Ui, ephem_type: &mut EphemerisType) {
    egui::ComboBox::from_label("Ephemeris type")
        .selected_text(format!("{}", ephem_type))
        .show_ui(ui, |ui| {
            ui.selectable_value(ephem_type, EphemerisType::Observables, "Observables");
            ui.selectable_value(ephem_type, EphemerisType::OsculatingOrbitalElements, "Osculating Orbital Elements");
            ui.selectable_value(ephem_type, EphemerisType::CartesianStateVectors, "Cartesian State Vectors");
            ui.selectable_value(ephem_type, EphemerisType::CloseApproaches, "Close Approaches");
            ui.selectable_value(ephem_type, EphemerisType::SpkBinaryTrajectoryFiles, "SPK Binary Trajectory Files");
        });
}

pub fn center_ui(ui: &mut Ui, center: &mut Center) {
    let center_type_text = match center {
        Center::Geocentric => "Geocentric",
        Center::EarthSite(_) => "Earth Site",
        Center::OtherSite(_, _) => "Other Site",
        Center::BodyCenter(_) => "Body Center",
    };

    egui::ComboBox::from_label("Center")
        .selected_text(center_type_text)
        .show_ui(ui, |ui| {
            if ui.selectable_label(matches!(center, Center::Geocentric), "Geocentric").clicked() {
                *center = Center::Geocentric;
            }
            if ui.selectable_label(matches!(center, Center::EarthSite(_)), "Earth Site").clicked() {
                *center = Center::EarthSite(String::new());
            }
            if ui.selectable_label(matches!(center, Center::OtherSite(_, _)), "Other Site").clicked() {
                *center = Center::OtherSite(String::new(), String::new());
            }
            if ui.selectable_label(matches!(center, Center::BodyCenter(_)), "Body Center").clicked() {
                *center = Center::BodyCenter(String::new());
            }
        });

    match center {
        Center::EarthSite(site) => {
            ui.text_edit_singleline(site);
        }
        Center::OtherSite(body, site) => {
            ui.text_edit_singleline(body);
            ui.text_edit_singleline(site);
        }
        Center::BodyCenter(body) => {
            ui.text_edit_singleline(body);
        }
        _ => {}
    }
}

impl Ephemeris {
    fn is_yes(&self) -> bool {
        matches!(self, Ephemeris::YES(_))
    }
}
