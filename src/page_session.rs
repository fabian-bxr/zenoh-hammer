use eframe::egui::{
    Align, Button, CentralPanel, CollapsingHeader, Color32, Context, Grid, Layout, RichText,
    ScrollArea, SidePanel, TextEdit, TextStyle, Ui, Widget,
};
use egui_dnd::dnd;
use crate::file_dialog_helper::NativeFileDialog;
use egui_json_tree::JsonTree;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use serde_json;
use toml;
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};
use crate::task_zenoh::SessionInfoData;

pub enum Event {
    Connect(Box<(u64, PathBuf)>),
    Disconnect,
    RequestSessionInfo,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct ArchiveConfigFileData {
    name: String,
    path: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct ArchivePageSession {
    config_files: Vec<ArchiveConfigFileData>,
}

struct ConfigFileData {
    id: u64,
    name: String,
    path: Option<PathBuf>,
    path_str: String,
    err_str: Option<String>,
    selected_page: FilePage,
    source: String,
    format: String,
    serde_json_value: serde_json::Value,
    is_toml: bool,
    is_dirty: bool,
}

impl From<&ConfigFileData> for ArchiveConfigFileData {
    fn from(value: &ConfigFileData) -> Self {
        let path = match &value.path {
            None => String::new(),
            Some(o) => o.to_string_lossy().to_string(),
        };
        ArchiveConfigFileData {
            name: value.name.clone(),
            path,
        }
    }
}

impl TryFrom<&ArchiveConfigFileData> for ConfigFileData {
    type Error = String;
    fn try_from(value: &ArchiveConfigFileData) -> Result<Self, Self::Error> {
        let path_str = value.path.clone();
        let path = if path_str.is_empty() {
            None
        } else {
            Some(PathBuf::from(&path_str))
        };
        let is_toml = path.as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str()) == Some("toml");
        Ok(ConfigFileData::blank(value.name.clone(), path, path_str, is_toml))
    }
}

impl TryFrom<ArchiveConfigFileData> for ConfigFileData {
    type Error = String;
    fn try_from(value: ArchiveConfigFileData) -> Result<Self, Self::Error> {
        ConfigFileData::try_from(&value)
    }
}

impl ConfigFileData {
    fn new(name: String, path: PathBuf) -> Self {
        let is_toml = path.extension().and_then(|e| e.to_str()) == Some("toml");
        let path_str = path.to_string_lossy().to_string();
        ConfigFileData::blank(name, Some(path), path_str, is_toml)
    }

    fn new_template() -> Self {
        let source = "mode = \"peer\"\n".to_string();
        let mut cfg = ConfigFileData::blank("new config".to_string(), None, String::new(), true);
        cfg.is_dirty = true;
        cfg.parse_source_into_self(&source);
        cfg.source = source;
        cfg
    }

    fn blank(name: String, path: Option<PathBuf>, path_str: String, is_toml: bool) -> Self {
        ConfigFileData {
            id: 0,
            name,
            path,
            path_str,
            err_str: None,
            selected_page: FilePage::Source,
            source: String::new(),
            format: String::new(),
            serde_json_value: serde_json::Value::Null,
            is_toml,
            is_dirty: false,
        }
    }

    /// Parse `source` and update `format` and `serde_json_value`. Returns false on parse error.
    fn parse_source_into_self(&mut self, source: &str) -> bool {
        let result: Result<serde_json::Value, String> = if self.is_toml {
            toml::from_str::<toml::Value>(source)
                .map_err(|e| e.to_string())
                .and_then(|tv| serde_json::to_value(tv).map_err(|e| e.to_string()))
        } else {
            json5::from_str::<serde_json::Value>(source).map_err(|e| e.to_string())
        };
        match result {
            Ok(value) => {
                self.format = serde_json::to_string_pretty(&value).unwrap_or_default();
                self.serde_json_value = value;
                true
            }
            Err(e) => {
                self.format = String::new();
                self.serde_json_value = serde_json::Value::Null;
                self.err_str = Some(format!("parse error: {e}"));
                false
            }
        }
    }

    fn try_save(&mut self) {
        // If no path is set yet, try to use whatever is typed in path_str.
        if self.path.is_none() && !self.path_str.trim().is_empty() {
            let p = PathBuf::from(self.path_str.trim());
            self.is_toml = p.extension().and_then(|e| e.to_str()) == Some("toml");
            self.path = Some(p);
        }

        let Some(path) = &self.path else {
            self.err_str = Some("Enter a file path below, then save.".to_string());
            return;
        };

        match fs::write(path, self.source.as_bytes()) {
            Ok(_) => {
                self.is_dirty = false;
                self.err_str = None;
                info!("saved config to \"{}\"", path.display());
            }
            Err(e) => {
                self.err_str = Some(format!("save failed: {e}"));
                warn!("failed to save config: {e}");
            }
        }
    }

    fn load_from_file(&mut self, p: PathBuf) {
        info!("loading config file \"{}\"", p.display());
        self.source.clear();
        self.format.clear();
        self.serde_json_value = serde_json::Value::Null;
        self.err_str = None;

        let source = match fs::read_to_string(p.as_path()) {
            Ok(o) => o,
            Err(e) => {
                warn!("failed to load config file, {e}");
                self.err_str = Some("failed to load config file".to_string());
                return;
            }
        };

        self.is_toml = p.extension().and_then(|e| e.to_str()) == Some("toml");
        if !self.parse_source_into_self(&source) {
            return;
        }
        self.source = source;
        self.is_dirty = false;
        info!("load config file ok \"{}\"", p.display());
    }

    fn show(
        &mut self,
        ui: &mut Ui,
        connected_config_file_id: Option<u64>,
        events: &mut VecDeque<Event>,
    ) {
        let is_connected = connected_config_file_id == Some(self.id);
        self.show_name_path(ui, is_connected, events);

        ui.add_space(10.0);

        ui.horizontal(|ui| {
            let source_label = if self.is_dirty { "● source" } else { "source" };
            if ui.selectable_label(self.selected_page == FilePage::Source, source_label).clicked() {
                self.selected_page = FilePage::Source;
            }
            if ui.selectable_label(self.selected_page == FilePage::Format, "format").clicked() {
                self.selected_page = FilePage::Format;
            }
            if ui.selectable_label(self.selected_page == FilePage::Tree, "tree").clicked() {
                self.selected_page = FilePage::Tree;
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add_enabled(self.is_dirty && !is_connected, Button::new("save")).clicked() {
                    self.try_save();
                }
            });
        });

        ui.add_space(4.0);

        // Reserve a fixed strip at the bottom for the error label so the textbox
        // position never shifts when an error appears or clears.
        let error_strip = 22.0;
        ScrollArea::both()
            .max_height(ui.available_height() - error_strip)
            .auto_shrink([false, true])
            .show(ui, |ui| match self.selected_page {
                FilePage::Source => {
                    let response = TextEdit::multiline(&mut self.source)
                        .desired_width(f32::INFINITY)
                        .code_editor()
                        .interactive(!is_connected)
                        .ui(ui);
                    if response.changed() {
                        self.is_dirty = true;
                        self.err_str = None;
                        let source = self.source.clone();
                        self.parse_source_into_self(&source);
                    }
                }
                FilePage::Format => {
                    TextEdit::multiline(&mut self.format)
                        .desired_width(f32::INFINITY)
                        .code_editor()
                        .interactive(false)
                        .ui(ui);
                }
                FilePage::Tree => {
                    JsonTree::new("page_session_json_tree", &self.serde_json_value).show(ui);
                }
            });

        if let Some(s) = &self.err_str {
            ui.label(RichText::new(s).color(Color32::RED));
        }
    }

    fn show_name_path(
        &mut self,
        ui: &mut Ui,
        is_connected: bool,
        events: &mut VecDeque<Event>,
    ) {
        Grid::new("page_session_config_file")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("name");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if is_connected {
                        if ui.selectable_label(true, "close session").clicked() {
                            events.push_back(Event::Disconnect);
                        }
                    } else {
                        if ui.selectable_label(false, "open session").clicked() {
                            self.err_str = None;
                            // Auto-save unsaved edits before connecting.
                            if self.is_dirty {
                                self.try_save();
                                if self.err_str.is_some() {
                                    return;
                                }
                            }
                            if let Some(path_buf) = &self.path {
                                let event = Event::Connect(Box::new((self.id, path_buf.clone())));
                                events.push_back(event);
                            } else {
                                self.err_str = Some("Enter a file path below and save first.".to_string());
                            }
                        }
                    }

                    ui.add_enabled_ui(!is_connected, |ui| {
                        if ui.add_enabled(self.path.is_some(), Button::new("load")).clicked() {
                            self.err_str = None;
                            if let Some(path_buf) = self.path.clone() {
                                self.load_from_file(path_buf);
                            }
                        }
                    });

                    TextEdit::singleline(&mut self.name)
                        .desired_width(3000.0)
                        .font(TextStyle::Monospace)
                        .interactive(!is_connected)
                        .ui(ui);
                });
                ui.end_row();

                ui.label("path");
                let path_response = TextEdit::multiline(&mut self.path_str)
                    .desired_rows(1)
                    .desired_width(3000.0)
                    .font(TextStyle::Monospace)
                    .interactive(!is_connected)
                    .ui(ui);
                if path_response.changed() {
                    // Update path from typed text; is_toml will be resolved on save.
                    self.path = if self.path_str.trim().is_empty() {
                        None
                    } else {
                        Some(PathBuf::from(self.path_str.trim()))
                    };
                }
                ui.end_row();
            });
    }
}

pub struct PageSession {
    pub events: VecDeque<Event>,
    connected_config_file_id: Option<u64>,
    selected_config_file_id: u64,
    config_file_id_count: u64,
    config_files: BTreeMap<u64, ConfigFileData>,
    dnd_items: Vec<DndItem>,
    file_dialog: Option<NativeFileDialog>,
    pub session_info: Option<SessionInfoData>,
    session_info_timer: Option<Instant>,
}

impl Default for PageSession {
    fn default() -> Self {
        PageSession {
            events: VecDeque::new(),
            connected_config_file_id: None,
            selected_config_file_id: 0,
            config_file_id_count: 0,
            config_files: BTreeMap::new(),
            dnd_items: Vec::new(),
            file_dialog: None,
            session_info: None,
            session_info_timer: None,
        }
    }
}

impl From<&PageSession> for ArchivePageSession {
    fn from(value: &PageSession) -> Self {
        ArchivePageSession {
            config_files: value
                .dnd_items
                .iter()
                .filter_map(|k| value.config_files.get(&k.id))
                .map(|d| d.into())
                .collect(),
        }
    }
}

impl PageSession {
    pub fn load(&mut self, archive: ArchivePageSession) -> Result<(), String> {
        let mut data = Vec::with_capacity(archive.config_files.len());
        for d in archive.config_files {
            let config_file_data = ConfigFileData::try_from(d)?;
            data.push(config_file_data);
        }
        self.clean_all_config_file_data();
        for d in data {
            self.add_config_file(d);
        }
        Ok(())
    }

    pub fn show(&mut self, ctx: &Context) {
        SidePanel::left("page_session_panel_left")
            .resizable(true)
            .show(ctx, |ui| {
                self.show_config_file_list(ui);
            });

        CentralPanel::default().show(ctx, |ui| {
            if self.connected_config_file_id.is_some() {
                let should_refresh = self
                    .session_info_timer
                    .map(|t| t.elapsed() >= Duration::from_secs(2))
                    .unwrap_or(true);
                if should_refresh {
                    self.events.push_back(Event::RequestSessionInfo);
                    self.session_info_timer = Some(Instant::now());
                }
                show_session_info_panel(ui, self.session_info.as_ref());
                ui.separator();
            } else {
                self.session_info = None;
                self.session_info_timer = None;
            }

            if let Some(config_file_data) = self.config_files.get_mut(&self.selected_config_file_id) {
                if config_file_data.source.is_empty() && config_file_data.path.is_some() {
                    let path = config_file_data.path.clone().unwrap();
                    config_file_data.load_from_file(path);
                }
                config_file_data.show(ui, self.connected_config_file_id, &mut self.events);
            }
        });

        if let Some(dialog) = &self.file_dialog {
            if let Some(result) = dialog.try_recv() {
                if let Some(p) = result {
                    if let Ok(o) = p.canonicalize() {
                        let name = match o.file_stem() {
                            None => "new file".to_string(),
                            Some(s) => s.to_string_lossy().to_string(),
                        };
                        self.add_config_file(ConfigFileData::new(name, o));
                    }
                }
                self.file_dialog = None;
            }
        }
    }

    fn add_config_file(&mut self, mut config_file_data: ConfigFileData) {
        self.config_file_id_count += 1;
        let id = self.config_file_id_count;
        self.selected_config_file_id = id;
        config_file_data.id = id;
        self.config_files.insert(id, config_file_data);
        self.dnd_items.push(DndItem { id });
    }

    fn del_config_file(&mut self) {
        if self.config_files.len() < 2 {
            return;
        }
        let remove_id = self.selected_config_file_id;
        if let Some(id) = self.connected_config_file_id {
            if id == remove_id {
                return;
            }
        }
        let _ = self.config_files.remove(&remove_id);
        let mut del_index = None;
        for (i, di) in self.dnd_items.iter().enumerate() {
            if di.id == remove_id {
                del_index = Some(i);
                break;
            }
        }
        if let Some(i) = del_index {
            self.dnd_items.remove(i);
        }
    }

    pub fn connected(&self) -> bool {
        self.connected_config_file_id.is_some()
    }

    pub fn set_connected(&mut self, connected_id: Option<u64>) {
        self.connected_config_file_id = connected_id;
    }

    pub fn set_connect_result(&mut self, r: Result<u64, (u64, String)>) {
        match r {
            Ok(id) => {
                self.connected_config_file_id = Some(id);
                if let Some(cf) = self.config_files.get_mut(&id) {
                    cf.err_str = None;
                }
            }
            Err((id, s)) => {
                self.connected_config_file_id = None;
                if let Some(cf) = self.config_files.get_mut(&id) {
                    cf.err_str = Some(s);
                }
            }
        }
    }

    fn show_config_file_list(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            if ui
                .button(RichText::new(" + ").code())
                .on_hover_text("Open an existing config file")
                .clicked()
            {
                self.file_dialog = Some(NativeFileDialog::open(None));
            }

            if ui
                .button(RichText::new("new").code())
                .on_hover_text("Create a new config from a template")
                .clicked()
            {
                self.add_config_file(ConfigFileData::new_template());
            }

            if ui
                .button(RichText::new(" - ").code())
                .on_hover_text("Remove selected config")
                .clicked()
            {
                self.del_config_file();
            }
        });

        ui.add_space(10.0);

        ScrollArea::both()
            .max_width(200.0)
            .auto_shrink([true, false])
            .show(ui, |ui| {
                dnd(ui, "page_session_config_list").show_vec(
                    self.dnd_items.as_mut_slice(),
                    |ui, item, handle, _state| {
                        if let Some(d) = self.config_files.get(&item.id) {
                            let mut text = if let Some(id) = self.connected_config_file_id {
                                if id == item.id {
                                    RichText::new(d.name.as_str()).underline().strong()
                                } else {
                                    RichText::new(d.name.as_str())
                                }
                            } else {
                                RichText::new(d.name.as_str())
                            };
                            if d.is_dirty {
                                text = text.color(Color32::YELLOW);
                            }
                            handle.ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.selected_config_file_id,
                                    item.id,
                                    text,
                                );
                            });
                        }
                    },
                );
            });
    }

    fn clean_all_config_file_data(&mut self) {
        self.selected_config_file_id = 0;
        self.config_files.clear();
    }
}

#[derive(Hash)]
struct DndItem {
    id: u64,
}

#[derive(Eq, PartialEq, Copy, Clone)]
enum FilePage {
    Source,
    Format,
    Tree,
}

fn show_session_info_panel(ui: &mut Ui, info: Option<&SessionInfoData>) {
    CollapsingHeader::new("Session")
        .default_open(true)
        .show(ui, |ui| {
            let Some(info) = info else {
                ui.label(RichText::new("connecting...").italics());
                return;
            };
            Grid::new("session_info_grid")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label("own zid:");
                    ui.label(RichText::new(&info.zid).monospace());
                    ui.end_row();

                    ui.label("peers:");
                    ui.vertical(|ui| {
                        if info.peers.is_empty() {
                            ui.label(RichText::new("-").monospace());
                        }
                        for peer in &info.peers {
                            ui.label(RichText::new(peer).monospace());
                        }
                    });
                    ui.end_row();

                    ui.label("routers:");
                    ui.vertical(|ui| {
                        if info.routers.is_empty() {
                            ui.label(RichText::new("-").monospace());
                        }
                        for router in &info.routers {
                            ui.label(RichText::new(router).monospace());
                        }
                    });
                    ui.end_row();
                });
        });
}
