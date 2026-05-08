use eframe::egui::{
    Align, CentralPanel, Color32, Context, Grid, Layout, RichText, ScrollArea, SidePanel, TextEdit,
    TextStyle, Ui,
};
use egui_extras::{Column, TableBuilder};
use std::{
    collections::VecDeque,
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use zenoh::{
    key_expr::OwnedKeyExpr,
    sample::{Sample, SampleKind},
};

use crate::task_zenoh::{LivelinessSubData, LivelinessTokenData};

pub enum Event {
    DeclareToken(Box<LivelinessTokenData>),
    UndeclareToken(u64),
    AddSub(Box<LivelinessSubData>),
    DelSub(u64),
}

struct TokenEntry {
    id: u64,
    key_expr: String,
    declared: bool,
    err_str: Option<String>,
}

struct LivelinessEvent {
    key: String,
    kind: SampleKind,
    time: SystemTime,
}

pub struct PageLiveliness {
    pub events: VecDeque<Event>,
    token_id_count: u64,
    tokens: Vec<TokenEntry>,
    new_token_key: String,
    token_err: Option<String>,
    sub_id: u64,
    sub_key: String,
    subscribed: bool,
    sub_err: Option<String>,
    liveliness_events: VecDeque<LivelinessEvent>,
    max_events: usize,
}

impl Default for PageLiveliness {
    fn default() -> Self {
        PageLiveliness {
            events: VecDeque::new(),
            token_id_count: 0,
            tokens: Vec::new(),
            new_token_key: "zenoh-hammer/liveliness".to_string(),
            token_err: None,
            sub_id: 0,
            sub_key: "zenoh-hammer/**".to_string(),
            subscribed: false,
            sub_err: None,
            liveliness_events: VecDeque::new(),
            max_events: 500,
        }
    }
}

impl PageLiveliness {
    pub fn show(&mut self, ctx: &Context) {
        SidePanel::left("page_liveliness_left")
            .resizable(true)
            .default_width(240.0)
            .show(ctx, |ui| {
                self.show_tokens(ui);
            });

        CentralPanel::default().show(ctx, |ui| {
            self.show_subscriber(ui);
        });
    }

    fn show_tokens(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Declare Token").strong());
        ui.separator();

        ui.label("key expression:");
        ui.add(
            TextEdit::singleline(&mut self.new_token_key)
                .font(TextStyle::Monospace)
                .desired_width(f32::INFINITY),
        );

        if ui.button("Declare").clicked() {
            let key_str = self.new_token_key.trim().to_string();
            match OwnedKeyExpr::from_str(&key_str) {
                Ok(key_expr) => {
                    self.token_err = None;
                    self.token_id_count += 1;
                    let id = self.token_id_count;
                    self.tokens.push(TokenEntry {
                        id,
                        key_expr: key_str,
                        declared: false,
                        err_str: None,
                    });
                    self.events
                        .push_back(Event::DeclareToken(Box::new(LivelinessTokenData {
                            id,
                            key_expr,
                        })));
                }
                Err(e) => {
                    self.token_err = Some(e.to_string());
                }
            }
        }

        if let Some(e) = &self.token_err {
            ui.label(RichText::new(e).color(Color32::RED).small());
        }

        ui.add_space(8.0);
        ui.label(RichText::new("Active Tokens").strong());
        ui.separator();

        ScrollArea::vertical()
            .id_salt("liveliness_token_scroll")
            .show(ui, |ui| {
                let mut to_remove: Option<u64> = None;
                for token in &self.tokens {
                    ui.horizontal(|ui| {
                        if ui.small_button("✕").clicked() {
                            to_remove = Some(token.id);
                        }
                        let (color, suffix) = if token.declared {
                            (Color32::GREEN, " ✓")
                        } else if token.err_str.is_some() {
                            (Color32::RED, " ✗")
                        } else {
                            (ui.visuals().text_color(), "")
                        };
                        ui.label(
                            RichText::new(format!("{}{}", token.key_expr, suffix))
                                .monospace()
                                .color(color),
                        );
                    });
                    if let Some(e) = &token.err_str {
                        ui.label(RichText::new(e).color(Color32::RED).small());
                    }
                }
                if let Some(id) = to_remove {
                    if let Some(pos) = self.tokens.iter().position(|t| t.id == id) {
                        self.tokens.remove(pos);
                    }
                    self.events.push_back(Event::UndeclareToken(id));
                }
            });
    }

    fn show_subscriber(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Liveliness Subscriber").strong());
        ui.separator();

        Grid::new("liveliness_sub_grid")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("key expression:");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let label = if self.subscribed {
                        "unsubscribe"
                    } else {
                        "subscribe"
                    };
                    if ui.selectable_label(self.subscribed, label).clicked() {
                        if !self.subscribed {
                            let key_str = self.sub_key.trim().to_string();
                            match OwnedKeyExpr::from_str(&key_str) {
                                Ok(key_expr) => {
                                    self.sub_id += 1;
                                    self.sub_err = None;
                                    self.events.push_back(Event::AddSub(Box::new(
                                        LivelinessSubData {
                                            id: self.sub_id,
                                            key_expr,
                                        },
                                    )));
                                    self.subscribed = true;
                                }
                                Err(e) => {
                                    self.sub_err = Some(e.to_string());
                                }
                            }
                        } else {
                            self.events.push_back(Event::DelSub(self.sub_id));
                            self.subscribed = false;
                        }
                    }
                    ui.add_enabled(
                        !self.subscribed,
                        TextEdit::singleline(&mut self.sub_key)
                            .font(TextStyle::Monospace)
                            .desired_width(f32::INFINITY),
                    );
                });
                ui.end_row();
            });

        if let Some(e) = &self.sub_err {
            ui.label(RichText::new(e).color(Color32::RED));
        }

        ui.horizontal(|ui| {
            if ui.button("clear").clicked() {
                self.liveliness_events.clear();
            }
            ui.label(
                RichText::new(format!("{} events", self.liveliness_events.len()))
                    .color(Color32::GRAY),
            );
        });

        ui.separator();

        let text_height = 18.0;
        ScrollArea::horizontal()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                TableBuilder::new(ui)
                    .striped(true)
                    .cell_layout(Layout::left_to_right(Align::Center))
                    .column(Column::initial(130.0).resizable(true).clip(true))
                    .column(Column::initial(90.0).resizable(true).clip(true))
                    .column(Column::remainder().clip(true))
                    .header(text_height, |mut header| {
                        header.col(|ui| {
                            ui.strong("time");
                        });
                        header.col(|ui| {
                            ui.strong("kind");
                        });
                        header.col(|ui| {
                            ui.strong("key");
                        });
                    })
                    .body(|body| {
                        let events: Vec<&LivelinessEvent> =
                            self.liveliness_events.iter().rev().collect();
                        body.rows(text_height, events.len(), |mut row| {
                            let event = events[row.index()];
                            row.col(|ui| {
                                ui.label(
                                    RichText::new(format_system_time(event.time))
                                        .monospace()
                                        .size(12.0),
                                );
                            });
                            row.col(|ui| {
                                let (text, color) = match event.kind {
                                    SampleKind::Put => {
                                        ("appeared", Color32::from_rgb(100, 200, 100))
                                    }
                                    SampleKind::Delete => {
                                        ("disappeared", Color32::from_rgb(220, 150, 60))
                                    }
                                };
                                ui.label(RichText::new(text).color(color));
                            });
                            row.col(|ui| {
                                ui.label(
                                    RichText::new(&event.key).monospace().size(12.0),
                                );
                            });
                        });
                    });
            });
    }

    pub fn processing_declare_token_res(&mut self, id: u64, r: Result<(), String>) {
        if let Some(token) = self.tokens.iter_mut().find(|t| t.id == id) {
            match r {
                Ok(_) => {
                    token.declared = true;
                    token.err_str = None;
                }
                Err(e) => {
                    token.declared = false;
                    token.err_str = Some(e);
                }
            }
        }
    }

    pub fn processing_add_sub_res(&mut self, id: u64, r: Result<(), String>) {
        if id == self.sub_id {
            match r {
                Ok(_) => {
                    self.sub_err = None;
                    self.subscribed = true;
                }
                Err(e) => {
                    self.sub_err = Some(e);
                    self.subscribed = false;
                }
            }
        }
    }

    pub fn processing_del_sub_res(&mut self, id: u64) {
        if id == self.sub_id {
            self.subscribed = false;
        }
    }

    pub fn processing_sub_cb(&mut self, id: u64, sample: Sample, _receipt_time: SystemTime) {
        if id != self.sub_id {
            return;
        }
        let entry = LivelinessEvent {
            key: sample.key_expr().to_string(),
            kind: sample.kind().clone(),
            time: SystemTime::now(),
        };
        if self.liveliness_events.len() >= self.max_events {
            self.liveliness_events.pop_front();
        }
        self.liveliness_events.push_back(entry);
    }
}

fn format_system_time(t: SystemTime) -> String {
    let d = t.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
    let secs = d.as_secs();
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    let ms = d.subsec_millis();
    format!("{:02}:{:02}:{:02}.{:03}", h, m, s, ms)
}
