use eframe::{egui, App};
use exotic_matters::gui::horizons::request_ui;
use exotic_matters::interop::horizons::Request;
use std::sync::mpsc;
use std::thread;

fn main() {
    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "Horizons Tester",
        native_options,
        Box::new(|cc| {
            let app: Box<dyn App> = Box::new(Horizons::new(cc));
            app
        }),
    )
    .expect("Could not create Horizons UI App");
}

struct Horizons {
    request: Request,
    url_cache: Option<String>,
    raw_response: Option<String>,
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
            raw_response: None,
            response_receiver: None,
            is_fetching: false,
        }
    }
}

impl eframe::App for Horizons {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(receiver) = &self.response_receiver {
            if let Ok(result) = receiver.try_recv() {
                self.raw_response = Some(match result {
                    Ok(text) => text,
                    Err(err) => err,
                });
                self.response_receiver = None;
                self.is_fetching = false;
            } else {
                ctx.request_repaint();
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            request_ui(ui, &mut self.request);

            ui.heading("URL");
            if ui.button("Generate URL").clicked() {
                self.url_cache = Some(format!("{}", self.request))
            }

            if let Some(url) = &self.url_cache {
                ui.label(url);
            }

            ui.heading("Response");
            let send_button = ui.add_enabled(!self.is_fetching, egui::Button::new("Send"));

            if send_button.clicked() {
                if self.url_cache.is_none() {
                    self.url_cache = Some(format!("{}", self.request));
                }
                let url = self.url_cache.as_ref().unwrap().clone();
                let (tx, rx) = mpsc::channel();
                self.response_receiver = Some(rx);
                self.is_fetching = true;
                self.raw_response = None;

                thread::spawn(move || {
                    let result = match reqwest::blocking::get(&url) {
                        Ok(response) => response.text().map(|s| { s.replace("\\n", "\n")}).map_err(|e| e.to_string()),
                        Err(e) => Err(e.to_string()),
                    };
                    tx.send(result).ok();
                });
            }

            if self.is_fetching {
                ui.spinner();
            }

            if let Some(response) = &self.raw_response {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.monospace(response);
                });
            }
        });
    }
}

