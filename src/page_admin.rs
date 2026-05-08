use eframe::egui::{
    CentralPanel, CollapsingHeader, Color32, Context, DragValue, RichText, ScrollArea, TextEdit,
    TextStyle, Ui,
};
use egui_json_tree::JsonTree;
use std::collections::VecDeque;

use crate::task_zenoh::AdminQueryData;

pub enum Event {
    Query(Box<AdminQueryData>),
}

enum AdminPayload {
    Json(serde_json::Value),
    Text(String),
    Binary { encoding: String, hex_preview: String, total_bytes: usize },
}

struct AdminResult {
    key: String,
    encoding: String,
    payload: AdminPayload,
}

pub struct PageAdmin {
    pub events: VecDeque<Event>,
    key_expr: String,
    timeout_ms: u64,
    query_id_count: u64,
    current_query_id: u64,
    is_querying: bool,
    results: Vec<AdminResult>,
    result_count: usize,
    err_str: Option<String>,
}

impl Default for PageAdmin {
    fn default() -> Self {
        PageAdmin {
            events: VecDeque::new(),
            key_expr: "@/**".to_string(),
            timeout_ms: 5000,
            query_id_count: 0,
            current_query_id: 0,
            is_querying: false,
            results: Vec::new(),
            result_count: 0,
            err_str: None,
        }
    }
}

fn decompress_gzip(data: &[u8]) -> Option<Vec<u8>> {
    use flate2::read::GzDecoder;
    use std::io::Read;
    let mut out = Vec::new();
    GzDecoder::new(data).read_to_end(&mut out).ok()?;
    Some(out)
}

/// Decode a zenoh payload into something displayable.
///
/// Handles: gzip-compressed content, raw JSON, CDR-prefixed strings, plain text, binary.
fn decode_payload(payload: &[u8], encoding: &str) -> AdminPayload {
    // Decompress gzip if the magic bytes or encoding say so
    let is_gzip = payload.starts_with(&[0x1f, 0x8b])
        || encoding.contains("content-encoding=gzip")
        || encoding.contains("gzip");
    let decompressed;
    let data: &[u8] = if is_gzip {
        match decompress_gzip(payload) {
            Some(d) => { decompressed = d; &decompressed }
            None => payload,
        }
    } else {
        payload
    };

    let try_text = |bytes: &[u8]| -> Option<AdminPayload> {
        let s = std::str::from_utf8(bytes).ok()?;
        let trimmed = s.trim_matches('\0').trim();
        if trimmed.is_empty() {
            return Some(AdminPayload::Text(String::new()));
        }
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
            Some(AdminPayload::Json(json))
        } else {
            Some(AdminPayload::Text(trimmed.to_string()))
        }
    };

    // Try raw UTF-8 / JSON
    if let Some(p) = try_text(data) {
        return p;
    }

    // Try CDR-encoded string: 4-byte LE u32 length prefix + UTF-8 content
    if data.len() >= 5 {
        let claimed = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        if claimed > 0 && claimed <= data.len().saturating_sub(4) {
            if let Some(p) = try_text(&data[4..4 + claimed]) {
                return p;
            }
        }
        if let Some(p) = try_text(&data[4..]) {
            return p;
        }
    }

    // Fall back: hexdump preview of the raw (pre-decompress) bytes
    let hex_preview: String = payload
        .chunks(16)
        .take(8)
        .map(|chunk| {
            chunk
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n");

    AdminPayload::Binary {
        encoding: encoding.to_string(),
        hex_preview,
        total_bytes: payload.len(),
    }
}

impl PageAdmin {
    pub fn show(&mut self, ctx: &Context) {
        CentralPanel::default().show(ctx, |ui| {
            self.show_content(ui);
        });
    }

    fn show_content(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("key:");
            ui.add(
                TextEdit::singleline(&mut self.key_expr)
                    .font(TextStyle::Monospace)
                    .desired_width(260.0),
            );

            ui.label("timeout (ms):");
            ui.add(
                DragValue::new(&mut self.timeout_ms)
                    .range(500..=30000)
                    .speed(100.0),
            );

            ui.add_enabled_ui(!self.is_querying, |ui| {
                if ui.button("Query").clicked() {
                    self.start_query();
                }
            });

            if self.is_querying {
                ui.label(RichText::new("querying…").color(Color32::YELLOW));
            } else if self.result_count > 0 {
                ui.label(
                    RichText::new(format!("{} replies", self.result_count)).color(Color32::GRAY),
                );
            }
        });

        if let Some(e) = &self.err_str {
            ui.label(RichText::new(e).color(Color32::RED));
        }

        ui.separator();

        if self.results.is_empty() && !self.is_querying {
            ui.label(
                RichText::new(
                    "No results. Connect to a zenoh session and click Query.\n\
                     The admin space (@/**) is enabled by default in zenohd and the Docker image.",
                )
                .color(Color32::GRAY),
            );
            return;
        }

        ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (i, result) in self.results.iter().enumerate() {
                    let header_text = RichText::new(&result.key).monospace().size(12.0);
                    CollapsingHeader::new(header_text)
                        .default_open(false)
                        .id_salt(format!("admin_result_{}", i))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(format!("encoding: {}", result.encoding))
                                    .color(Color32::GRAY)
                                    .small(),
                            );
                            match &result.payload {
                                AdminPayload::Json(json) => {
                                    JsonTree::new(format!("admin_json_{}", i), json).show(ui);
                                }
                                AdminPayload::Text(text) => {
                                    if text.is_empty() {
                                        ui.label(
                                            RichText::new("(empty)").color(Color32::GRAY).small(),
                                        );
                                    } else {
                                        ui.label(RichText::new(text).monospace().size(11.0));
                                    }
                                }
                                AdminPayload::Binary {
                                    encoding,
                                    hex_preview,
                                    total_bytes,
                                } => {
                                    ui.label(
                                        RichText::new(format!(
                                            "binary data — {} bytes (encoding: {})",
                                            total_bytes, encoding
                                        ))
                                        .color(Color32::YELLOW)
                                        .small(),
                                    );
                                    ui.label(
                                        RichText::new(hex_preview).monospace().size(10.0),
                                    );
                                    if *total_bytes > 128 {
                                        ui.label(
                                            RichText::new(format!(
                                                "… {} bytes total",
                                                total_bytes
                                            ))
                                            .color(Color32::GRAY)
                                            .small(),
                                        );
                                    }
                                }
                            }
                        });
                }
            });
    }

    fn start_query(&mut self) {
        self.query_id_count += 1;
        self.current_query_id = self.query_id_count;
        self.is_querying = true;
        self.results.clear();
        self.result_count = 0;
        self.err_str = None;
        self.events.push_back(Event::Query(Box::new(AdminQueryData {
            id: self.current_query_id,
            key_expr: self.key_expr.clone(),
            timeout_ms: self.timeout_ms,
        })));
    }

    pub fn set_not_connected(&mut self) {
        self.is_querying = false;
        self.err_str = Some("not connected to a zenoh session".to_string());
    }

    pub fn processing_query_res(
        &mut self,
        id: u64,
        key: String,
        encoding: String,
        payload: Vec<u8>,
        is_ok: bool,
    ) {
        if id != self.current_query_id {
            return;
        }
        self.result_count += 1;
        if is_ok && self.results.len() < 500 {
            let decoded = decode_payload(&payload, &encoding);
            self.results.push(AdminResult {
                key,
                encoding,
                payload: decoded,
            });
        }
    }

    pub fn processing_query_done(&mut self, id: u64) {
        if id == self.current_query_id {
            self.is_querying = false;
            self.results.sort_by(|a, b| a.key.cmp(&b.key));
        }
    }
}
