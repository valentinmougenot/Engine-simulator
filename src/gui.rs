//! Graphical interface (egui): same controls as the TUI (through `Pilot`), with
//! vector-drawn engine views.

use std::f64::consts::PI;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use eframe::egui::{self, Align2, Color32, FontId, Key, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};
use ratatui::style::Color;

use crate::ambience::Mic;
use crate::app::Shared;
use crate::engine::EngineCfg;
use crate::pilot::Pilot;
use crate::view::cylinder_state;

mod picker;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Scope,
    Cylinders,
    Top,
    Front,
}

struct GuiApp {
    shared: Arc<Shared>,
    presets: Vec<EngineCfg>,
    pilot: Pilot,
    tab: Tab,
    vis_theta: f64,
    notice: String,
    filter: String,
    rpm_history: Vec<f32>,
    history_clock: f64,
    brake_by_key: bool,
    /// Brake held with the mouse during the previous frame.
    pointer_brake: bool,
    picker_open: bool,
    /// Engine highlighted in the picker (index into `presets`).
    picker_cursor: usize,
    picker_sort: picker::Col,
    picker_reverse: bool,
    picker_focus: bool,
    picker_scroll: bool,
    show_help: bool,
}

pub fn run(presets: Vec<EngineCfg>, shared: Arc<Shared>) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 780.0]).with_title("engine-sim-rs"),
        ..Default::default()
    };
    eframe::run_native(
        "engine-sim-rs",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(GuiApp {
                shared,
                presets,
                pilot: Pilot::default(),
                // ENGINE_SIM_TAB=0..3 selects the starting tab (handy for screenshots).
                tab: match std::env::var("ENGINE_SIM_TAB").ok().as_deref() {
                    Some("0") => Tab::Scope,
                    Some("2") => Tab::Top,
                    Some("3") => Tab::Front,
                    _ => Tab::Cylinders,
                },
                vis_theta: 0.0,
                notice: String::new(),
                filter: String::new(),
                rpm_history: Vec::new(),
                history_clock: 0.0,
                brake_by_key: false,
                pointer_brake: false,
                // ENGINE_SIM_PICKER=1 opens the engine picker at startup (handy for screenshots).
                picker_open: std::env::var_os("ENGINE_SIM_PICKER").is_some(),
                picker_cursor: 0,
                picker_sort: picker::Col::Name,
                picker_reverse: false,
                picker_focus: false,
                picker_scroll: false,
                show_help: std::env::var_os("ENGINE_SIM_HELP").is_some(),
            }))
        }),
    )
}

/// Upper-cases the first letter.
fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

fn rgb(c: Color) -> Color32 {
    match c {
        Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
        _ => Color32::GRAY,
    }
}

const PALETTE: [Color32; 8] = [
    Color32::from_rgb(110, 220, 255),
    Color32::from_rgb(120, 230, 130),
    Color32::from_rgb(230, 130, 230),
    Color32::from_rgb(120, 150, 255),
    Color32::from_rgb(235, 235, 235),
    Color32::from_rgb(60, 180, 200),
    Color32::from_rgb(70, 170, 90),
    Color32::from_rgb(180, 90, 180),
];

impl GuiApp {
    fn engine(&self) -> &EngineCfg {
        &self.presets[self.shared.active_engine.load(Ordering::Relaxed).min(self.presets.len() - 1)]
    }

    fn reload(&mut self) {
        let (list, errors) = crate::config::load_all();
        let active_id = self.engine().id();
        let index = list.iter().position(|e| e.id() == active_id).unwrap_or(0);
        self.notice = match errors.first() {
            Some(e) => format!("{} error(s): {e}", errors.len()),
            None => format!("{} engines reloaded", list.len()),
        };
        self.presets = list;
        self.shared.engine_index.store(index, Ordering::Relaxed);
        self.shared.reload_gen.fetch_add(1, Ordering::Relaxed);
        self.pilot.reset(&self.shared);
        self.rpm_history.clear();
    }

    fn select_engine(&mut self, index: usize) {
        if index != self.shared.active_engine.load(Ordering::Relaxed) {
            self.shared.engine_index.store(index, Ordering::Relaxed);
            self.pilot.reset(&self.shared);
            self.rpm_history.clear();
        }
    }

    /// A window sees key releases: the brake follows the key or click while held.
    fn update_brake(&mut self, space_down: bool, s: &Shared) {
        let wanted = space_down || self.pointer_brake;
        self.pointer_brake = false;
        if wanted {
            self.brake_by_key = true;
            self.pilot.set_brake(true, s);
        } else if self.brake_by_key {
            self.brake_by_key = false;
            self.pilot.set_brake(false, s);
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let s = Arc::clone(&self.shared);
        let gears = self.engine().gearbox.ratios.len() as i32;
        let idle_default = self.engine().idle_rpm;
        // Shortcuts are suspended while typing in a text field (filter).
        if self.picker_open || ctx.egui_wants_keyboard_input() {
            self.update_brake(false, &s);
            return;
        }
        let thr = self.pilot.throttle;
        let mut next_tab = false;
        let mut prev_tab = false;
        let mut toggle_help = false;
        let mut open_picker = false;
        let mut close_help = false;
        let mut reload = false;
        let mut quit = false;
        let space_down = ctx.input(|i| {
            if i.key_pressed(Key::ArrowUp) {
                self.pilot.set_throttle(thr + 0.05, &s);
            }
            if i.key_pressed(Key::ArrowDown) {
                self.pilot.set_throttle(thr - 0.05, &s);
            }
            if i.key_pressed(Key::F) {
                self.pilot.set_throttle(1.0, &s);
            }
            if i.key_pressed(Key::X) {
                self.pilot.set_throttle(0.0, &s);
            }
            for (k, v) in [
                (Key::Num0, 0.0),
                (Key::Num1, 0.1),
                (Key::Num2, 0.2),
                (Key::Num3, 0.3),
                (Key::Num4, 0.4),
                (Key::Num5, 0.5),
                (Key::Num6, 0.6),
                (Key::Num7, 0.7),
                (Key::Num8, 0.8),
                (Key::Num9, 0.9),
            ] {
                if i.key_pressed(k) {
                    self.pilot.set_throttle(v, &s);
                }
            }
            if i.key_pressed(Key::A) || i.key_pressed(Key::ArrowRight) {
                self.pilot.shift(1, gears, &s);
            }
            if i.key_pressed(Key::Z) || i.key_pressed(Key::ArrowLeft) {
                self.pilot.shift(-1, gears, &s);
            }
            if i.key_pressed(Key::N) {
                self.pilot.neutral(&s);
            }
            if i.key_pressed(Key::C) {
                self.pilot.toggle_clutch();
            }
            if i.key_pressed(Key::Comma) {
                crate::pilot::adjust_idle(&s, idle_default, -crate::pilot::IDLE_STEP);
            }
            if i.key_pressed(Key::Period) || i.key_pressed(Key::Semicolon) {
                crate::pilot::adjust_idle(&s, idle_default, crate::pilot::IDLE_STEP);
            }
            if i.key_pressed(Key::S) {
                s.starter.store(true, Ordering::Relaxed);
            }
            if i.key_pressed(Key::I) {
                let on = !s.ignition.load(Ordering::Relaxed);
                s.ignition.store(on, Ordering::Relaxed);
            }
            if i.key_pressed(Key::OpenBracket) {
                s.volume.set((s.volume.get() - 0.05).max(0.0));
            }
            if i.key_pressed(Key::CloseBracket) {
                s.volume.set((s.volume.get() + 0.05).min(1.5));
            }
            if i.key_pressed(Key::Minus) {
                s.convolution.set((s.convolution.get() - 0.1).max(0.0));
            }
            if i.key_pressed(Key::Equals) || i.key_pressed(Key::Plus) {
                s.convolution.set((s.convolution.get() + 0.1).min(1.0));
            }
            if i.key_pressed(Key::M) {
                open_picker = true;
            }
            if i.key_pressed(Key::V) {
                s.mic.store((s.mic.load(Ordering::Relaxed) + 1) % Mic::ALL.len(), Ordering::Relaxed);
            }
            if i.key_pressed(Key::T) {
                self.pilot.cycle_anim();
            }
            if i.key_pressed(Key::Questionmark) || i.key_pressed(Key::F1) || i.key_pressed(Key::H) {
                toggle_help = true;
            }
            if i.key_pressed(Key::Escape) {
                close_help = true;
            }
            if i.key_pressed(Key::E) {
                if i.modifiers.shift {
                    prev_tab = true;
                } else {
                    next_tab = true;
                }
            }
            reload = i.key_pressed(Key::R);
            quit = i.key_pressed(Key::Q);
            i.key_down(Key::Space)
        });
        self.update_brake(space_down, &s);
        if open_picker {
            self.open_picker();
        }
        if toggle_help {
            self.show_help = !self.show_help;
        }
        if close_help {
            self.show_help = false;
        }
        if next_tab {
            self.tab = match self.tab {
                Tab::Scope => Tab::Cylinders,
                Tab::Cylinders => Tab::Top,
                Tab::Top => Tab::Front,
                Tab::Front => Tab::Scope,
            };
        }
        if prev_tab {
            self.tab = match self.tab {
                Tab::Scope => Tab::Front,
                Tab::Cylinders => Tab::Scope,
                Tab::Top => Tab::Cylinders,
                Tab::Front => Tab::Top,
            };
        }
        if reload {
            self.reload();
        }
        if quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        let s = Arc::clone(&self.shared);
        ui.horizontal_wrapped(|ui| {
            let active = s.active_engine.load(Ordering::Relaxed);
            let name = self.presets[active.min(self.presets.len() - 1)].name;
            if ui.button(RichText::new(format!("Engine: {name}")).strong()).on_hover_text("Choose an engine (M)").clicked() {
                self.open_picker();
            }

            let nominal = &self.presets[active.min(self.presets.len() - 1)];
            if nominal.power_kw > 0.0 {
                ui.label(
                    RichText::new(format!("{:.0} kW ({:.0} hp) · {:.0} N·m", nominal.power_kw, nominal.power_kw * 1.341, nominal.torque_nm)).strong(),
                );
            }

            let mic = Mic::from_index(s.mic.load(Ordering::Relaxed));
            egui::ComboBox::from_id_salt("mic")
                .selected_text(format!("Mic: {}", cap(mic.name())))
                .show_ui(ui, |ui| {
                    for (k, m) in Mic::ALL.into_iter().enumerate() {
                        if ui.selectable_label(m == mic, cap(m.name())).clicked() {
                            s.mic.store(k, Ordering::Relaxed);
                        }
                    }
                });

            let mut vol = s.volume.get();
            if ui.add(egui::Slider::new(&mut vol, 0.0..=1.5).text("Volume")).changed() {
                s.volume.set(vol);
            }
            let mut conv = s.convolution.get();
            if ui.add(egui::Slider::new(&mut conv, 0.0..=1.0).text("Convolution")).changed() {
                s.convolution.set(conv);
            }

            let ignition = s.ignition.load(Ordering::Relaxed);
            if ui.selectable_label(ignition, "Ignition").clicked() {
                s.ignition.store(!ignition, Ordering::Relaxed);
            }
            if ui.button("Starter").clicked() {
                s.starter.store(true, Ordering::Relaxed);
            }
            if ui.button("Reload TOML").clicked() {
                self.reload();
            }
            if ui.button("?").on_hover_text("Keyboard shortcuts").clicked() {
                self.show_help = !self.show_help;
            }
            if !self.notice.is_empty() {
                ui.colored_label(Color32::YELLOW, &self.notice);
            }
        });
    }

    /// Vertical bar filled bottom to top; returns the response for interaction.
    fn pedal_bar(ui: &mut egui::Ui, label: &str, value: f32, color: Color32) -> egui::Response {
        ui.vertical(|ui| {
            let (resp, painter) = ui.allocate_painter(Vec2::new(78.0, 120.0), Sense::click_and_drag());
            let r = resp.rect;
            painter.rect_filled(r, 4.0, Color32::from_gray(32));
            let h = r.height() * value.clamp(0.0, 1.0);
            painter.rect_filled(Rect::from_min_max(Pos2::new(r.left(), r.bottom() - h), r.max), 4.0, color);
            painter.rect_stroke(r, 4.0, Stroke::new(1.0, Color32::from_gray(90)), StrokeKind::Inside);
            painter.text(r.center_top() + Vec2::new(0.0, 6.0), Align2::CENTER_TOP, format!("{:.0} %", value * 100.0), FontId::proportional(14.0), Color32::WHITE);
            ui.add_sized([78.0, 16.0], egui::Label::new(label));
            resp
        })
        .inner
    }

    fn pedals(&mut self, ui: &mut egui::Ui) {
        let s = Arc::clone(&self.shared);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let gas = Self::pedal_bar(ui, "Throttle", self.pilot.throttle, Color32::from_rgb(70, 190, 100));
            if gas.dragged() || gas.clicked() {
                if let Some(pos) = gas.interact_pointer_pos() {
                    let v = 1.0 - (pos.y - gas.rect.top()) / gas.rect.height();
                    self.pilot.set_throttle(v, &s);
                }
            }
            let brake = Self::pedal_bar(ui, "Brake", if self.pilot.brake_on { 1.0 } else { 0.0 }, Color32::from_rgb(210, 60, 60));
            if brake.is_pointer_button_down_on() {
                self.pointer_brake = true;
            }
            let clutch = Self::pedal_bar(ui, "Clutch", 1.0 - s.clutch_engagement.get(), Color32::from_rgb(70, 160, 220));
            if clutch.clicked() {
                self.pilot.toggle_clutch();
            }
        });
        ui.small("Drag the throttle · Hold the brake · Click the clutch");
    }

    fn left_panel(&mut self, ui: &mut egui::Ui) {
        let s = Arc::clone(&self.shared);
        let cfg_redline = self.engine().redline_rpm;
        let default_idle = self.engine().idle_rpm;
        let gears = self.engine().gearbox.ratios.len() as i32;
        let gear = s.gear.load(Ordering::Relaxed);

        egui::Panel::bottom("pedals").show(ui, |ui| self.pedals(ui));

        ui.vertical_centered(|ui| {
            let txt = if gear == 0 { "N".to_string() } else { gear.to_string() };
            ui.label(RichText::new(txt).size(110.0).strong().color(Color32::from_rgb(250, 210, 60)));
            // Always present (even if empty), otherwise the layout jumps during a gear change.
            let pending = if self.pilot.gear_target != gear {
                format!("Requested gear: {}", if self.pilot.gear_target == 0 { "N".to_string() } else { self.pilot.gear_target.to_string() })
            } else {
                String::new()
            };
            ui.add_sized([ui.available_width(), 18.0], egui::Label::new(RichText::new(pending).color(Color32::GRAY)));
            ui.label(RichText::new(format!("{:.0} km/h", s.speed_kmh.get())).size(28.0));
        });
        ui.add_space(8.0);

        let (resp, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), 190.0), Sense::hover());
        let rect = resp.rect;
        let center = Pos2::new(rect.center().x, rect.bottom() - 18.0);
        let radius = (rect.width() / 2.0 - 12.0).min(rect.height() - 24.0);
        let rpm = s.rpm.get() as f64;
        let max_rpm = (cfg_redline * 1.1).ceil();
        let angle_of = |r: f64| PI + (r / max_rpm).clamp(0.0, 1.0) * PI;
        let at = |a: f64, k: f32| Pos2::new(center.x + (a.cos() as f32) * radius * k, center.y + (a.sin() as f32) * radius * k);
        let steps = 90;
        for i in 0..steps {
            let (a0, a1) = (angle_of(max_rpm * i as f64 / steps as f64), angle_of(max_rpm * (i + 1) as f64 / steps as f64));
            let r_mid = max_rpm * (i as f64 + 0.5) / steps as f64;
            let color = if r_mid > cfg_redline { Color32::from_rgb(200, 50, 50) } else { Color32::from_rgb(60, 70, 90) };
            painter.line_segment([at(a0, 1.0), at(a1, 1.0)], Stroke::new(6.0, color));
        }
        let thousand = if max_rpm > 3000.0 { 1000.0 } else { 500.0 };
        let mut t = 0.0;
        while t <= max_rpm {
            let a = angle_of(t);
            painter.line_segment([at(a, 0.88), at(a, 0.97)], Stroke::new(1.5, Color32::GRAY));
            painter.text(at(a, 0.74), Align2::CENTER_CENTER, format!("{:.0}", t / 1000.0), FontId::proportional(13.0), Color32::LIGHT_GRAY);
            t += thousand;
        }
        let needle = angle_of(rpm);
        painter.line_segment([center, at(needle, 0.9)], Stroke::new(3.0, Color32::from_rgb(255, 120, 60)));
        painter.circle_filled(center, 6.0, Color32::from_rgb(255, 120, 60));
        painter.text(Pos2::new(center.x, center.y - radius * 0.35), Align2::CENTER_CENTER, format!("{rpm:.0}"), FontId::proportional(26.0), Color32::WHITE);
        painter.text(Pos2::new(center.x, center.y - radius * 0.18), Align2::CENTER_CENTER, "rpm", FontId::proportional(12.0), Color32::GRAY);

        ui.add_space(6.0);
        // Gears (wraps if the engine has many).
        ui.horizontal_wrapped(|ui| {
            ui.label("Gear");
            if ui.button("−").clicked() {
                self.pilot.shift(-1, gears, &s);
            }
            if ui.selectable_label(self.pilot.gear_target == 0, "N").clicked() {
                self.pilot.neutral(&s);
            }
            for g in 1..=gears {
                if ui.selectable_label(self.pilot.gear_target == g, g.to_string()).clicked() {
                    self.pilot.shift(g - self.pilot.gear_target, gears, &s);
                }
            }
            if ui.button("+").clicked() {
                self.pilot.shift(1, gears, &s);
            }
        });

        let (lo, hi) = crate::pilot::idle_range(default_idle);
        let mut idle = s.idle_rpm.get();
        if ui.add(egui::Slider::new(&mut idle, lo..=hi).step_by(10.0).suffix(" rpm").text("Idle speed")).changed() {
            s.idle_rpm.set(idle);
        }
        // Engine braking on overrun (clearly negative torque) is shown in red; avoid "-0".
        let zero = |v: f32| if v.abs() < 0.5 { 0.0 } else { v };
        let braking = s.torque.get() < -3.0;
        let tone = if braking { Color32::from_rgb(230, 80, 80) } else { ui.visuals().text_color() };
        ui.label(
            RichText::new(format!(
                "Torque: {:.0} N·m   Power: {:.0} kW ({:.0} hp){}",
                zero(s.torque.get()),
                zero(s.power.get()),
                zero(s.power.get() * 1.341),
                if braking { "  braking" } else { "" }
            ))
            .color(tone),
        );
        ui.label(format!("Actual throttle: {:.0} %", s.throttle_effective.get() * 100.0));
        ui.label(format!("Leveler gain: {:.4}", s.leveler_gain.get()));
        let rpm_now = s.rpm.get() as f64;
        ui.horizontal(|ui| {
            egui::ComboBox::from_label("Animation")
                .selected_text(crate::pilot::ANIM_MODES[self.pilot.anim_mode].0)
                .show_ui(ui, |ui| {
                    for (k, (name, _)) in crate::pilot::ANIM_MODES.iter().enumerate() {
                        ui.selectable_value(&mut self.pilot.anim_mode, k, *name);
                    }
                });
            ui.weak(crate::pilot::anim_label(self.pilot.anim_mode, rpm_now, cfg_redline));
        });
    }

    fn central(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (t, name) in [(Tab::Scope, "Oscilloscope"), (Tab::Cylinders, "Cylinders"), (Tab::Top, "Top"), (Tab::Front, "Front")] {
                ui.selectable_value(&mut self.tab, t, name);
            }
        });
        let rect = ui.available_rect_before_wrap();
        let (resp, painter) = ui.allocate_painter(rect.size(), Sense::hover());
        let rect = resp.rect.shrink(4.0);
        painter.rect_filled(resp.rect, 4.0, Color32::from_rgb(16, 18, 24));
        match self.tab {
            Tab::Scope => self.draw_scope(&painter, rect),
            Tab::Cylinders => {
                let (top, bottom) = split_v(rect, 0.64);
                self.draw_cylinders(&painter, top);
                self.draw_pv(&painter, bottom);
            }
            Tab::Top => self.draw_top(&painter, rect),
            Tab::Front => self.draw_front(&painter, rect),
        }
    }

    fn firing_load(&self) -> (bool, f64) {
        (self.shared.firing.load(Ordering::Relaxed), self.shared.throttle_effective.get() as f64)
    }

    fn draw_scope(&self, p: &egui::Painter, rect: Rect) {
        let (scope_rect, hist_rect) = split_v(rect, 0.65);
        p.rect_stroke(scope_rect, 2.0, Stroke::new(1.0, Color32::from_gray(60)), StrokeKind::Inside);
        if let Ok(v) = self.shared.scope.try_lock() {
            let n = v.len().max(2) as f32;
            let pts: Vec<Pos2> = v
                .iter()
                .enumerate()
                .map(|(i, &y)| Pos2::new(scope_rect.left() + scope_rect.width() * i as f32 / n, scope_rect.center().y - y.clamp(-1.0, 1.0) * scope_rect.height() / 2.0))
                .collect();
            p.add(egui::Shape::line(pts, Stroke::new(1.5, Color32::from_rgb(90, 200, 255))));
        }
        p.rect_stroke(hist_rect, 2.0, Stroke::new(1.0, Color32::from_gray(60)), StrokeKind::Inside);
        p.text(hist_rect.left_top() + Vec2::new(6.0, 4.0), Align2::LEFT_TOP, "RPM history", FontId::proportional(12.0), Color32::GRAY);
        let max = self.engine().redline_rpm as f32;
        let n = self.rpm_history.len().max(2) as f32;
        let pts: Vec<Pos2> = self
            .rpm_history
            .iter()
            .enumerate()
            .map(|(i, &r)| Pos2::new(hist_rect.left() + hist_rect.width() * i as f32 / n, hist_rect.bottom() - hist_rect.height() * (r / max).clamp(0.0, 1.0)))
            .collect();
        p.add(egui::Shape::line(pts, Stroke::new(1.5, Color32::from_rgb(250, 210, 60))));
    }

    /// Cylinders in section, to scale, with the real crank-slider kinematics.
    fn draw_cylinders(&self, p: &egui::Painter, rect: Rect) {
        let cfg = self.engine();
        let (firing, load) = self.firing_load();
        let n = cfg.cylinders.len();
        let banks = cfg.cylinders.iter().map(|c| c.bank).max().unwrap_or(0) + 1;
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by_key(|&i| (cfg.cylinders[i].bank, i));
        let per_bank: Vec<Vec<usize>> = (0..banks).map(|b| order.iter().copied().filter(|&i| cfg.cylinders[i].bank == b).collect()).collect();
        // One row per bank; a very long bank is split over two rows.
        let mut rows: Vec<Vec<usize>> = Vec::new();
        let max_cols = (rect.width() / 70.0).max(1.0) as usize;
        for bank in &per_bank {
            for chunk in bank.chunks(max_cols.max(1)) {
                rows.push(chunk.to_vec());
            }
        }
        let row_h = rect.height() / rows.len().max(1) as f32;
        let a = cfg.stroke / 2.0;
        let l = cfg.rod_length;
        let half_h = 0.25 * cfg.bore;
        let reach = l + a + half_h + 0.05 * cfg.bore;
        for (r, row) in rows.iter().enumerate() {
            let cell_w = (rect.width() / row.len().max(1) as f32).min(150.0);
            let x0 = rect.center().x - cell_w * row.len() as f32 / 2.0;
            let top = rect.top() + row_h * r as f32;
            // Scale: cylinder + crankshaft height fits in the row (room left for the label).
            let k = ((row_h - 26.0) / (reach + a) as f32).min(cell_w * 0.75 / (cfg.bore as f32 * 1.15));
            for (c, &i) in row.iter().enumerate() {
                let cx = x0 + cell_w * (c as f32 + 0.5);
                let crank_y = top + 8.0 + reach as f32 * k;
                let (phi, stroke, burning, gas) = cylinder_state(cfg, i, self.vis_theta, firing, load);
                let ph = phi.rem_euclid(2.0 * PI);
                let xp = a * ph.cos() + (l * l - a * a * ph.sin().powi(2)).sqrt();
                let head_y = crank_y - ((l + a + half_h + 0.05 * cfg.bore) as f32) * k;
                let bore_w = cfg.bore as f32 * k;
                let pin_y = crank_y - xp as f32 * k;
                let crown_y = pin_y - half_h as f32 * k;
                let col = PALETTE[i % PALETTE.len()];
                let active = stroke == "PWR";
                let chamber = Rect::from_min_max(Pos2::new(cx - bore_w / 2.0, head_y), Pos2::new(cx + bore_w / 2.0, crown_y.max(head_y)));
                p.rect_filled(chamber, 0.0, rgb(gas).gamma_multiply(if burning { 0.95 } else { 0.55 }));
                p.line_segment([Pos2::new(cx - bore_w / 2.0, head_y), Pos2::new(cx - bore_w / 2.0, crank_y - ((l - a - half_h) as f32) * k)], Stroke::new(2.0, Color32::GRAY));
                p.line_segment([Pos2::new(cx + bore_w / 2.0, head_y), Pos2::new(cx + bore_w / 2.0, crank_y - ((l - a - half_h) as f32) * k)], Stroke::new(2.0, Color32::GRAY));
                p.line_segment([Pos2::new(cx - bore_w / 2.0, head_y), Pos2::new(cx + bore_w / 2.0, head_y)], Stroke::new(3.0, Color32::GRAY));
                let (vi, ve) = cfg.valve_openings(phi);
                let valve = |x: f32, open: f64, color: Color32| {
                    let drop = 2.0 + open as f32 * 9.0;
                    p.line_segment([Pos2::new(x, head_y - 7.0), Pos2::new(x, head_y + drop)], Stroke::new(2.5, if open > 0.03 { color } else { Color32::DARK_GRAY }));
                };
                valve(cx - bore_w * 0.28, vi, Color32::from_rgb(90, 230, 110));
                valve(cx + bore_w * 0.28, ve, Color32::from_rgb(240, 90, 70));
                let spark = firing && load > 0.02 && (-30.0..8.0).contains(&phi.to_degrees());
                p.line_segment([Pos2::new(cx, head_y - 8.0), Pos2::new(cx, head_y + 4.0)], Stroke::new(2.0, if spark { Color32::YELLOW } else { Color32::DARK_GRAY }));
                let piston = Rect::from_center_size(Pos2::new(cx, pin_y), Vec2::new(bore_w * 0.96, 2.0 * half_h as f32 * k));
                p.rect_filled(piston, 2.0, if active { Color32::from_rgb(255, 235, 150) } else { Color32::from_gray(210) });
                let crank_r = a as f32 * k;
                let pin = Pos2::new(cx + crank_r * ph.sin() as f32, crank_y - crank_r * ph.cos() as f32);
                p.circle_stroke(Pos2::new(cx, crank_y), crank_r, Stroke::new(1.0, Color32::from_gray(80)));
                p.line_segment([Pos2::new(cx, crank_y), pin], Stroke::new(3.0, col));
                p.line_segment([pin, Pos2::new(cx, pin_y)], Stroke::new(2.5, col));
                p.circle_filled(pin, 4.0, col);
                p.circle_filled(Pos2::new(cx, crank_y), 3.0, Color32::GRAY);
                let label_col = if active { Color32::from_rgb(250, 210, 60) } else { Color32::GRAY };
                p.text(Pos2::new(cx, crank_y + crank_r + 8.0), Align2::CENTER_TOP, format!("{} {}", i + 1, stroke), FontId::proportional(12.0), label_col);
            }
        }
    }

    fn draw_pv(&self, p: &egui::Painter, rect: Rect) {
        p.rect_stroke(rect, 2.0, Stroke::new(1.0, Color32::from_gray(60)), StrokeKind::Inside);
        let pv = self.shared.pv.try_lock().map(|v| v.clone()).unwrap_or_default();
        let pts: Vec<(f32, f32)> = pv.iter().filter(|q| q[0] > 0.0).map(|q| (q[0] * 1e6, q[1] / 1e5)).collect();
        if pts.is_empty() {
            return;
        }
        let xmax = pts.iter().map(|q| q.0).fold(1.0, f32::max) * 1.05;
        let peak = pts.iter().map(|q| q.1).fold(0.0, f32::max);
        let ymax = peak.max(5.0) * 1.1;
        let plot = rect.shrink2(Vec2::new(14.0, 20.0));
        let to = |q: &(f32, f32)| Pos2::new(plot.left() + plot.width() * q.0 / xmax, plot.bottom() - plot.height() * q.1 / ymax);
        let line: Vec<Pos2> = pts.iter().map(to).collect();
        p.add(egui::Shape::line(line, Stroke::new(1.5, Color32::from_rgb(255, 110, 90))));
        p.text(rect.left_top() + Vec2::new(8.0, 4.0), Align2::LEFT_TOP, format!("P-V diagram (cylinder 1) · peak {peak:.0} bar"), FontId::proportional(12.0), Color32::GRAY);
        p.text(rect.right_bottom() - Vec2::new(6.0, 4.0), Align2::RIGHT_BOTTOM, format!("{xmax:.0} cm³"), FontId::proportional(11.0), Color32::GRAY);
    }

    /// Top view: cylinders on either side of the crankshaft; the one on its power stroke is highlighted.
    fn draw_top(&self, p: &egui::Painter, rect: Rect) {
        let cfg = self.engine();
        let (firing, load) = self.firing_load();
        let n = cfg.cylinders.len();
        let banks = cfg.cylinders.iter().map(|c| c.bank.min(1)).max().unwrap_or(0) + 1;
        let per_bank: Vec<Vec<usize>> = (0..banks).map(|b| (0..n).filter(|&i| cfg.cylinders[i].bank.min(1) == b).collect()).collect();
        let cols = per_bank.iter().map(Vec::len).max().unwrap_or(1).max(1);
        let cell_w = (rect.width() / cols as f32).min(150.0);
        let x0 = rect.center().x - cell_w * cols as f32 / 2.0;
        let box_h = (rect.height() * 0.28).min(150.0);
        let axis_y = rect.center().y - 10.0;
        let mut active = Vec::new();
        for (b, bank) in per_bank.iter().enumerate() {
            for (c, &i) in bank.iter().enumerate() {
                let (phi, stroke, burning, gas) = cylinder_state(cfg, i, self.vis_theta, firing, load);
                let cx = x0 + cell_w * (c as f32 + 0.5) + if b == 1 { cell_w * 0.12 } else { 0.0 };
                let (y0, y1) = if b == 0 { (axis_y - 30.0 - box_h, axis_y - 30.0) } else { (axis_y + 30.0, axis_y + 30.0 + box_h) };
                let bx = Rect::from_min_max(Pos2::new(cx - cell_w * 0.4, y0), Pos2::new(cx + cell_w * 0.4, y1));
                let is_active = stroke == "PWR";
                if is_active {
                    active.push(i + 1);
                    p.rect_filled(bx.expand(6.0), 8.0, Color32::from_rgba_unmultiplied(255, 200, 60, if burning { 70 } else { 30 }));
                }
                p.rect_filled(bx, 4.0, rgb(gas).gamma_multiply(if burning { 1.0 } else { 0.5 }));
                p.rect_stroke(bx, 4.0, Stroke::new(if is_active { 3.0 } else { 1.5 }, if is_active { Color32::YELLOW } else { Color32::GRAY }), StrokeKind::Inside);
                p.text(bx.center(), Align2::CENTER_CENTER, format!("{}", i + 1), FontId::proportional(26.0), Color32::WHITE);
                p.text(bx.center_bottom() - Vec2::new(0.0, 6.0), Align2::CENTER_BOTTOM, stroke, FontId::proportional(13.0), if is_active { Color32::YELLOW } else { Color32::LIGHT_GRAY });
                let ang = if b == 0 { phi } else { phi + PI };
                let r = 16.0;
                let pin = Pos2::new(cx + r * ang.sin() as f32, axis_y - r * ang.cos() as f32);
                p.circle_stroke(Pos2::new(cx, axis_y), r, Stroke::new(1.0, Color32::from_gray(90)));
                p.line_segment([Pos2::new(cx, axis_y), pin], Stroke::new(2.5, Color32::from_rgb(100, 220, 255)));
                p.circle_filled(pin, 4.0, Color32::from_rgb(100, 220, 255));
            }
        }
        p.line_segment([Pos2::new(x0, axis_y), Pos2::new(x0 + cell_w * cols as f32, axis_y)], Stroke::new(2.0, Color32::from_rgb(60, 130, 160)));
        let shape = match (banks, cfg.bank_angle_deg) {
            (1, _) => format!("inline, {n} cylinders"),
            (_, a) if a >= 179.0 => format!("flat, {n} cylinders"),
            (_, a) => format!("V{n} at {a:.0}°"),
        };
        p.text(rect.left_top() + Vec2::new(8.0, 4.0), Align2::LEFT_TOP, format!("Top view — {shape}"), FontId::proportional(13.0), Color32::GRAY);
        let mut order: Vec<(f64, usize)> = cfg.cylinders.iter().enumerate().map(|(i, c)| (c.fire_deg, i + 1)).collect();
        order.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut x = rect.left() + 10.0;
        let y = rect.bottom() - 22.0;
        p.text(Pos2::new(x, y), Align2::LEFT_CENTER, "Firing order:", FontId::proportional(13.0), Color32::GRAY);
        x += 130.0;
        for (_, c) in &order {
            let r = Rect::from_center_size(Pos2::new(x + 12.0, y), Vec2::new(26.0, 20.0));
            let on = active.contains(c);
            p.rect_filled(r, 3.0, if on { Color32::from_rgb(250, 210, 60) } else { Color32::from_gray(45) });
            p.text(r.center(), Align2::CENTER_CENTER, c, FontId::proportional(13.0), if on { Color32::BLACK } else { Color32::LIGHT_GRAY });
            x += 30.0;
        }
    }

    /// Front view: crankshaft, rods and pistons of all cylinders, bank by bank.
    fn draw_front(&self, p: &egui::Painter, rect: Rect) {
        let cfg = self.engine();
        let (firing, load) = self.firing_load();
        let a = cfg.stroke / 2.0;
        let l = cfg.rod_length;
        let bore = cfg.bore;
        let half_h = 0.25 * bore;
        let reach = l + a + half_h + 0.05 * bore;
        // Inline or V engine: crankshaft at the bottom, cylinders upward; flat engine: centred.
        let flat = cfg.bank_angle_deg >= 150.0;
        let (scale, center) = if flat {
            let sc = (rect.width() / (2.3 * reach as f32)).min(rect.height() / (1.6 * a as f32 * 2.0 + 0.5 * reach as f32));
            (sc as f64, rect.center())
        } else {
            let sc = (rect.height() / (1.3 * reach as f32)).min(rect.width() / (1.8 * reach as f32));
            (sc as f64, Pos2::new(rect.center().x, rect.bottom() - 0.12 * rect.height() - (a as f32 * sc)))
        };
        let w = |x: f64, y: f64| Pos2::new(center.x + (x * scale) as f32, center.y - (y * scale) as f32);
        let banks = cfg.cylinders.iter().map(|c| c.bank.min(1)).max().unwrap_or(0) + 1;
        let angle = cfg.bank_angle_deg.to_radians();
        let dir = |b: usize| -> (f64, f64) {
            let t = if banks == 1 { 0.0 } else if b == 0 { -angle / 2.0 } else { angle / 2.0 };
            (t.sin(), t.cos())
        };
        for b in 0..banks {
            let (ux, uy) = dir(b);
            let (px, py) = (uy, -ux);
            let at = |d: f64, side: f64| w(ux * d + px * side * bore / 2.0, uy * d + py * side * bore / 2.0);
            let (d0, d1) = (l - a - half_h, l + a + half_h + 0.05 * bore);
            for side in [-1.0, 1.0] {
                p.line_segment([at(d0, side), at(d1, side)], Stroke::new(2.0, Color32::GRAY));
            }
            p.line_segment([at(d1, -1.0), at(d1, 1.0)], Stroke::new(3.0, Color32::GRAY));
        }
        p.circle_stroke(w(0.0, 0.0), (a * scale) as f32, Stroke::new(1.0, Color32::from_gray(80)));
        p.circle_filled(w(0.0, 0.0), (a * 0.3 * scale) as f32, Color32::GRAY);
        for (i, cyl) in cfg.cylinders.iter().enumerate() {
            let (ux, uy) = dir(cyl.bank.min(1));
            let (px, py) = (uy, -ux);
            let (phi, stroke, burning, gas) = cylinder_state(cfg, i, self.vis_theta, firing, load);
            let ph = phi.rem_euclid(2.0 * PI);
            let active = stroke == "PWR";
            let color = if active { Color32::from_rgb(255, 215, 70) } else { PALETTE[i % PALETTE.len()] };
            let crank = (a * (ph.cos() * ux + ph.sin() * px), a * (ph.cos() * uy + ph.sin() * py));
            let xp = a * ph.cos() + (l * l - a * a * ph.sin().powi(2)).sqrt();
            let pin = (xp * ux, xp * uy);
            p.line_segment([w(0.0, 0.0), w(crank.0, crank.1)], Stroke::new(4.0, color));
            p.line_segment([w(crank.0, crank.1), w(pin.0, pin.1)], Stroke::new(3.0, color));
            p.circle_filled(w(crank.0, crank.1), (a * 0.22 * scale) as f32, color);
            let head = l + a + half_h + 0.05 * bore;
            let crown = xp + half_h;
            if head > crown + 0.02 * bore && stroke != "EXH" {
                let quad = [
                    w(ux * crown - px * bore / 2.0, uy * crown - py * bore / 2.0),
                    w(ux * crown + px * bore / 2.0, uy * crown + py * bore / 2.0),
                    w(ux * head + px * bore / 2.0, uy * head + py * bore / 2.0),
                    w(ux * head - px * bore / 2.0, uy * head - py * bore / 2.0),
                ];
                p.add(egui::Shape::convex_polygon(quad.to_vec(), rgb(gas).gamma_multiply(if burning { 0.9 } else { 0.4 }), Stroke::NONE));
            }
            let corner = |du: f64, side: f64| w(pin.0 + ux * du * half_h + px * side * bore / 2.0, pin.1 + uy * du * half_h + py * side * bore / 2.0);
            let quad = vec![corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)];
            p.add(egui::Shape::convex_polygon(quad, color.gamma_multiply(0.35), Stroke::new(2.0, color)));
            let label = w(crank.0 * 1.4, crank.1 * 1.4);
            p.text(label, Align2::CENTER_CENTER, i + 1, FontId::proportional(13.0), color);
        }
        p.text(rect.left_top() + Vec2::new(8.0, 4.0), Align2::LEFT_TOP, "Front view — crankshaft, rods and pistons", FontId::proportional(13.0), Color32::GRAY);
    }
}

fn split_v(rect: Rect, ratio: f32) -> (Rect, Rect) {
    let h = rect.height() * ratio;
    (
        Rect::from_min_size(rect.min, Vec2::new(rect.width(), h - 3.0)),
        Rect::from_min_max(Pos2::new(rect.left(), rect.top() + h + 3.0), rect.max),
    )
}

impl eframe::App for GuiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let dt = ctx.input(|i| i.stable_dt) as f64;
        self.history_clock += dt;
        if self.history_clock > 0.05 {
            self.history_clock = 0.0;
            self.rpm_history.push(self.shared.rpm.get());
            if self.rpm_history.len() > 600 {
                self.rpm_history.remove(0);
            }
        }
        let rpm = self.shared.rpm.get() as f64;
        if rpm > 100.0 {
            let rev_per_s = crate::pilot::anim_rev_per_s(self.pilot.anim_mode, rpm, self.engine().redline_rpm);
            self.vis_theta = (self.vis_theta + rev_per_s * dt * 2.0 * PI).rem_euclid(4.0 * PI);
        }
        self.handle_keys(&ctx);
        self.pilot.sync(&self.shared);

        egui::Window::new("Keyboard shortcuts")
            .open(&mut self.show_help)
            .collapsible(false)
            .resizable(false)
            .show(&ctx, |ui| {
                egui::Grid::new("shortcuts").num_columns(2).spacing([24.0, 4.0]).show(ui, |ui| {
                    for (k, d) in crate::help::SHORTCUTS {
                        ui.label(RichText::new(*k).strong().color(Color32::from_rgb(250, 210, 60)));
                        ui.label(*d);
                        ui.end_row();
                    }
                });
            });
        self.engine_picker(&ctx);
        egui::Panel::top("header").show(ui, |ui| self.header(ui));
        egui::Panel::left("left").exact_size(330.0).show(ui, |ui| self.left_panel(ui));
        egui::CentralPanel::default().show(ui, |ui| self.central(ui));
        ctx.request_repaint_after(Duration::from_millis(16));
    }
}
