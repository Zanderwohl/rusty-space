use eframe::egui;
use exotic_matters::gui::horizons::{request_ui, BodyListStatus};
use exotic_matters::interop::horizons::{self, MajorBody, Request};
use std::sync::mpsc;
use std::thread;

fn main() {
    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "Horizons Tester",
        native_options,
        Box::new(|cc| Ok(Box::new(Horizons::new(cc)))),
    )
    .expect("Could not create Horizons UI App");
}

struct Horizons {
    request: Request,
    body_list: Vec<MajorBody>,
    body_list_status: BodyListStatus,
    body_list_receiver: Option<mpsc::Receiver<Result<String, String>>>,
    body_search: String,
    url_cache: Option<String>,
    api_error: Option<String>,
    result_text: Option<String>,
    response_receiver: Option<mpsc::Receiver<Result<String, String>>>,
    is_fetching: bool,
}


impl Horizons {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = mpsc::channel();
        let url = horizons::major_body_list_url();
        thread::spawn(move || {
            let result = match reqwest::blocking::get(&url) {
                Ok(response) => response.text().map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            tx.send(result).ok();
        });

        Self {
            request: Request::default(),
            body_list: Vec::new(),
            body_list_status: BodyListStatus::Loading,
            body_list_receiver: Some(rx),
            body_search: String::new(),
            url_cache: None,
            api_error: None,
            result_text: None,
            response_receiver: None,
            is_fetching: false,
        }
    }
}

fn parse_horizons_json(body: &str) -> (Option<String>, Option<String>) {
    let mut result = None;
    let mut error = None;
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
            error = Some(e.to_string());
        }
        if let Some(r) = v.get("result").and_then(|r| r.as_str()) {
            result = Some(r.to_string());
        }
    }
    (result, error)
}

impl eframe::App for Horizons {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Poll body list fetch
        if let Some(rx) = &self.body_list_receiver {
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok(body) => {
                        let (result_text, error) = parse_horizons_json(&body);
                        if let Some(err) = error {
                            self.body_list_status = BodyListStatus::Failed(err);
                        } else if let Some(text) = result_text {
                            self.body_list = horizons::parse_major_body_list(&text);
                            self.body_list_status = BodyListStatus::Ready;
                        } else {
                            self.body_list_status =
                                BodyListStatus::Failed("Empty response".into());
                        }
                    }
                    Err(err) => {
                        self.body_list_status = BodyListStatus::Failed(err);
                    }
                }
                self.body_list_receiver = None;
            } else {
                ctx.request_repaint();
            }
        }

        // Poll ephemeris request fetch
        if let Some(receiver) = &self.response_receiver {
            if let Ok(result) = receiver.try_recv() {
                match result {
                    Ok(body) => {
                        let (result, error) = parse_horizons_json(&body);
                        self.api_error = error;
                        self.result_text = result.or(Some(body));
                    }
                    Err(err) => {
                        self.api_error = Some(err);
                        self.result_text = None;
                    }
                }
                self.response_receiver = None;
                self.is_fetching = false;
            } else {
                ctx.request_repaint();
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            request_ui(
                ui,
                &mut self.request,
                &self.body_list,
                &self.body_list_status,
                &mut self.body_search,
            );

            ui.separator();
            ui.heading("URL");
            if ui.button("Generate URL").clicked() {
                self.url_cache = Some(self.request.to_url());
            }
            if let Some(url) = &self.url_cache {
                ui.label(url);
            }

            ui.separator();
            ui.heading("Response");

            let send_button =
                ui.add_enabled(!self.is_fetching, egui::Button::new("Send"));
            if send_button.clicked() {
                let url = self.request.to_url();
                self.url_cache = Some(url.clone());
                let (tx, rx) = mpsc::channel();
                self.response_receiver = Some(rx);
                self.is_fetching = true;
                self.api_error = None;
                self.result_text = None;

                thread::spawn(move || {
                    let result = match reqwest::blocking::get(&url) {
                        Ok(response) => {
                            response.text().map_err(|e| e.to_string())
                        }
                        Err(e) => Err(e.to_string()),
                    };
                    tx.send(result).ok();
                });
            }

            if self.is_fetching {
                ui.spinner();
            }

            if let Some(err) = &self.api_error {
                ui.colored_label(
                    egui::Color32::from_rgb(220, 60, 60),
                    format!("API Error: {}", err),
                );
            }

            if let Some(text) = &self.result_text {
                egui::ScrollArea::vertical()
                    .id_salt("horizons_response")
                    .show(ui, |ui| {
                        ui.monospace(text);
                    });
            }
        });
    }
}
