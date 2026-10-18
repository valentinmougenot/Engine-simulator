//! Virtual driver: sequences clutch-out, gear change and clutch re-engagement
//! so gear changes sound natural.

use crate::engine::Gearbox;

#[derive(Clone, Copy, Debug)]
enum Phase {
    Idle,
    /// Pedal goes down, then a short pause before the target gear engages.
    Disengage(f64),
    /// Pedal released progressively over `duration` seconds.
    Engage { elapsed: f64, duration: f64 },
}

pub struct Driver {
    ratios: Vec<f64>,
    final_drive: f64,
    gear: usize,
    target: usize,
    phase: Phase,
    pedal: f64,
    shift_time: f64,
    /// Gear being left during the current change (tells upshifts from downshifts).
    from_gear: usize,
}

const PEDAL_DOWN: f64 = 0.09;
const PAUSE: f64 = 0.06;
const ENGAGE: f64 = 0.28;
const LAUNCH: f64 = 1.0;

impl Driver {
    pub fn new(gb: &Gearbox) -> Self {
        Self {
            ratios: gb.ratios.clone(),
            final_drive: gb.final_drive,
            gear: 0,
            target: 0,
            phase: Phase::Idle,
            pedal: 0.0,
            shift_time: gb.shift_time,
            from_gear: 0,
        }
    }

    pub fn gear_count(&self) -> usize {
        self.ratios.len()
    }

    /// Engaged gear (0 = neutral).
    pub fn gear(&self) -> usize {
        self.gear
    }

    /// Requests a gear (0 = neutral). Ignored if it doesn't exist.
    pub fn request(&mut self, gear: usize) {
        if gear > self.ratios.len() || (gear == self.target && !matches!(self.phase, Phase::Idle)) {
            return;
        }
        if gear == self.gear && matches!(self.phase, Phase::Idle) {
            return;
        }
        self.target = gear;
        self.from_gear = self.gear;
        self.phase = Phase::Disengage(0.0);
    }

    pub fn gear_ratio(&self) -> f64 {
        match self.gear {
            0 => 0.0,
            g => self.ratios[g - 1] * self.final_drive,
        }
    }

    /// Throttle actually applied during a gear change: cut on upshifts
    /// (like a quickshifter) and a blip on downshifts to match revs.
    pub fn shape_throttle(&self, user: f64) -> f64 {
        let upshift = self.target > self.from_gear && self.from_gear > 0;
        let downshift = self.target < self.from_gear && self.target > 0;
        match self.phase {
            Phase::Idle => user,
            Phase::Disengage(_) if upshift => 0.0,
            Phase::Disengage(_) if downshift => user.max(0.6),
            Phase::Engage { elapsed, duration } if upshift => {
                let x = (elapsed / (0.8 * duration)).min(1.0);
                user * x * x * (3.0 - 2.0 * x)
            }
            _ => user,
        }
    }

    /// Advances time. `manual_pedal`: 1 while the driver holds the clutch.
    /// Returns the clutch engagement (1 = engaged).
    pub fn update(&mut self, dt: f64, manual_pedal: f64, speed: f64) -> f64 {
        match self.phase {
            Phase::Idle => {}
            Phase::Disengage(t) => {
                let t = t + dt;
                self.pedal = (t / (PEDAL_DOWN * self.shift_time)).min(1.0);
                if t >= (PEDAL_DOWN + PAUSE) * self.shift_time {
                    self.gear = self.target;
                    let launch = speed < 1.5 && self.gear > 0;
                    self.phase = Phase::Engage {
                        elapsed: 0.0,
                        duration: if launch { LAUNCH } else { ENGAGE * self.shift_time },
                    };
                } else {
                    self.phase = Phase::Disengage(t);
                }
            }
            Phase::Engage { elapsed, duration } => {
                let e = elapsed + dt;
                let x = (e / duration).min(1.0);
                self.pedal = 1.0 - x * x * (3.0 - 2.0 * x);
                self.phase = if e >= duration { Phase::Idle } else { Phase::Engage { elapsed: e, duration } };
                if e >= duration {
                    self.pedal = 0.0;
                }
            }
        }
        1.0 - self.pedal.max(manual_pedal)
    }
}
