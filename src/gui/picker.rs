//! Engine picker window: a sortable, searchable table with a detail pane and keyboard navigation.

use eframe::egui::{self, Align2, Color32, FontId, Key, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use super::GuiApp;
use crate::engine::EngineCfg;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Col {
    Name,
    Displacement,
    Cylinders,
    Redline,
    Power,
    Torque,
}

impl Col {
    const ALL: [Col; 6] = [Col::Name, Col::Displacement, Col::Cylinders, Col::Redline, Col::Power, Col::Torque];

    fn title(self) -> &'static str {
        match self {
            Col::Name => "Name",
            Col::Displacement => "Displ.",
            Col::Cylinders => "Cyl.",
            Col::Redline => "Redline",
            Col::Power => "Power",
            Col::Torque => "Torque",
        }
    }

    fn width(self) -> f32 {
        match self {
            Col::Name => 250.0,
            Col::Displacement => 70.0,
            Col::Cylinders => 50.0,
            Col::Redline => 85.0,
            Col::Power => 80.0,
            Col::Torque => 90.0,
        }
    }

    fn cell(self, e: &EngineCfg) -> String {
        match self {
            Col::Name => e.name.to_string(),
            Col::Displacement => format!("{:.1} L", e.displacement() * 1e3),
            Col::Cylinders => e.cylinders.len().to_string(),
            Col::Redline => format!("{:.0} rpm", e.redline_rpm),
            Col::Power if e.power_kw > 0.0 => format!("{:.0} kW", e.power_kw),
            Col::Torque if e.torque_nm > 0.0 => format!("{:.0} N·m", e.torque_nm),
            _ => "-".to_string(),
        }
    }
}

/// "Inline-4", "V8 90°", "Flat-6".
fn layout_label(e: &EngineCfg) -> String {
    let n = e.cylinders.len();
    let banks = e.cylinders.iter().map(|c| c.bank).max().unwrap_or(0) + 1;
    match (banks, e.bank_angle_deg) {
        (1, _) => format!("Inline-{n}"),
        (_, a) if a >= 179.0 => format!("Flat-{n}"),
        (_, a) => format!("V{n} {a:.0}°"),
    }
}

impl GuiApp {
    pub(super) fn open_picker(&mut self) {
        self.picker_open = true;
        self.picker_cursor = self.shared.active_engine.load(std::sync::atomic::Ordering::Relaxed);
        self.picker_focus = true;
        self.picker_scroll = true;
        self.filter.clear();
    }

    /// Engine indices matching the search text, in the current sort order.
    fn picker_list(&self) -> Vec<usize> {
        let words: Vec<String> = self.filter.split_whitespace().map(str::to_lowercase).collect();
        let mut list: Vec<usize> = (0..self.presets.len())
            .filter(|&i| {
                let e = &self.presets[i];
                let hay = format!("{} {}cyl {} {:.1}l", e.name, e.cylinders.len(), layout_label(e), e.displacement() * 1e3).to_lowercase();
                words.iter().all(|w| hay.contains(w.as_str()))
            })
            .collect();
        list.sort_by(|&a, &b| {
            let (ea, eb) = (&self.presets[a], &self.presets[b]);
            let by_name = || ea.name.to_lowercase().cmp(&eb.name.to_lowercase());
            match self.picker_sort {
                Col::Name => by_name(),
                Col::Displacement => ea.displacement().total_cmp(&eb.displacement()).then_with(by_name),
                Col::Cylinders => ea.cylinders.len().cmp(&eb.cylinders.len()).then_with(by_name),
                Col::Redline => ea.redline_rpm.total_cmp(&eb.redline_rpm).then_with(by_name),
                Col::Power => ea.power_kw.total_cmp(&eb.power_kw).then_with(by_name),
                Col::Torque => ea.torque_nm.total_cmp(&eb.torque_nm).then_with(by_name),
            }
        });
        if self.picker_reverse {
            list.reverse();
        }
        list
    }

    pub(super) fn engine_picker(&mut self, ctx: &egui::Context) {
        if !self.picker_open {
            return;
        }
        let list = self.picker_list();
        if !list.contains(&self.picker_cursor) {
            self.picker_cursor = list.first().copied().unwrap_or(0);
        }

        let (up, down, enter, esc) =
            ctx.input(|i| (i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown), i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
        if let Some(pos) = list.iter().position(|&i| i == self.picker_cursor) {
            if down && pos + 1 < list.len() {
                self.picker_cursor = list[pos + 1];
                self.picker_scroll = true;
            }
            if up && pos > 0 {
                self.picker_cursor = list[pos - 1];
                self.picker_scroll = true;
            }
        }

        let mut open = true;
        let mut cancel = false;
        let mut confirm: Option<usize> = if enter && !list.is_empty() { Some(self.picker_cursor) } else { None };
        let active = self.shared.active_engine.load(std::sync::atomic::Ordering::Relaxed);
        let row_h = 24.0;
        let table_w: f32 = Col::ALL.iter().map(|c| c.width()).sum();
        // The window fills the screen height (with a margin), so the table gets all the room it can.
        let screen = ctx.content_rect();
        let detail_w = 280.0;
        let win_w = (table_w + detail_w + 70.0).min(screen.width() - 30.0);
        let win_h = (screen.height() - 70.0).max(240.0);
        let body_h = win_h - 96.0;

        egui::Window::new("Select an engine")
            .open(&mut open)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .fixed_size([win_w, win_h])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let edit = ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Search: name, 12cyl, v8, flat…").desired_width(340.0));
                    if self.picker_focus {
                        edit.request_focus();
                        self.picker_focus = false;
                    }
                    ui.weak(format!("{} of {} engines", list.len(), self.presets.len()));
                });
                ui.add_space(4.0);

                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(Vec2::new(table_w, body_h), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(table_w);
                        self.picker_header(ui, row_h);
                        egui::ScrollArea::vertical().max_height(body_h - row_h - 6.0).auto_shrink([false, false]).show(ui, |ui| {
                            for &i in &list {
                                let e = &self.presets[i];
                                let (rect, resp) = ui.allocate_exact_size(Vec2::new(table_w, row_h), Sense::click());
                                let painter = ui.painter();
                                let selected = i == self.picker_cursor;
                                if selected {
                                    painter.rect_filled(rect, 3.0, ui.visuals().selection.bg_fill);
                                } else if resp.hovered() {
                                    painter.rect_filled(rect, 3.0, ui.visuals().widgets.hovered.weak_bg_fill);
                                }
                                if i == active {
                                    painter.circle_filled(Pos2::new(rect.left() + 8.0, rect.center().y), 3.5, Color32::from_rgb(90, 220, 120));
                                }
                                let mut x = rect.left();
                                for col in Col::ALL {
                                    let text = col.cell(e);
                                    let color = if selected { Color32::WHITE } else { ui.visuals().text_color() };
                                    if col == Col::Name {
                                        painter.text(Pos2::new(x + 20.0, rect.center().y), Align2::LEFT_CENTER, text, FontId::proportional(14.0), color);
                                    } else {
                                        painter.text(Pos2::new(x + col.width() - 10.0, rect.center().y), Align2::RIGHT_CENTER, text, FontId::proportional(14.0), color);
                                    }
                                    x += col.width();
                                }
                                if resp.clicked() {
                                    self.picker_cursor = i;
                                }
                                if resp.double_clicked() {
                                    confirm = Some(i);
                                }
                                if selected && self.picker_scroll {
                                    ui.scroll_to_rect(rect, None);
                                }
                            }
                            if list.is_empty() {
                                ui.add_space(20.0);
                                ui.weak("No engine matches the search.");
                            }
                        });
                        self.picker_scroll = false;
                    });
                    ui.add_space(8.0);
                    ui.allocate_ui_with_layout(Vec2::new(detail_w, body_h), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(detail_w);
                        if let Some(e) = list.iter().find(|&&i| i == self.picker_cursor).map(|&i| &self.presets[i]) {
                            Self::picker_details(ui, e);
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                if ui.button(RichText::new("Select").strong()).clicked() {
                                    confirm = Some(self.picker_cursor);
                                }
                                if ui.button("Cancel").clicked() {
                                    cancel = true;
                                }
                            });
                        }
                    });
                });
                ui.add_space(2.0);
                ui.weak("Click a column title to sort · Up / Down to move · Enter or double-click to select · Esc to close");
            });

        if let Some(i) = confirm {
            self.select_engine(i);
            open = false;
        }
        if esc || cancel {
            open = false;
        }
        if !open {
            self.picker_open = false;
            self.filter.clear();
        }
    }

    /// Clickable column titles; the sorted column is highlighted with a `^` / `v` marker.
    fn picker_header(&mut self, ui: &mut egui::Ui, row_h: f32) {
        let total: f32 = Col::ALL.iter().map(|c| c.width()).sum();
        let (rect, _) = ui.allocate_exact_size(Vec2::new(total, row_h), Sense::hover());
        ui.painter().rect_filled(rect, 3.0, ui.visuals().faint_bg_color);
        let mut x = rect.left();
        for col in Col::ALL {
            let cell = Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(col.width(), row_h));
            let resp = ui.interact(cell, egui::Id::unique(("picker-col", col.title())), Sense::click());
            let sorted = self.picker_sort == col;
            let marker = if !sorted { "" } else if self.picker_reverse { " v" } else { " ^" };
            let color = if sorted { Color32::from_rgb(250, 210, 60) } else { ui.visuals().strong_text_color() };
            let text = format!("{}{marker}", col.title());
            if col == Col::Name {
                ui.painter().text(Pos2::new(cell.left() + 20.0, cell.center().y), Align2::LEFT_CENTER, text, FontId::proportional(14.0), color);
            } else {
                ui.painter().text(Pos2::new(cell.right() - 10.0, cell.center().y), Align2::RIGHT_CENTER, text, FontId::proportional(14.0), color);
            }
            if resp.hovered() {
                ui.painter().rect_stroke(cell.shrink(1.0), 3.0, Stroke::new(1.0, Color32::GRAY), egui::StrokeKind::Inside);
            }
            if resp.clicked() {
                if sorted {
                    self.picker_reverse = !self.picker_reverse;
                } else {
                    self.picker_sort = col;
                    self.picker_reverse = false;
                }
                self.picker_scroll = true;
            }
            x += col.width();
        }
    }

    fn picker_details(ui: &mut egui::Ui, e: &EngineCfg) {
        ui.label(RichText::new(e.name).heading());
        ui.label(RichText::new(layout_label(e)).strong().color(Color32::from_rgb(250, 210, 60)));
        ui.add_space(4.0);
        let row = |ui: &mut egui::Ui, k: &str, v: String| {
            ui.horizontal(|ui| {
                ui.weak(format!("{k}:"));
                ui.label(v);
            });
        };
        row(ui, "Displacement", format!("{:.2} L", e.displacement() * 1e3));
        row(ui, "Bore × stroke", format!("{:.1} × {:.1} mm", e.bore * 1e3, e.stroke * 1e3));
        row(ui, "Compression", format!("{:.1}:1", e.compression_ratio));
        row(ui, "Idle / redline", format!("{:.0} / {:.0} rpm", e.idle_rpm, e.redline_rpm));
        if e.power_kw > 0.0 {
            row(ui, "Power", format!("{:.0} kW ({:.0} hp)", e.power_kw, e.power_kw * 1.341));
        }
        if e.torque_nm > 0.0 {
            row(ui, "Torque", format!("{:.0} N·m", e.torque_nm));
        }
        let gb = &e.gearbox;
        row(ui, "Gearbox", format!("{} speeds, FD {:.2}", gb.ratios.len(), gb.final_drive));
        row(ui, "Vehicle mass", format!("{:.0} kg", gb.vehicle_mass));
        let mut extras = Vec::new();
        if let Some(t) = &e.turbo {
            extras.push(format!("Forced induction (up to {:.1} bar abs.)", t.max_boost));
        }
        if let Some(el) = &e.electric {
            extras.push(format!("Electric assist {:.0} kW", el.power_kw));
        }
        if e.anti_lag {
            extras.push("Anti-lag".to_string());
        }
        if gb.traction_control {
            extras.push("Traction control".to_string());
        }
        if !extras.is_empty() {
            ui.add_space(4.0);
            for line in extras {
                ui.label(format!("• {line}"));
            }
        }
    }
}
