//! Driver controls state (throttle, requested gear, clutch, brake), shared by both
//! interfaces (TUI and GUI) so they behave the same way.

use std::sync::atomic::Ordering;

use crate::app::Shared;

#[derive(Default)]
pub struct Pilot {
    pub throttle: f32,
    pub gear_target: i32,
    pub clutch_latched: bool,
    /// The brake stays on until pressed again or until throttle is added
    /// (a terminal doesn't report key releases).
    pub brake_on: bool,
    /// Engine animation speed: index into `ANIM_MODES` (0 = automatic).
    pub anim_mode: usize,
}

impl Pilot {
    /// Sets the throttle; adding throttle releases the brake.
    pub fn set_throttle(&mut self, value: f32, shared: &Shared) {
        let value = value.clamp(0.0, 1.0);
        if value > self.throttle {
            self.brake_on = false;
        }
        self.throttle = value;
        shared.throttle.set(value);
    }

    pub fn shift(&mut self, delta: i32, gears: i32, shared: &Shared) {
        self.gear_target = (self.gear_target + delta).clamp(0, gears);
        shared.gear_request.store(self.gear_target, Ordering::Relaxed);
    }

    pub fn neutral(&mut self, shared: &Shared) {
        self.gear_target = 0;
        shared.gear_request.store(0, Ordering::Relaxed);
    }

    pub fn cycle_anim(&mut self) {
        self.anim_mode = (self.anim_mode + 1) % ANIM_MODES.len();
    }

    pub fn toggle_clutch(&mut self) {
        self.clutch_latched = !self.clutch_latched;
    }

    /// Braking also lifts the throttle.
    pub fn set_brake(&mut self, on: bool, shared: &Shared) {
        self.brake_on = on;
        if on {
            self.throttle = 0.0;
            shared.throttle.set(0.0);
        }
    }

    /// After an engine change or reload.
    pub fn reset(&mut self, shared: &Shared) {
        self.gear_target = 0;
        self.throttle = 0.0;
        self.clutch_latched = false;
        shared.throttle.set(0.0);
    }

    /// Call every frame: forwards brake and clutch to the simulation.
    pub fn sync(&self, shared: &Shared) {
        shared.brake.set(if self.brake_on { 1.0 } else { 0.0 });
        shared.clutch_pedal.set(if self.clutch_latched { 1.0 } else { 0.0 });
    }
}

/// Idle adjustment step (rpm); range is 50 % to 200 % of the engine's default idle.
pub const IDLE_STEP: f32 = 50.0;

pub fn idle_range(default_idle: f64) -> (f32, f32) {
    (default_idle as f32 * 0.5, default_idle as f32 * 2.0)
}

pub fn adjust_idle(shared: &Shared, default_idle: f64, delta: f32) {
    let (lo, hi) = idle_range(default_idle);
    shared.idle_rpm.set((shared.idle_rpm.get() + delta).clamp(lo, hi));
}

/// Animation speeds: automatic (slow, grows with rpm) or a fraction of real speed.
pub const ANIM_MODES: [(&str, Option<f64>); 7] = [
    ("Auto (slow motion)", None),
    ("Real time", Some(1.0)),
    ("Slow ×2", Some(0.5)),
    ("Slow ×5", Some(0.2)),
    ("Slow ×10", Some(0.1)),
    ("Slow ×50", Some(0.02)),
    ("Slow ×200", Some(0.005)),
];

/// Crankshaft revolutions displayed per second.
pub fn anim_rev_per_s(mode: usize, rpm: f64, redline: f64) -> f64 {
    match ANIM_MODES[mode % ANIM_MODES.len()].1 {
        None => 0.25 + 1.5 * (rpm / redline).min(1.2),
        Some(f) => rpm / 60.0 * f,
    }
}

/// Short text for the view title: "real time" or "slow ×N".
pub fn anim_label(mode: usize, rpm: f64, redline: f64) -> String {
    let shown = anim_rev_per_s(mode, rpm, redline);
    let ratio = if shown > 1e-6 { rpm / 60.0 / shown } else { 1.0 };
    if ratio <= 1.05 {
        "real time".to_string()
    } else {
        format!("slow ×{ratio:.0}")
    }
}
