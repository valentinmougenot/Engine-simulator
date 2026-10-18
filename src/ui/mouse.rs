//! TUI mouse support: wheel, clicks and drags. The clickable areas are stored on every
//! draw (`Hit`) and compared with the event position.

use std::cell::{Cell, RefCell};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use super::{SortKey, Ui, View};
use crate::ambience::Mic;
use crate::pilot::{adjust_idle, IDLE_STEP};

/// Action attached to an item of the bottom bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BarAction {
    Clutch,
    Brake,
    Menu,
    View,
    Mic,
    Help,
    Quit,
}

/// Screen areas of the last draw.
#[derive(Default)]
pub(super) struct Hit {
    pub header: Cell<Rect>,
    pub gear: Cell<Rect>,
    /// Rows of the Throttle, Clutch and Brake gauges.
    pub pedals: [Cell<Rect>; 3],
    pub audio: Cell<Rect>,
    pub view: Cell<Rect>,
    pub tabs: RefCell<Vec<(Rect, View)>>,
    pub bar: RefCell<Vec<(Rect, BarAction)>>,
    /// Inside of the engine list, scroll offset and displayed indices.
    pub menu_popup: Cell<Rect>,
    pub menu_list: Cell<Rect>,
    pub menu_offset: Cell<usize>,
    pub menu_matches: RefCell<Vec<usize>>,
    /// Clickable sort-column titles of the engine list.
    pub menu_header: RefCell<Vec<(Rect, SortKey)>>,
}

impl Ui {
    /// Returns `true` to quit.
    pub(super) fn handle_mouse(&mut self, m: MouseEvent) -> bool {
        let pos = Position::new(m.column, m.row);
        if self.help {
            match m.kind {
                MouseEventKind::Down(_) => self.help = false,
                MouseEventKind::ScrollDown => self.help_scroll = self.help_scroll.saturating_add(1),
                MouseEventKind::ScrollUp => self.help_scroll = self.help_scroll.saturating_sub(1),
                _ => {}
            }
            return false;
        }
        if self.menu.is_some() {
            self.mouse_menu(m.kind, pos);
            return false;
        }
        match m.kind {
            MouseEventKind::ScrollUp => self.mouse_scroll(pos, true),
            MouseEventKind::ScrollDown => self.mouse_scroll(pos, false),
            MouseEventKind::Down(MouseButton::Left) => return self.mouse_left(pos),
            MouseEventKind::Down(MouseButton::Right) => {
                let s = Arc::clone(&self.shared);
                let gears = self.engine().gearbox.ratios.len() as i32;
                if self.hit.gear.get().contains(pos) {
                    self.pilot.shift(-1, gears, &s);
                } else if self.hit.view.get().contains(pos) {
                    self.view = self.view.prev();
                }
            }
            MouseEventKind::Down(MouseButton::Middle) => {
                if self.hit.gear.get().contains(pos) {
                    self.pilot.neutral(&self.shared);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if self.dragging_gas {
                    self.set_throttle_from_x(pos.x);
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.dragging_gas = false;
                if self.mouse_brake {
                    self.mouse_brake = false;
                    self.pilot.set_brake(false, &self.shared);
                }
            }
            _ => {}
        }
        false
    }

    fn set_throttle_from_x(&mut self, x: u16) {
        let row = self.hit.pedals[0].get();
        if row.width > 1 {
            let ratio = (x.saturating_sub(row.x)) as f32 / (row.width - 1) as f32;
            self.pilot.set_throttle(ratio, &self.shared);
        }
    }

    fn mouse_left(&mut self, pos: Position) -> bool {
        let s = Arc::clone(&self.shared);
        let gears = self.engine().gearbox.ratios.len() as i32;
        let bar: Vec<(Rect, BarAction)> = self.hit.bar.borrow().clone();
        if let Some((_, action)) = bar.iter().find(|(r, _)| r.contains(pos)) {
            match action {
                BarAction::Clutch => self.pilot.toggle_clutch(),
                BarAction::Brake => self.pilot.set_brake(!self.pilot.brake_on, &s),
                BarAction::Menu => self.open_menu(),
                BarAction::View => self.view = self.view.next(),
                BarAction::Mic => s.mic.store((s.mic.load(Ordering::Relaxed) + 1) % Mic::ALL.len(), Ordering::Relaxed),
                BarAction::Help => self.help = true,
                BarAction::Quit => return true,
            }
            return false;
        }
        let tab = self.hit.tabs.borrow().iter().find(|(r, _)| r.contains(pos)).map(|(_, v)| *v);
        if let Some(view) = tab {
            self.view = view;
        } else if self.hit.header.get().contains(pos) {
            self.open_menu();
        } else if self.hit.gear.get().contains(pos) {
            self.pilot.shift(1, gears, &s);
        } else if self.hit.pedals[0].get().contains(pos) {
            self.dragging_gas = true;
            self.set_throttle_from_x(pos.x);
        } else if self.hit.pedals[1].get().contains(pos) {
            self.pilot.toggle_clutch();
        } else if self.hit.pedals[2].get().contains(pos) {
            // Brake held while the button is down.
            self.mouse_brake = true;
            self.pilot.set_brake(true, &s);
        } else if self.hit.view.get().contains(pos) {
            self.view = self.view.next();
        }
        false
    }

    fn mouse_scroll(&mut self, pos: Position, up: bool) {
        let s = Arc::clone(&self.shared);
        let gears = self.engine().gearbox.ratios.len() as i32;
        let sign = if up { 1.0 } else { -1.0 };
        let audio = self.hit.audio.get();
        if self.hit.gear.get().contains(pos) {
            self.pilot.shift(if up { 1 } else { -1 }, gears, &s);
        } else if audio.contains(pos) {
            // One row = one setting: volume, convolution, (level), (throttle), idle.
            match pos.y.saturating_sub(audio.y + 1) {
                0 => s.volume.set((s.volume.get() + 0.05 * sign).clamp(0.0, 1.5)),
                1 => s.convolution.set((s.convolution.get() + 0.1 * sign).clamp(0.0, 1.0)),
                4 => adjust_idle(&s, self.engine().idle_rpm, IDLE_STEP * sign),
                _ => {}
            }
        } else {
            self.pilot.set_throttle(self.pilot.throttle + 0.05 * sign, &s);
        }
    }

    fn mouse_menu(&mut self, kind: MouseEventKind, pos: Position) {
        let Some(mut menu) = self.menu.take() else { return };
        let count = self.hit.menu_matches.borrow().len().max(1);
        match kind {
            MouseEventKind::ScrollDown => menu.cursor = (menu.cursor + 1).min(count - 1),
            MouseEventKind::ScrollUp => menu.cursor = menu.cursor.saturating_sub(1),
            MouseEventKind::Down(MouseButton::Left) => {
                let list = self.hit.menu_list.get();
                let column = self.hit.menu_header.borrow().iter().find(|(r, _)| r.contains(pos)).map(|(_, k)| *k);
                if let Some(key) = column {
                    self.sort_by(&mut menu, key);
                } else if list.contains(pos) {
                    let index = self.hit.menu_offset.get() + (pos.y - list.y) as usize;
                    let chosen = self.hit.menu_matches.borrow().get(index).copied();
                    if let Some(engine) = chosen {
                        self.choose_engine(engine);
                        return;
                    }
                } else if !self.hit.menu_popup.get().contains(pos) {
                    return;
                }
            }
            MouseEventKind::Down(MouseButton::Right) => return,
            _ => {}
        }
        self.menu = Some(menu);
    }
}
