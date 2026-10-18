//! Terminal UI (ratatui): gauges, pedals, engaged gear, scope and engine views.

use std::f64::consts::PI;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Axis, Block, Chart, Clear, Dataset, Gauge, GraphType, List, ListItem, ListState, Paragraph, Sparkline,
};
use ratatui::{DefaultTerminal, Frame};

use crate::app::{Shared, SCOPE_POINTS};
use crate::pilot::Pilot;

mod mouse;
use mouse::{BarAction, Hit};
use crate::engine::EngineCfg;

const BIG_DIGITS: [[&str; 5]; 11] = [
    ["███", "█ █", "█ █", "█ █", "███"],
    [" █ ", "██ ", " █ ", " █ ", "███"],
    ["███", "  █", "███", "█  ", "███"],
    ["███", "  █", "███", "  █", "███"],
    ["█ █", "█ █", "███", "  █", "  █"],
    ["███", "█  ", "███", "  █", "███"],
    ["███", "█  ", "███", "█ █", "███"],
    ["███", "  █", "  █", "  █", "  █"],
    ["███", "█ █", "███", "█ █", "███"],
    ["███", "█ █", "███", "  █", "███"],
    ["█ █", "███", "███", "███", "█ █"],
];

/// Right-hand panel: scope or one of the engine views (`e` key).
#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Scope,
    Cylinders,
    Top,
    Front,
}

impl View {
    const ALL: [View; 4] = [View::Scope, View::Cylinders, View::Top, View::Front];

    fn title(self) -> &'static str {
        match self {
            View::Scope => "Scope",
            View::Cylinders => "Cylinders",
            View::Top => "Top",
            View::Front => "Front",
        }
    }

    fn prev(self) -> View {
        match self {
            View::Scope => View::Front,
            View::Cylinders => View::Scope,
            View::Top => View::Cylinders,
            View::Front => View::Top,
        }
    }

    fn next(self) -> View {
        match self {
            View::Scope => View::Cylinders,
            View::Cylinders => View::Top,
            View::Top => View::Front,
            View::Front => View::Scope,
        }
    }
}

/// Column the engine list is sorted by (like htop/btop process sorting).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SortKey {
    Name,
    Displacement,
    Cylinders,
    Redline,
    Power,
    Torque,
}

impl SortKey {
    const ALL: [SortKey; 6] =
        [SortKey::Name, SortKey::Displacement, SortKey::Cylinders, SortKey::Redline, SortKey::Power, SortKey::Torque];

    fn title(self) -> &'static str {
        match self {
            SortKey::Name => "Name",
            SortKey::Displacement => "Displ.",
            SortKey::Cylinders => "Cyl.",
            SortKey::Redline => "Redline",
            SortKey::Power => "Power",
            SortKey::Torque => "Torque",
        }
    }

    fn step(self, delta: isize) -> SortKey {
        let i = Self::ALL.iter().position(|k| *k == self).unwrap_or(0) as isize;
        Self::ALL[(i + delta).rem_euclid(Self::ALL.len() as isize) as usize]
    }
}

/// Engine list: filter text and position in the filtered list.
struct Menu {
    filter: String,
    cursor: usize,
    /// Filter being typed (after `/`); otherwise `j` `k` move like the arrow keys.
    typing: bool,
}

struct Ui {
    shared: Arc<Shared>,
    presets: Vec<EngineCfg>,
    pilot: Pilot,
    /// Engine list state while it is open.
    menu: Option<Menu>,
    /// Help window visible.
    help: bool,
    help_scroll: u16,
    help_filter: String,
    help_typing: bool,
    view: View,
    sort: SortKey,
    sort_reverse: bool,
    /// Clickable areas of the last draw.
    hit: Hit,
    /// Mouse: throttle gauge being dragged, brake held by the button.
    dragging_gas: bool,
    mouse_brake: bool,
    /// Displayed crank angle (rad, 0..4π); advances in slow motion, real speed would be unreadable.
    vis_theta: f64,
    last_frame: Instant,
    /// Last status message (reload result, errors).
    notice: String,
    rpm_history: Vec<u64>,
    last_history: Instant,
}

/// Gives the mouse back to the terminal on exit, even after an error or panic.
struct MouseGuard;

impl Drop for MouseGuard {
    fn drop(&mut self) {
        let _ = ratatui::crossterm::execute!(std::io::stdout(), DisableMouseCapture);
    }
}

impl Ui {
    fn new(shared: Arc<Shared>, presets: Vec<EngineCfg>) -> Ui {
        Ui {
            shared,
            presets,
            pilot: Pilot::default(),
            menu: None,
            help: false,
            help_scroll: 0,
            help_filter: String::new(),
            help_typing: false,
            view: View::Scope,
            sort: SortKey::Name,
            sort_reverse: false,
            hit: Hit::default(),
            dragging_gas: false,
            mouse_brake: false,
            vis_theta: 0.0,
            last_frame: Instant::now(),
            notice: String::new(),
            rpm_history: Vec::new(),
            last_history: Instant::now(),
        }
    }
}

pub fn run(mut terminal: DefaultTerminal, presets: Vec<EngineCfg>, shared: Arc<Shared>) -> std::io::Result<()> {
    let mut ui = Ui::new(shared, presets);
    ratatui::crossterm::execute!(std::io::stdout(), EnableMouseCapture)?;
    let _mouse = MouseGuard;

    loop {
        ui.sample_history();
        ui.advance_visual();
        ui.pilot.sync(&ui.shared);

        terminal.draw(|f| ui.draw(f))?;

        if event::poll(Duration::from_millis(33))? {
            match event::read()? {
                Event::Key(k) => {
                    if k.kind == KeyEventKind::Release {
                        continue;
                    }
                    if ui.handle_key(k.code, k.modifiers) {
                        break;
                    }
                }
                Event::Mouse(m) => {
                    if ui.handle_mouse(m) {
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

impl Ui {
    /// Rotates the displayed crankshaft; the visual speed grows with the real rpm.
    fn advance_visual(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f64().min(0.2);
        self.last_frame = now;
        let rpm = self.shared.rpm.get() as f64;
        if rpm > 100.0 {
            let rev_per_s = crate::pilot::anim_rev_per_s(self.pilot.anim_mode, rpm, self.engine().redline_rpm);
            self.vis_theta = (self.vis_theta + rev_per_s * dt * 2.0 * PI).rem_euclid(4.0 * PI);
        }
    }

    fn engine(&self) -> &EngineCfg {
        &self.presets[self.shared.active_engine.load(Ordering::Relaxed)]
    }

    fn sample_history(&mut self) {
        if self.last_history.elapsed() >= Duration::from_millis(100) {
            self.last_history = Instant::now();
            self.rpm_history.push(self.shared.rpm.get() as u64);
            if self.rpm_history.len() > 400 {
                self.rpm_history.remove(0);
            }
        }
    }

    /// Returns `true` to quit.
    fn handle_key(&mut self, code: KeyCode, mods: KeyModifiers) -> bool {
        if mods.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
            return true;
        }
        if self.help {
            self.handle_help_key(code, mods);
            return false;
        }
        if self.menu.is_some() {
            return self.handle_menu_key(code, mods);
        }
        let shared = Arc::clone(&self.shared);
        let s = &*shared;
        let gears = self.engine().gearbox.ratios.len() as i32;
        let throttle = self.pilot.throttle;
        match code {
            KeyCode::Char('q') => return true,
            KeyCode::Up => self.pilot.set_throttle(throttle + 0.05, s),
            KeyCode::Down => self.pilot.set_throttle(throttle - 0.05, s),
            KeyCode::Char('f') => self.pilot.set_throttle(1.0, s),
            KeyCode::Char('x') => self.pilot.set_throttle(0.0, s),
            KeyCode::Char(c @ '0'..='9') => self.pilot.set_throttle((c as u8 - b'0') as f32 / 10.0, s),
            KeyCode::Char('a') | KeyCode::Right => self.pilot.shift(1, gears, s),
            KeyCode::Char('z') | KeyCode::Left => self.pilot.shift(-1, gears, s),
            KeyCode::Char('n') => self.pilot.neutral(s),
            KeyCode::Char('c') => self.pilot.toggle_clutch(),
            KeyCode::Char(' ') => self.pilot.set_brake(!self.pilot.brake_on, s),
            KeyCode::Char('s') => s.starter.store(true, Ordering::Relaxed),
            KeyCode::Char('i') => {
                let on = !s.ignition.load(Ordering::Relaxed);
                s.ignition.store(on, Ordering::Relaxed);
            }
            KeyCode::Char('[') => s.volume.set((s.volume.get() - 0.05).max(0.0)),
            KeyCode::Char(']') => s.volume.set((s.volume.get() + 0.05).min(1.5)),
            KeyCode::Char('-') => s.convolution.set((s.convolution.get() - 0.1).max(0.0)),
            KeyCode::Char('=' | '+') => s.convolution.set((s.convolution.get() + 0.1).min(1.0)),
            KeyCode::Char(',' | '<') => crate::pilot::adjust_idle(s, self.engine().idle_rpm, -crate::pilot::IDLE_STEP),
            KeyCode::Char('.' | '>') => crate::pilot::adjust_idle(s, self.engine().idle_rpm, crate::pilot::IDLE_STEP),
            KeyCode::Char('e') => self.view = self.view.next(),
            KeyCode::Char('E') => self.view = self.view.prev(),
            KeyCode::Char('t') => self.pilot.cycle_anim(),
            KeyCode::Char('v') => {
                let next = (s.mic.load(Ordering::Relaxed) + 1) % crate::ambience::Mic::ALL.len();
                s.mic.store(next, Ordering::Relaxed);
            }
            KeyCode::Char('r') => self.reload(),
            KeyCode::Char('?' | 'h') | KeyCode::F(1) => self.help = true,
            KeyCode::Tab | KeyCode::Char('m') => self.open_menu(),
            KeyCode::Char('/') => {
                self.open_menu();
                if let Some(menu) = self.menu.as_mut() {
                    menu.typing = true;
                    menu.cursor = 0;
                }
            }
            _ => {}
        }
        false
    }

    fn open_menu(&mut self) {
        let active = self.shared.active_engine.load(Ordering::Relaxed);
        let cursor = self.filtered("").iter().position(|&i| i == active).unwrap_or(0);
        self.menu = Some(Menu { filter: String::new(), cursor, typing: false });
    }

    fn choose_engine(&mut self, index: usize) {
        if index != self.shared.active_engine.load(Ordering::Relaxed) {
            self.shared.engine_index.store(index, Ordering::Relaxed);
            self.pilot.reset(&self.shared);
            self.rpm_history.clear();
        }
    }

    /// Reloads the TOML files; the active engine is rebuilt from the new description.
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

    /// Indices of the engines whose name (or cylinder count) contains every filter word.
    fn filtered(&self, filter: &str) -> Vec<usize> {
        let words: Vec<String> = filter.split_whitespace().map(str::to_lowercase).collect();
        let mut indices: Vec<usize> = (0..self.presets.len())
            .filter(|&i| {
                let e = &self.presets[i];
                let hay = format!("{} {}cyl {:.1}l", e.name, e.cylinders.len(), e.displacement() * 1e3).to_lowercase();
                words.iter().all(|w| hay.contains(w.as_str()))
            })
            .collect();
        indices.sort_by(|&a, &b| {
            let (ea, eb) = (&self.presets[a], &self.presets[b]);
            let by_name = || ea.name.to_lowercase().cmp(&eb.name.to_lowercase());
            match self.sort {
                SortKey::Name => by_name(),
                SortKey::Displacement => ea.displacement().total_cmp(&eb.displacement()).then_with(by_name),
                SortKey::Cylinders => ea.cylinders.len().cmp(&eb.cylinders.len()).then_with(by_name),
                SortKey::Redline => ea.redline_rpm.total_cmp(&eb.redline_rpm).then_with(by_name),
                SortKey::Power => ea.power_kw.total_cmp(&eb.power_kw).then_with(by_name),
                SortKey::Torque => ea.torque_nm.total_cmp(&eb.torque_nm).then_with(by_name),
            }
        });
        if self.sort_reverse {
            indices.reverse();
        }
        indices
    }

    /// Applies a sort change while keeping the cursor on the same engine.
    fn change_sort(&mut self, menu: &mut Menu, change: impl FnOnce(&mut Self)) {
        let selected = self.filtered(&menu.filter).get(menu.cursor).copied();
        change(self);
        let order = self.filtered(&menu.filter);
        menu.cursor = selected.and_then(|s| order.iter().position(|&i| i == s)).unwrap_or(0);
    }

    /// Sorts by `key`; choosing the current column again reverses the order.
    fn sort_by(&mut self, menu: &mut Menu, key: SortKey) {
        self.change_sort(menu, |ui| {
            if ui.sort == key {
                ui.sort_reverse = !ui.sort_reverse;
            } else {
                ui.sort = key;
                ui.sort_reverse = false;
            }
        });
    }

    /// Engine list: `j` `k` or arrows to move, `/` to filter.
    fn handle_menu_key(&mut self, code: KeyCode, mods: KeyModifiers) -> bool {
        let Some(mut menu) = self.menu.take() else { return false };
        let ctrl = mods.contains(KeyModifiers::CONTROL);
        let matches = self.filtered(&menu.filter);
        let count = matches.len().max(1);
        if menu.typing {
            match code {
                // Esc cancels the typing and the filter.
                KeyCode::Esc => {
                    menu.filter.clear();
                    menu.typing = false;
                }
                KeyCode::Enter => {
                    menu.typing = false;
                    // A single match is selected directly.
                    if matches.len() == 1 {
                        self.choose_engine(matches[0]);
                        return false;
                    }
                }
                KeyCode::Backspace => {
                    if menu.filter.pop().is_none() {
                        menu.typing = false;
                    }
                }
                KeyCode::Char('u') if ctrl => menu.filter.clear(),
                KeyCode::Up => menu.cursor = (menu.cursor + count - 1) % count,
                KeyCode::Down => menu.cursor = (menu.cursor + 1) % count,
                KeyCode::Char(c) if !ctrl => menu.filter.push(c),
                _ => {}
            }
            if matches!(code, KeyCode::Char(_) | KeyCode::Backspace) {
                menu.cursor = 0;
            }
        } else {
            match code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Tab | KeyCode::Char('m') => {
                    // A confirmed filter is cleared first; the next press closes the list.
                    if menu.filter.is_empty() {
                        return false;
                    }
                    menu.filter.clear();
                    menu.cursor = 0;
                }
                KeyCode::Char('j') | KeyCode::Down => menu.cursor = (menu.cursor + 1) % count,
                KeyCode::Char('k') | KeyCode::Up => menu.cursor = (menu.cursor + count - 1) % count,
                KeyCode::PageDown => menu.cursor = (menu.cursor + 8).min(count - 1),
                KeyCode::PageUp => menu.cursor = menu.cursor.saturating_sub(8),
                KeyCode::Home | KeyCode::Char('g') => menu.cursor = 0,
                KeyCode::End | KeyCode::Char('G') => menu.cursor = count - 1,
                KeyCode::Char('/') => {
                    menu.filter.clear();
                    menu.typing = true;
                    menu.cursor = 0;
                }
                KeyCode::Char('>' | '.') => self.change_sort(&mut menu, |ui| ui.sort = ui.sort.step(1)),
                KeyCode::Char('<' | ',') => self.change_sort(&mut menu, |ui| ui.sort = ui.sort.step(-1)),
                KeyCode::Char('r') => self.change_sort(&mut menu, |ui| ui.sort_reverse = !ui.sort_reverse),
                KeyCode::Enter => {
                    if let Some(&selected) = matches.get(menu.cursor) {
                        self.choose_engine(selected);
                        return false;
                    }
                }
                _ => {}
            }
        }
        let matches = self.filtered(&menu.filter);
        menu.cursor = menu.cursor.min(matches.len().saturating_sub(1));
        self.menu = Some(menu);
        false
    }

    /// Help window: `j` `k` or arrows to scroll, `/` to filter.
    fn handle_help_key(&mut self, code: KeyCode, mods: KeyModifiers) {
        let ctrl = mods.contains(KeyModifiers::CONTROL);
        if self.help_typing {
            match code {
                KeyCode::Esc => {
                    self.help_filter.clear();
                    self.help_typing = false;
                }
                KeyCode::Enter => self.help_typing = false,
                KeyCode::Backspace => {
                    if self.help_filter.pop().is_none() {
                        self.help_typing = false;
                    }
                }
                KeyCode::Char('u') if ctrl => self.help_filter.clear(),
                KeyCode::Char(c) if !ctrl => self.help_filter.push(c),
                _ => {}
            }
            self.help_scroll = 0;
            return;
        }
        match code {
            KeyCode::Esc | KeyCode::Char('q' | 'h' | '?') | KeyCode::F(1) | KeyCode::Enter => {
                // A confirmed filter is cleared first; the next press closes the help.
                if self.help_filter.is_empty() {
                    self.help = false;
                } else {
                    self.help_filter.clear();
                }
                self.help_scroll = 0;
            }
            KeyCode::Char('j') | KeyCode::Down => self.help_scroll = self.help_scroll.saturating_add(1),
            KeyCode::Char('k') | KeyCode::Up => self.help_scroll = self.help_scroll.saturating_sub(1),
            KeyCode::PageDown => self.help_scroll = self.help_scroll.saturating_add(8),
            KeyCode::PageUp => self.help_scroll = self.help_scroll.saturating_sub(8),
            KeyCode::Home | KeyCode::Char('g') => self.help_scroll = 0,
            KeyCode::End | KeyCode::Char('G') => self.help_scroll = u16::MAX,
            KeyCode::Char('/') => {
                self.help_filter.clear();
                self.help_typing = true;
                self.help_scroll = 0;
            }
            _ => {}
        }
    }

    fn draw_menu(&self, f: &mut Frame, menu: &Menu) {
        let area = f.area();
        let matches = self.filtered(&menu.filter);
        let details = |e: &EngineCfg| {
            format!(
                "{:>5.1} L  {:>2} cyl  {:>6.0} rpm  {}  {}",
                e.displacement() * 1e3,
                e.cylinders.len(),
                e.redline_rpm,
                if e.power_kw > 0.0 { format!("{:>5.0} kW", e.power_kw) } else { "      — ".to_string() },
                if e.torque_nm > 0.0 { format!("{:>5.0} N·m", e.torque_nm) } else { "        —".to_string() }
            )
        };
        // Name column: width of the longest name (capped), to keep the rest aligned.
        let name_w = self.presets.iter().map(|e| e.name.chars().count()).max().unwrap_or(10).min(48);
        let detail_w = self.presets.first().map_or(0, |e| details(e).chars().count());
        let width = ((name_w + detail_w + 8) as u16).min(area.width);
        let height = (matches.len().max(1) as u16 + 3).clamp(6, area.height.saturating_sub(2).max(6)).min(area.height);
        let popup = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height - height) / 2,
            width,
            height,
        };
        let active = self.shared.active_engine.load(Ordering::Relaxed);
        let items: Vec<ListItem> = if matches.is_empty() {
            vec![ListItem::new(Span::styled("  No engine matches", Style::new().dim()))]
        } else {
            matches
                .iter()
                .map(|&i| {
                    let e = &self.presets[i];
                    let mark = if i == active { "● " } else { "  " };
                    ListItem::new(Line::from(vec![
                        Span::raw(mark),
                        Span::styled(fit(e.name, name_w), Style::new().bold()),
                        Span::raw("  "),
                        Span::styled(details(e), Style::new().dim()),
                    ]))
                })
                .collect()
        };
        let title = if menu.typing {
            format!(" Engine ({}/{}) · /{}▏", matches.len(), self.presets.len(), menu.filter)
        } else if menu.filter.is_empty() {
            format!(" Engine ({}/{}) ", matches.len(), self.presets.len())
        } else {
            format!(" Engine ({}/{}) · /{} ", matches.len(), self.presets.len(), menu.filter)
        };
        let footer = if menu.typing {
            " Enter confirm · Esc cancel "
        } else {
            " j k move · / filter · < > sort · r reverse · Enter select "
        };
        let block = Block::bordered().title(title).title_bottom(footer);
        let inner = block.inner(popup);
        f.render_widget(Clear, popup);
        f.render_widget(block, popup);
        let header_row = Rect { height: 1, ..inner };
        let list_area = Rect { y: inner.y + 1, height: inner.height.saturating_sub(1), ..inner };

        // Sort header: the column in use carries an arrow; each title is clickable.
        let arrow = if self.sort_reverse { "▼" } else { "▲" };
        let columns = [(SortKey::Name, name_w, false), (SortKey::Displacement, 7, true), (SortKey::Cylinders, 6, true), (SortKey::Redline, 10, true), (SortKey::Power, 8, true), (SortKey::Torque, 9, true)];
        let mut spans = vec![Span::raw("    ")];
        let mut zones = Vec::new();
        let mut x = header_row.x + 4;
        for (n, (key, w, right)) in columns.into_iter().enumerate() {
            let label = if key == self.sort { format!("{}{arrow}", key.title()) } else { key.title().to_string() };
            let text = if right { format!("{label:>w$}") } else { format!("{label:<w$}") };
            let style = if key == self.sort { Style::new().black().on_cyan().bold() } else { Style::new().bold().yellow() };
            zones.push((Rect { x, y: header_row.y, width: w as u16, height: 1 }, key));
            spans.push(Span::styled(text, style));
            if n + 1 < columns.len() {
                spans.push(Span::raw("  "));
                x += w as u16 + 2;
            }
        }
        f.render_widget(Paragraph::new(Line::from(spans)), header_row);

        let list = List::new(items).highlight_style(Style::new().black().on_cyan()).highlight_symbol("▶ ");
        let mut state = ListState::default().with_selected(if matches.is_empty() { None } else { Some(menu.cursor) });
        f.render_stateful_widget(list, list_area, &mut state);
        self.hit.menu_popup.set(popup);
        self.hit.menu_list.set(list_area);
        self.hit.menu_offset.set(state.offset());
        *self.hit.menu_matches.borrow_mut() = matches;
        *self.hit.menu_header.borrow_mut() = zones;
    }

    fn draw(&self, f: &mut Frame) {
        let rows = Layout::vertical([Constraint::Length(3), Constraint::Min(10), Constraint::Length(3)]).split(f.area());
        self.draw_header(f, rows[0]);
        self.hit.header.set(rows[0]);

        let cols = Layout::horizontal([Constraint::Length(44), Constraint::Min(20)]).split(rows[1]);
        let left = Layout::vertical([
            Constraint::Length(9),
            Constraint::Length(3),
            Constraint::Length(5),
            Constraint::Min(5),
        ])
        .split(cols[0]);
        self.hit.gear.set(left[0]);
        self.hit.audio.set(left[3]);
        let right_col = Layout::vertical([Constraint::Length(1), Constraint::Min(3)]).split(cols[1]);
        self.draw_tabs(f, right_col[0]);
        let view_area = right_col[1];
        self.hit.view.set(view_area);
        self.draw_gear(f, left[0]);
        self.draw_rpm(f, left[1]);
        self.draw_pedals(f, left[2]);
        self.draw_audio(f, left[3]);

        let right = Layout::new(
            Direction::Vertical,
            [Constraint::Percentage(60), Constraint::Percentage(40)],
        )
        .split(view_area);
        match self.view {
            View::Scope => {
                self.draw_scope(f, right[0]);
                self.draw_history(f, right[1]);
            }
            View::Cylinders => self.draw_engine_view(f, view_area),
            View::Top | View::Front => {
                let cfg = self.engine();
                let s = &self.shared;
                let firing = s.firing.load(Ordering::Relaxed);
                let load = s.throttle_effective.get() as f64;
                if self.view == View::Top {
                    crate::view::draw_top(f, view_area, cfg, self.vis_theta, firing, load);
                } else {
                    crate::view::draw_front(f, view_area, cfg, self.vis_theta, firing, load);
                }
            }
        }

        self.draw_help(f, rows[2]);
        if let Some(menu) = &self.menu {
            self.draw_menu(f, menu);
        }
        if self.help {
            self.draw_shortcuts(f);
        }
    }

    fn draw_header(&self, f: &mut Frame, area: Rect) {
        let cfg = self.engine();
        let s = &self.shared;
        let status = if s.starter.load(Ordering::Relaxed) {
            Span::styled(" STARTER ", Style::new().black().on_yellow())
        } else if !s.ignition.load(Ordering::Relaxed) {
            Span::styled(" IGNITION OFF ", Style::new().white().on_red())
        } else if s.rpm.get() < 150.0 {
            Span::styled(" STALLED ", Style::new().white().on_red())
        } else {
            Span::styled(" ENGINE RUNNING ", Style::new().black().on_green())
        };
        let line = Line::from(vec![
            Span::styled(cfg.name, Style::new().bold().cyan()),
            Span::raw(format!(
                "  {:.1} L  {} cyl.  ",
                cfg.displacement() * 1e3,
                cfg.cylinders.len()
            )),
            Span::styled(
                format!(
                    "{}{}",
                    if cfg.power_kw > 0.0 { format!("{:.0} kW ({:.0} hp)  ", cfg.power_kw, cfg.power_kw * 1.341) } else { String::new() },
                    if cfg.torque_nm > 0.0 { format!("{:.0} N·m  ", cfg.torque_nm) } else { String::new() }
                ),
                Style::new().bold().white(),
            ),
            status,
            Span::styled(
                format!("  Mic: {}", crate::ambience::Mic::from_index(s.mic.load(Ordering::Relaxed)).name()),
                Style::new().dim(),
            ),
            Span::styled(
                if self.notice.is_empty() { String::new() } else { format!("  {}", self.notice) },
                Style::new().yellow(),
            ),
        ]);
        f.render_widget(
            Paragraph::new(line).block(Block::bordered().title(" engine-sim-rs ")),
            area,
        );
    }

    fn draw_gear(&self, f: &mut Frame, area: Rect) {
        let gear = self.shared.gear.load(Ordering::Relaxed);
        let target = self.pilot.gear_target;
        let glyph = &BIG_DIGITS[if gear == 0 { 10 } else { gear as usize }];
        let mut lines: Vec<Line> = glyph
            .iter()
            .map(|r| Line::styled(format!("  {r}"), Style::new().bold().yellow()))
            .collect();
        lines.push(Line::raw(""));
        let shifting = target != gear;
        lines.push(Line::styled(
            if shifting { format!("  -> {}", gear_name(target)) } else { String::new() },
            Style::new().dim(),
        ));

        let block = Block::bordered().title(" Gear ");
        let inner = block.inner(area);
        f.render_widget(block, area);
        let parts = Layout::horizontal([Constraint::Length(12), Constraint::Min(10)]).split(inner);
        f.render_widget(Paragraph::new(lines), parts[0]);

        let kmh = self.shared.speed_kmh.get();
        let text = vec![
            Line::raw(""),
            Line::from(vec![Span::styled(format!("{kmh:5.0}"), Style::new().bold().white().add_modifier(Modifier::BOLD)), Span::raw(" km/h")]),
            Line::raw(""),
            Line::from(vec![
                Span::styled(format!("{:5.0}", self.shared.rpm.get()), Style::new().bold().white()),
                Span::raw(" rpm"),
            ]),
            self.output_line(self.shared.torque.get(), "N·m", "", self.shared.torque.get() < -3.0),
            self.output_line(self.shared.power.get(), "kW", &format!("  ({:.0} hp)", self.shared.power.get() * 1.341), false),
        ];
        f.render_widget(Paragraph::new(text), parts[1]);
    }

    /// Torque / power readout; engine braking on overrun (`braking`) is shown in red.
    fn output_line(&self, value: f32, unit: &str, suffix: &str, braking: bool) -> Line<'static> {
        // Avoid "-0" for tiny negative values.
        let value = if value.abs() < 0.5 { 0.0 } else { value };
        let style = if braking || value < -3.0 { Style::new().bold().red() } else { Style::new().bold().white() };
        let mut spans = vec![Span::styled(format!("{value:5.0}"), style), Span::raw(format!(" {unit}{suffix}"))];
        if braking {
            spans.push(Span::styled(" braking", Style::new().red()));
        }
        Line::from(spans)
    }

    fn draw_rpm(&self, f: &mut Frame, area: Rect) {
        let cfg = self.engine();
        let rpm = self.shared.rpm.get() as f64;
        let ratio = (rpm / (cfg.redline_rpm * 1.05)).clamp(0.0, 1.0);
        let color = if rpm > cfg.redline_rpm * 0.97 {
            Color::Red
        } else if rpm > cfg.redline_rpm * 0.82 {
            Color::Yellow
        } else {
            Color::Green
        };
        let g = Gauge::default()
            .block(Block::bordered().title(format!(" RPM  (redline {:.0}) ", cfg.redline_rpm)))
            .gauge_style(Style::new().fg(color).bg(Color::DarkGray))
            .label(format!("{rpm:.0} rpm"))
            .ratio(ratio);
        f.render_widget(g, area);
    }

    fn draw_pedals(&self, f: &mut Frame, area: Rect) {
        let s = &self.shared;
        let block = Block::bordered().title(" Pedals ");
        let inner = block.inner(area);
        f.render_widget(block, area);
        let rows = Layout::vertical([Constraint::Length(1); 3]).split(inner);
        for (cell, row) in self.hit.pedals.iter().zip(rows.iter()) {
            cell.set(*row);
        }

        let gauge = |label: &'static str, ratio: f32, color: Color| {
            Gauge::default()
                .label(label)
                .gauge_style(Style::new().fg(color).bg(Color::DarkGray))
                .ratio(ratio.clamp(0.0, 1.0) as f64)
        };
        f.render_widget(gauge("Throttle", s.throttle_effective.get(), Color::Green), rows[0]);
        f.render_widget(gauge("Clutch", 1.0 - s.clutch_engagement.get(), Color::Cyan), rows[1]);
        f.render_widget(gauge("Brake", s.brake.get(), Color::Red), rows[2]);
    }

    fn draw_audio(&self, f: &mut Frame, area: Rect) {
        let s = &self.shared;
        let lines = vec![
            Line::from(format!("Volume          {:>4.0} %", s.volume.get() * 100.0)),
            Line::from(format!("Convolution     {:>4.0} %", s.convolution.get() * 100.0)),
            Line::from(format!("Leveler gain    {:>8.4}", s.leveler_gain.get())),
            Line::from(format!("Throttle input  {:>4.0} %", self.pilot.throttle * 100.0)),
            Line::from(format!("Idle speed      {:>4.0} rpm", s.idle_rpm.get())),
        ];
        f.render_widget(Paragraph::new(lines).block(Block::bordered().title(" Audio ")), area);
    }

    fn draw_engine_view(&self, f: &mut Frame, area: Rect) {
        let cfg = self.engine();
        let parts = Layout::vertical([Constraint::Percentage(62), Constraint::Percentage(38)]).split(area);
        let rpm = self.shared.rpm.get() as f64;
        let speed = crate::pilot::anim_label(self.pilot.anim_mode, rpm, cfg.redline_rpm);
        let firing = self.shared.firing.load(Ordering::Relaxed);
        let load = self.shared.throttle_effective.get() as f64;
        crate::view::draw_cylinders(f, parts[0], cfg, self.vis_theta, &speed, firing, load);

        let bottom = Layout::horizontal([Constraint::Min(30), Constraint::Length(40)]).split(parts[1]);
        let pv = self.shared.pv.try_lock().map(|v| v.clone()).unwrap_or_default();
        crate::view::draw_pv(f, bottom[0], &pv);

        let mut order: Vec<(f64, usize)> = cfg.cylinders.iter().enumerate().map(|(i, c)| (c.fire_deg, i + 1)).collect();
        order.sort_by(|a, b| a.0.total_cmp(&b.0));
        let order: Vec<String> = order.iter().map(|o| o.1.to_string()).collect();
        let info = vec![
            Line::from(format!("Displacement {:.2} L", cfg.displacement() * 1e3)),
            Line::from(format!("Bore × stroke {:.1}×{:.1} mm", cfg.bore * 1e3, cfg.stroke * 1e3)),
            Line::from(format!("Compression  {:.1}:1", cfg.compression_ratio)),
            Line::from(format!("RPM          {rpm:.0}")),
            Line::from(format!("Throttle     {:.0} %", load * 100.0)),
            Line::from(if firing { Span::styled("Injection    on", Style::new().green()) } else { Span::styled("Injection    cut", Style::new().red()) }),
            Line::from(format!("Firing order {}", order.join("-"))),
        ];
        f.render_widget(
            Paragraph::new(info)
                .wrap(ratatui::widgets::Wrap { trim: false })
                .block(Block::bordered().title(" Engine ")),
            bottom[1],
        );
    }

    fn draw_scope(&self, f: &mut Frame, area: Rect) {
        let data: Vec<(f64, f64)> = match self.shared.scope.try_lock() {
            Ok(v) => v.iter().enumerate().map(|(i, &s)| (i as f64, s as f64)).collect(),
            Err(_) => Vec::new(),
        };
        let datasets = vec![Dataset::default()
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::new().fg(Color::Cyan))
            .data(&data)];
        let chart = Chart::new(datasets)
            .block(Block::bordered().title(" Output signal "))
            .x_axis(Axis::default().bounds([0.0, SCOPE_POINTS as f64]))
            .y_axis(Axis::default().bounds([-1.0, 1.0]));
        f.render_widget(chart, area);
    }

    fn draw_history(&self, f: &mut Frame, area: Rect) {
        let width = area.width.saturating_sub(2) as usize;
        let start = self.rpm_history.len().saturating_sub(width);
        let spark = Sparkline::default()
            .block(Block::bordered().title(" RPM history "))
            .data(&self.rpm_history[start..])
            .max(self.engine().redline_rpm as u64)
            .style(Style::new().fg(Color::Yellow));
        f.render_widget(spark, area);
    }

    /// Tab strip above the right-hand panel; each tab is clickable.
    fn draw_tabs(&self, f: &mut Frame, area: Rect) {
        let mut spans = Vec::new();
        let mut zones = Vec::new();
        let mut x = area.x;
        for view in View::ALL {
            let label = format!(" {} ", view.title());
            let width = label.chars().count() as u16;
            let style = if view == self.view { Style::new().black().on_cyan().bold() } else { Style::new().dim() };
            zones.push((Rect { x, y: area.y, width, height: 1 }, view));
            spans.push(Span::styled(label, style));
            spans.push(Span::raw(" "));
            x += width + 1;
        }
        *self.hit.tabs.borrow_mut() = zones;
        f.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    /// btop-style bottom bar: the main keys, highlighted and clickable, or the last message.
    fn draw_help(&self, f: &mut Frame, area: Rect) {
        let items: [(&str, &str, Option<BarAction>); 9] = [
            ("↑↓", "Throttle", None),
            ("a z", "Gear", None),
            ("c", "Clutch", Some(BarAction::Clutch)),
            ("Space", "Brake", Some(BarAction::Brake)),
            ("m", "Engines", Some(BarAction::Menu)),
            ("e", "View", Some(BarAction::View)),
            ("v", "Mic", Some(BarAction::Mic)),
            ("h", "Help", Some(BarAction::Help)),
            ("q", "Quit", Some(BarAction::Quit)),
        ];
        let mut spans = Vec::new();
        let mut zones = Vec::new();
        let mut x = area.x + 1;
        for (key, label, action) in items {
            let width = (key.chars().count() + 1 + label.chars().count()) as u16;
            if let Some(action) = action {
                zones.push((Rect { x, y: area.y + 1, width, height: 1 }, action));
            }
            spans.push(Span::styled(key, Style::new().bold().yellow()));
            spans.push(Span::raw(format!(" {label}  ")));
            x += width + 2;
        }
        *self.hit.bar.borrow_mut() = zones;
        if !self.notice.is_empty() {
            spans.push(Span::styled(format!("   {}", self.notice), Style::new().yellow()));
        }
        f.render_widget(Paragraph::new(Line::from(spans)).block(Block::bordered()), area);
    }

    fn draw_shortcuts(&self, f: &mut Frame) {
        let area = f.area();
        let words: Vec<String> = self.help_filter.split_whitespace().map(str::to_lowercase).collect();
        let list: Vec<(&str, &str)> = crate::help::SHORTCUTS
            .iter()
            .chain(crate::help::MOUSE.iter())
            .copied()
            .filter(|(k, d)| {
                let hay = format!("{k} {d}").to_lowercase();
                words.iter().all(|w| hay.contains(w.as_str()))
            })
            .collect();
        let all = crate::help::SHORTCUTS.iter().chain(crate::help::MOUSE.iter());
        let key_w = all.clone().map(|s| s.0.chars().count()).max().unwrap_or(10);
        let text_w = all.map(|s| s.1.chars().count()).max().unwrap_or(10);
        let width = ((key_w + text_w + 6) as u16).min(area.width);
        let total = crate::help::SHORTCUTS.len() + crate::help::MOUSE.len();
        let height = (total as u16 + 2).min(area.height);
        let popup = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height - height) / 2,
            width,
            height,
        };
        let mut lines: Vec<Line> = list
            .iter()
            .map(|(k, d)| Line::from(vec![Span::styled(format!(" {}", fit(k, key_w)), Style::new().bold().yellow()), Span::raw("  "), Span::raw(*d)]))
            .collect();
        if lines.is_empty() {
            lines.push(Line::styled(" No shortcut matches", Style::new().dim()));
        }
        let max_scroll = (lines.len() as u16).saturating_sub(popup.height.saturating_sub(2));
        let title = if self.help_typing {
            format!(" Shortcuts · /{}▏", self.help_filter)
        } else if self.help_filter.is_empty() {
            " Keyboard shortcuts ".to_string()
        } else {
            format!(" Shortcuts · /{} ", self.help_filter)
        };
        let footer = if self.help_typing { " Enter confirm · Esc cancel " } else { " j k scroll · / filter · h or Esc close " };
        f.render_widget(Clear, popup);
        f.render_widget(
            Paragraph::new(lines)
                .scroll((self.help_scroll.min(max_scroll), 0))
                .block(Block::bordered().title(title).title_bottom(footer)),
            popup,
        );
    }
}

/// Truncates or pads with spaces to exactly `width` characters.
fn fit(text: &str, width: usize) -> String {
    let n = text.chars().count();
    if n > width {
        let mut t: String = text.chars().take(width.saturating_sub(1)).collect();
        t.push('…');
        t
    } else {
        format!("{text}{}", " ".repeat(width - n))
    }
}

fn gear_name(g: i32) -> String {
    if g == 0 { "N".into() } else { g.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseEvent, MouseEventKind};
    use ratatui::Terminal;

    fn render(ui: &Ui, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| ui.draw(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                text.push_str(buf[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    fn ui_with(shared: Arc<Shared>) -> Ui {
        Ui::new(shared, crate::presets::all())
    }

    fn press(ui: &mut Ui, keys: &str) -> bool {
        let mut quit = false;
        for c in keys.chars() {
            let code = if c == '\n' { KeyCode::Enter } else { KeyCode::Char(c) };
            quit |= ui.handle_key(code, KeyModifiers::NONE);
        }
        quit
    }

    #[test]
    fn renders_dashboard() {
        let shared = Arc::new(Shared::new());
        shared.rpm.set(4200.0);
        shared.speed_kmh.set(87.0);
        shared.gear.store(3, Ordering::Relaxed);
        shared.throttle_effective.set(0.6);
        shared.clutch_engagement.set(0.8);
        *shared.scope.lock().unwrap() = (0..SCOPE_POINTS).map(|i| (i as f32 * 0.2).sin() * 0.6).collect();
        let mut ui = ui_with(shared);
        ui.rpm_history = (0..300).map(|i| 900 + i * 12).collect();
        ui.menu = Some(Menu { filter: "v1".into(), cursor: 1, typing: false });
        println!("{}", render(&ui, 120, 34));
    }

    #[test]
    fn renders_views() {
        for (cyl, view) in [(12, View::Cylinders), (8, View::Top), (8, View::Front), (4, View::Top), (4, View::Front)] {
            let shared = Arc::new(Shared::new());
            let mut ui = ui_with(shared.clone());
            let idx = ui.presets.iter().position(|e| e.cylinders.len() == cyl).unwrap();
            shared.active_engine.store(idx, Ordering::Relaxed);
            shared.rpm.set(3000.0);
            shared.throttle_effective.set(0.5);
            shared.firing.store(true, Ordering::Relaxed);
            ui.view = view;
            ui.vis_theta = 0.9;
            assert!(render(&ui, 140, 40).contains("Engine") || view != View::Cylinders);
        }
    }

    #[test]
    fn basic_keys() {
        let shared = Arc::new(Shared::new());
        let mut ui = ui_with(shared.clone());
        press(&mut ui, "5");
        assert!((ui.pilot.throttle - 0.5).abs() < 1e-6);
        ui.handle_key(KeyCode::Up, KeyModifiers::NONE);
        assert!((ui.pilot.throttle - 0.55).abs() < 1e-6);
        press(&mut ui, "f");
        assert_eq!(ui.pilot.throttle, 1.0);
        press(&mut ui, "x");
        assert_eq!(ui.pilot.throttle, 0.0);
        press(&mut ui, "aa");
        assert_eq!(ui.pilot.gear_target, 2);
        press(&mut ui, "z");
        assert_eq!(ui.pilot.gear_target, 1);
        press(&mut ui, "n");
        assert_eq!(ui.pilot.gear_target, 0);
        press(&mut ui, " ");
        assert!(ui.pilot.brake_on);
        press(&mut ui, "h");
        assert!(ui.help);
        press(&mut ui, "h");
        assert!(!ui.help);
        assert!(press(&mut ui, "q"));
    }

    #[test]
    fn engine_filter() {
        let shared = Arc::new(Shared::new());
        let mut ui = ui_with(shared.clone());
        press(&mut ui, "m");
        assert!(ui.menu.is_some());
        // Without `/`, j and k move and letters don't filter.
        let start = ui.menu.as_ref().unwrap().cursor;
        press(&mut ui, "jj");
        assert_eq!(ui.menu.as_ref().unwrap().cursor, start + 2);
        press(&mut ui, "k");
        assert_eq!(ui.menu.as_ref().unwrap().cursor, start + 1);
        assert_eq!(ui.menu.as_ref().unwrap().filter, "");
        press(&mut ui, "/bugatti");
        assert_eq!(ui.menu.as_ref().unwrap().filter, "bugatti");
        press(&mut ui, "\n");
        assert!(ui.menu.is_none());
        assert_eq!(shared.engine_index.load(Ordering::Relaxed), ui.presets.iter().position(|e| e.name.contains("Bugatti")).unwrap());
    }

    fn mouse(kind: MouseEventKind, x: u16, y: u16) -> MouseEvent {
        MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE }
    }

    #[test]
    fn mouse_interactions() {
        use ratatui::crossterm::event::MouseButton::{Left, Middle, Right};
        let shared = Arc::new(Shared::new());
        let mut ui = ui_with(shared.clone());
        render(&ui, 120, 34);

        // Wheel: throttle on the right panel, gears on the Gear panel.
        let view = ui.hit.view.get();
        ui.handle_mouse(mouse(MouseEventKind::ScrollUp, view.x + 5, view.y + 5));
        assert!((ui.pilot.throttle - 0.05).abs() < 1e-6);
        let gear = ui.hit.gear.get();
        ui.handle_mouse(mouse(MouseEventKind::ScrollUp, gear.x + 2, gear.y + 2));
        assert_eq!(ui.pilot.gear_target, 1);
        ui.handle_mouse(mouse(MouseEventKind::Down(Left), gear.x + 2, gear.y + 2));
        assert_eq!(ui.pilot.gear_target, 2);
        ui.handle_mouse(mouse(MouseEventKind::Down(Right), gear.x + 2, gear.y + 2));
        assert_eq!(ui.pilot.gear_target, 1);
        ui.handle_mouse(mouse(MouseEventKind::Down(Middle), gear.x + 2, gear.y + 2));
        assert_eq!(ui.pilot.gear_target, 0);

        // Gauges: throttle by click and drag, brake held, clutch by click.
        let gas = ui.hit.pedals[0].get();
        ui.handle_mouse(mouse(MouseEventKind::Down(Left), gas.x + gas.width / 2, gas.y));
        assert!((ui.pilot.throttle - 0.5).abs() < 0.05, "{}", ui.pilot.throttle);
        ui.handle_mouse(mouse(MouseEventKind::Drag(Left), gas.x + gas.width - 1, gas.y + 3));
        assert!(ui.pilot.throttle > 0.99);
        ui.handle_mouse(mouse(MouseEventKind::Up(Left), 0, 0));
        let brake = ui.hit.pedals[2].get();
        ui.handle_mouse(mouse(MouseEventKind::Down(Left), brake.x + 3, brake.y));
        assert!(ui.pilot.brake_on);
        ui.handle_mouse(mouse(MouseEventKind::Up(Left), 0, 0));
        assert!(!ui.pilot.brake_on);
        let clutch = ui.hit.pedals[1].get();
        ui.handle_mouse(mouse(MouseEventKind::Down(Left), clutch.x + 3, clutch.y));
        assert!(ui.pilot.clutch_latched);

        // Bottom bar: "Engines" opens the list; clicking a row selects the engine.
        let (zone, _) = *ui.hit.bar.borrow().iter().find(|(_, a)| *a == BarAction::Menu).unwrap();
        ui.handle_mouse(mouse(MouseEventKind::Down(Left), zone.x + 1, zone.y));
        assert!(ui.menu.is_some());
        render(&ui, 120, 34);
        let list = ui.hit.menu_list.get();
        ui.handle_mouse(mouse(MouseEventKind::Down(Left), list.x + 3, list.y + 2));
        assert!(ui.menu.is_none());
        // The list is sorted alphabetically: the third row is the third engine by name.
        assert_eq!(shared.engine_index.load(Ordering::Relaxed), ui.filtered("")[2]);

        // Click on "Quit".
        render(&ui, 120, 34);
        let (zone, _) = *ui.hit.bar.borrow().iter().find(|(_, a)| *a == BarAction::Quit).unwrap();
        assert!(ui.handle_mouse(mouse(MouseEventKind::Down(Left), zone.x, zone.y)));
    }

    #[test]
    fn help_scroll_and_filter() {
        let shared = Arc::new(Shared::new());
        let mut ui = ui_with(shared);
        press(&mut ui, "h");
        assert!(ui.help);
        press(&mut ui, "jj");
        assert_eq!(ui.help_scroll, 2);
        press(&mut ui, "/brake");
        assert!(ui.help_typing);
        let text = render(&ui, 140, 40);
        assert!(text.contains("Brake"));
        assert!(!text.contains("Starter"), "the filter hides the other lines");
        press(&mut ui, "\n");
        assert!(!ui.help_typing && ui.help);
        press(&mut ui, "h");
        assert!(ui.help && ui.help_filter.is_empty(), "a first h clears the filter");
        press(&mut ui, "h");
        assert!(!ui.help);
    }

    #[test]
    fn tabs_select_the_view() {
        use ratatui::crossterm::event::MouseButton::Left;
        let shared = Arc::new(Shared::new());
        let mut ui = ui_with(shared);
        let text = render(&ui, 120, 34);
        for title in ["Scope", "Cylinders", "Top", "Front"] {
            assert!(text.contains(title), "missing tab {title}");
        }
        let tabs = ui.hit.tabs.borrow().clone();
        for (rect, view) in tabs {
            ui.handle_mouse(mouse(MouseEventKind::Down(Left), rect.x + 1, rect.y));
            assert!(ui.view == view);
            render(&ui, 120, 34);
        }
        // The `e` key still cycles, and the active tab follows.
        ui.view = View::Front;
        press(&mut ui, "e");
        assert!(ui.view == View::Scope);
    }

    fn names(ui: &Ui) -> Vec<&'static str> {
        ui.filtered("").iter().map(|&i| ui.presets[i].name).collect()
    }

    #[test]
    fn engine_list_is_alphabetical_by_default() {
        let ui = ui_with(Arc::new(Shared::new()));
        let list = names(&ui);
        let mut sorted = list.clone();
        sorted.sort_by_key(|n| n.to_lowercase());
        assert_eq!(list, sorted);
        assert_eq!(list.len(), ui.presets.len());
    }

    #[test]
    fn engine_list_sorting_keys_and_reverse() {
        let shared = Arc::new(Shared::new());
        let mut ui = ui_with(shared);
        press(&mut ui, "m");
        // Next column: displacement, ascending, ties broken by name.
        press(&mut ui, ">");
        assert_eq!(ui.sort, SortKey::Displacement);
        let disp: Vec<f64> = ui.filtered("").iter().map(|&i| ui.presets[i].displacement()).collect();
        assert!(disp.windows(2).all(|w| w[0] <= w[1]));
        press(&mut ui, ">");
        assert_eq!(ui.sort, SortKey::Cylinders);
        let cyl: Vec<usize> = ui.filtered("").iter().map(|&i| ui.presets[i].cylinders.len()).collect();
        assert!(cyl.windows(2).all(|w| w[0] <= w[1]));
        press(&mut ui, ">");
        assert_eq!(ui.sort, SortKey::Redline);
        press(&mut ui, ">");
        assert_eq!(ui.sort, SortKey::Power);
        press(&mut ui, ">");
        assert_eq!(ui.sort, SortKey::Torque);
        let nm: Vec<f64> = ui.filtered("").iter().map(|&i| ui.presets[i].torque_nm).collect();
        assert!(nm.windows(2).all(|w| w[0] <= w[1]));
        press(&mut ui, "<");
        assert_eq!(ui.sort, SortKey::Power);
        // `r` reverses the order: strongest engine first.
        press(&mut ui, "r");
        let kw: Vec<f64> = ui.filtered("").iter().map(|&i| ui.presets[i].power_kw).collect();
        assert!(kw.windows(2).all(|w| w[0] >= w[1]));
        // `<` goes back a column and `>` wraps around to the name.
        press(&mut ui, "<");
        assert_eq!(ui.sort, SortKey::Redline);
        press(&mut ui, ">>>");
        assert_eq!(ui.sort, SortKey::Name);
    }

    #[test]
    fn sorting_keeps_the_cursor_on_the_same_engine() {
        let mut ui = ui_with(Arc::new(Shared::new()));
        press(&mut ui, "m");
        press(&mut ui, "jjj");
        let picked = ui.filtered("")[ui.menu.as_ref().unwrap().cursor];
        press(&mut ui, ">");
        let after = ui.filtered("")[ui.menu.as_ref().unwrap().cursor];
        assert_eq!(picked, after);
    }

    #[test]
    fn opening_the_list_puts_the_cursor_on_the_active_engine() {
        let shared = Arc::new(Shared::new());
        let mut ui = ui_with(shared.clone());
        let target = ui.presets.iter().position(|e| e.name.contains("Bugatti")).unwrap();
        shared.active_engine.store(target, Ordering::Relaxed);
        press(&mut ui, "m");
        let cursor = ui.menu.as_ref().unwrap().cursor;
        assert_eq!(ui.filtered("")[cursor], target);
    }

    #[test]
    fn filtering_respects_the_sort_order() {
        let mut ui = ui_with(Arc::new(Shared::new()));
        press(&mut ui, "m/ferrari\n");
        let shown: Vec<_> = ui.filtered("ferrari").iter().map(|&i| ui.presets[i].name).collect();
        assert!(shown.len() > 1 && shown.iter().all(|n| n.to_lowercase().contains("ferrari")));
        let mut sorted = shown.clone();
        sorted.sort_by_key(|n| n.to_lowercase());
        assert_eq!(shown, sorted);
    }

    #[test]
    fn clicking_a_header_sorts_and_clicking_again_reverses() {
        use ratatui::crossterm::event::MouseButton::Left;
        let mut ui = ui_with(Arc::new(Shared::new()));
        press(&mut ui, "m");
        render(&ui, 120, 40);
        let zone = |ui: &Ui, key: SortKey| ui.hit.menu_header.borrow().iter().find(|(_, k)| *k == key).unwrap().0;
        let z = zone(&ui, SortKey::Cylinders);
        ui.handle_mouse(mouse(MouseEventKind::Down(Left), z.x + 1, z.y));
        assert_eq!((ui.sort, ui.sort_reverse), (SortKey::Cylinders, false));
        render(&ui, 120, 40);
        ui.handle_mouse(mouse(MouseEventKind::Down(Left), z.x + 1, z.y));
        assert_eq!((ui.sort, ui.sort_reverse), (SortKey::Cylinders, true));
        assert!(ui.menu.is_some(), "sorting keeps the list open");
        let text = render(&ui, 120, 40);
        assert!(text.contains("Cyl.▼"), "the header shows the sort direction");
    }

    #[test]
    fn shift_e_goes_to_the_previous_view() {
        let mut ui = ui_with(Arc::new(Shared::new()));
        press(&mut ui, "e");
        assert!(ui.view == View::Cylinders);
        press(&mut ui, "E");
        assert!(ui.view == View::Scope);
        press(&mut ui, "E");
        assert!(ui.view == View::Front);
    }

    #[test]
    fn header_shows_the_nominal_power() {
        let ui = ui_with(Arc::new(Shared::new()));
        let text = render(&ui, 120, 34);
        let first = text.lines().nth(1).unwrap();
        let (kw, nm) = (ui.presets[0].power_kw, ui.presets[0].torque_nm);
        assert!(first.contains(&format!("{kw:.0} kW")) && first.contains(&format!("{nm:.0} N·m")), "{first}");
    }

    #[test]
    fn renders_shortcuts_popup() {
        let shared = Arc::new(Shared::new());
        let mut ui = ui_with(shared);
        ui.help = true;
        let text = render(&ui, 120, 40);
        assert!(text.contains("Keyboard shortcuts"));
        press(&mut ui, "?");
        assert!(!ui.help);
    }
}
