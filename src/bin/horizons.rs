use eframe::egui;
use exotic_matters::gui::horizons::request_ui;
use exotic_matters::interop::horizons::Request;
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
    url_cache: Option<String>,
    api_error: Option<String>,
    result_text: Option<String>,
    response_receiver: Option<mpsc::Receiver<Result<String, String>>>,
    is_fetching: bool,
}

impl Horizons {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self::default()
    }
}

impl Default for Horizons {
    fn default() -> Self {
        Self {
            request: Request::default(),
            url_cache: None,
            api_error: None,
            result_text: None,
            response_receiver: None,
            is_fetching: false,
        }
    }
}

/// Try to extract the "result" and "error" fields from the JSON response.
fn parse_horizons_json(body: &str) -> (Option<String>, Option<String>) {
    let mut result = None;
    let mut error = None;
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
            error = Some(e.to_string());
        }
        if let Some(r) = v.get("result").and_then(|r| r.as_str()) {
            result = Some(r.replace("\\n", "\n"));
        }
    }
    (result, error)
}

impl eframe::App for Horizons {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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
            request_ui(ui, &mut self.request);

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

            let send_button = ui.add_enabled(!self.is_fetching, egui::Button::new("Send"));
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
                        Ok(response) => response
                            .text()
                            .map_err(|e| e.to_string()),
                        Err(e) => Err(e.to_string()),
                    };
                    tx.send(result).ok();
                });
            }

            if self.is_fetching {
                ui.spinner();
            }

            if let Some(err) = &self.api_error {
                ui.colored_label(egui::Color32::from_rgb(220, 60, 60), format!("API Error: {}", err));
            }

            if let Some(text) = &self.result_text {
                egui::ScrollArea::vertical().id_salt("horizons_response").show(ui, |ui| {
                    ui.monospace(text);
                });
            }
        });
    }
}
