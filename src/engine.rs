//! Simplified engine model whose only purpose is to produce realistic exhaust flows
//! for the synthesizer. Each cylinder is a single thermodynamic zone (ideal gas, Wiebe
//! combustion) linked to an exhaust runner, then a shared collector, then the atmosphere.

use std::f64::consts::PI;

use serde::{Deserialize, Serialize};

const R: f64 = 287.0;
const GAMMA: f64 = 1.35;
const CV: f64 = R / (GAMMA - 1.0);
const CP: f64 = CV + R;
const P_ATM: f64 = 101_325.0;
const T_AMBIENT: f64 = 300.0;
const SPEED_OF_SOUND: f64 = 343.0;
const LHV_FUEL: f64 = 43.0e6;
const STOICH_AFR: f64 = 14.7;
const SUBSTEPS: usize = 4;
/// Points of the P-V diagram of cylinder 1 (one per 720°/PV_BINS slice).
pub const PV_BINS: usize = 360;
/// Viscous clutch stiffness (N·m per rad/s of slip).
const CLUTCH_STIFFNESS: f64 = 500.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ir {
    Default0,
    Real0,
    Real1,
    Real2,
}

impl Ir {
    pub fn bytes(self) -> &'static [u8] {
        match self {
            Ir::Default0 => include_bytes!("../assets/ir/default_0.wav"),
            Ir::Real0 => include_bytes!("../assets/ir/real_engine_0.wav"),
            Ir::Real1 => include_bytes!("../assets/ir/real_engine_1.wav"),
            Ir::Real2 => include_bytes!("../assets/ir/real_engine_2.wav"),
        }
    }

    /// Same volume as the original `impulse_responses.mr`.
    pub fn volume(self) -> f32 {
        0.001
    }
}

fn one() -> f64 {
    1.0
}

fn default_exhaust_area() -> f64 {
    PI / 4.0 * 0.057 * 0.057
}

/// Names loaded from TOML files live for the whole program.
fn leak_str<'de, D: serde::Deserializer<'de>>(d: D) -> Result<&'static str, D::Error> {
    String::deserialize(d).map(|s| &*Box::leak(s.into_boxed_str()))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExhaustCfg {
    /// Pipe length after the collector (m).
    pub length: f64,
    pub audio_volume: f64,
    pub ir: Ir,
    /// Collector / outlet area (m²).
    #[serde(default = "default_exhaust_area")]
    pub area: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CylinderCfg {
    /// Crank angle (0..720°) at which this cylinder is at firing TDC.
    pub fire_deg: f64,
    pub exhaust: usize,
    /// Primary tube length (m).
    pub primary_length: f64,
    #[serde(default = "one")]
    pub attenuation: f64,
    /// Bank index (0, 1, ...), only used by the engine views.
    #[serde(default)]
    pub bank: usize,
}

/// Turbo: boost follows a turbine speed that rises with engine speed and load,
/// with a response lag.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Turbo {
    /// Maximum intake pressure / ambient pressure.
    pub max_boost: f64,
    pub spool_start_rpm: f64,
    pub spool_full_rpm: f64,
    /// Response time (s); very short with an MGU-H.
    pub lag: f64,
    /// Turbine whine level added to the sound (0 = none).
    pub whine: f64,
}

/// Electric assist (MGU-K): torque added to the crankshaft at full load.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Electric {
    pub power_kw: f64,
    pub max_torque: f64,
}

/// Drivetrain and vehicle: gearbox, final drive, wheels, mass and drag.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gearbox {
    /// Gear ratios (1st, 2nd, ...).
    pub ratios: Vec<f64>,
    pub final_drive: f64,
    pub wheel_radius: f64,
    pub vehicle_mass: f64,
    /// Drag area Cd·A (m²).
    pub drag_area: f64,
    /// Relative shift duration (1 = manual, 0.25 = sequential).
    pub shift_time: f64,
    /// Maximum grip of the driven wheels, in g.
    pub traction_g: f64,
    /// Downforce area Cz·A (m²): grip grows with speed squared.
    pub downforce_area: f64,
    /// Traction control: cuts the throttle when the wheels reach their grip limit
    /// instead of letting the engine run away.
    pub traction_control: bool,
}

impl Default for Gearbox {
    fn default() -> Self {
        Self {
            ratios: vec![3.6, 2.2, 1.5, 1.1, 0.85, 0.7],
            final_drive: 3.42,
            wheel_radius: 0.33,
            vehicle_mass: 1500.0,
            drag_area: 0.65,
            shift_time: 1.0,
            traction_g: 1.1,
            downforce_area: 0.0,
            traction_control: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineCfg {
    #[serde(deserialize_with = "leak_str")]
    pub name: &'static str,
    pub bore: f64,
    pub stroke: f64,
    pub rod_length: f64,
    pub compression_ratio: f64,
    /// Crankshaft + flywheel inertia (kg·m²).
    pub inertia: f64,
    pub redline_rpm: f64,
    pub idle_rpm: f64,
    /// Internal friction multiplier (< 1 for highly optimised race engines).
    pub friction_scale: f64,
    /// Minimum throttle opening held by the idle governor (anti-stall).
    pub idle_throttle_min: f64,
    /// Clutch capacity relative to 180 N·m per litre (raise for turbo engines).
    pub clutch_capacity_scale: f64,
    /// Fraction of the fuel energy actually released (0.62 ≈ road engine).
    pub combustion_efficiency: f64,
    /// Maximum fuel flow (kg/s) above 10,500 rpm; 0 = unlimited.
    /// F1 rule: 100 kg/h, reduced proportionally at low rpm.
    pub max_fuel_flow: f64,
    /// Typical burn duration, in crank degrees.
    pub burn_duration_deg: f64,
    /// Intake "ram" boost (resonance, trumpets); 1.0 = none.
    pub intake_boost: f64,
    /// Weight of the dynamic pressure in the sound signal (0 = static pressure only).
    pub dynamic_pressure_gain: f64,
    /// Share of the primary tube volume counted in the runner's lumped volume;
    /// the lower it is, the sharper the pulses.
    pub pipe_volume_fraction: f64,

    pub intake_duration_deg: f64,
    pub exhaust_duration_deg: f64,
    /// Lobe centre, in crank degrees before/after gas-exchange TDC.
    pub intake_center_deg: f64,
    pub exhaust_center_deg: f64,
    pub intake_lift: f64,
    pub exhaust_lift: f64,
    /// Valve diameter as a fraction of the bore.
    pub intake_valve_frac: f64,
    pub exhaust_valve_frac: f64,

    pub runner_volume: f64,
    pub primary_area: f64,

    /// Per-engine audio parameters (see `hf_gain`, `noise`, `jitter`).
    pub hf_gain: f32,
    pub noise: f32,
    pub jitter: f32,
    /// Cutoff of the input filter and of the air noise (Hz).
    pub input_cutoff: f32,
    pub air_noise_cutoff: f32,
    /// Final EQ, to match the tone to a reference recording.
    pub eq: Vec<crate::dsp::EqBand>,
    pub sim_rate: f64,

    pub cylinders: Vec<CylinderCfg>,
    pub exhausts: Vec<ExhaustCfg>,
    pub gearbox: Gearbox,
    pub turbo: Option<Turbo>,
    pub electric: Option<Electric>,
    /// Anti-lag (rally): on lift-off, keep the throttle slightly open and fire very late
    /// so combustion happens in the exhaust (keeps the turbo spooled, makes pops).
    pub anti_lag: bool,
    /// Nominal peak power (kW), shown in the engine list; 0 = unknown. Not used by the simulation.
    #[serde(default)]
    pub power_kw: f64,
    /// Nominal peak torque (N·m), shown next to the power; 0 = unknown. Not used by the simulation.
    #[serde(default)]
    pub torque_nm: f64,
    /// Angle between banks (0 = inline, 180 = flat), for the engine views.
    #[serde(default)]
    pub bank_angle_deg: f64,
}

impl EngineCfg {
    /// Piston position at cycle angle `phi` (0 = firing TDC): 0 at TDC, 1 at BDC.
    pub fn piston_fraction(&self, phi: f64) -> f64 {
        let a = self.stroke / 2.0;
        let l = self.rod_length;
        let x = a * phi.cos() + (l * l - a * a * phi.sin().powi(2)).sqrt();
        ((a + l - x) / self.stroke).clamp(0.0, 1.0)
    }

    /// Intake and exhaust valve opening (0..1) at angle `phi`.
    pub fn valve_openings(&self, phi: f64) -> (f64, f64) {
        let ex_center = 2.0 * PI - self.exhaust_center_deg.to_radians();
        let in_center = -(2.0 * PI - self.intake_center_deg.to_radians());
        (
            EngineSim::lift(phi, in_center, self.intake_duration_deg.to_radians(), 1.0),
            EngineSim::lift(phi, ex_center, self.exhaust_duration_deg.to_radians(), 1.0),
        )
    }

    pub fn displacement(&self) -> f64 {
        self.cylinders.len() as f64 * PI / 4.0 * self.bore * self.bore * self.stroke
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Controls {
    pub throttle: f64,
    pub ignition: bool,
    pub starter: bool,
    /// Total ratio (gearbox × final drive); 0 = neutral.
    pub gear_ratio: f64,
    /// 1 = engaged, 0 = disengaged.
    pub clutch: f64,
    /// 0..1, 1 = full braking.
    pub brake: f64,
}

struct Cylinder {
    m: f64,
    t: f64,
    v: f64,
    phi: f64,
    q_total: f64,
    burn_x: f64,
    runner_m: f64,
    runner_t: f64,
    /// Mass flowed through the exhaust valve during the current step (kg).
    out_mass: f64,
    delay: Vec<f64>,
    delay_pos: usize,
}

struct Collector {
    volume: f64,
    m: f64,
    t: f64,
}

pub struct EngineSim {
    cfg: EngineCfg,
    /// Normalised turbine speed (0..1).
    spool: f64,
    lag_rng: u32,
    /// Target idle speed (adjustable at runtime, defaults to `cfg.idle_rpm`).
    idle_target: f64,
    /// Smoothed engine torque at the crankshaft (gas + electric - friction), N·m.
    brake_torque: f64,
    firing_now: bool,
    pv: Vec<[f32; 2]>,
    boost_dbg: f64,
    /// Traction-control throttle factor (1 = no cut) and last-iteration state.
    tc_scale: f64,
    /// Current transmission capacity (grip/clutch) and average clutch torque.
    tc_cap: f64,
    clutch_avg: f64,
    cyl: Vec<Cylinder>,
    collectors: Vec<Collector>,
    runner_volume: Vec<f64>,
    /// Angle in the 4-stroke cycle, [0, 4π).
    theta: f64,
    omega: f64,
    /// Vehicle speed (m/s).
    speed: f64,
    last_throttle: f64,
    pub dbg: [f64; 4],
    filtered_omega: f64,
    idle_throttle: f64,
    limiter: bool,
    plenum_m: f64,
    plenum_volume: f64,
    dt: f64,
    piston_area: f64,
    crank_radius: f64,
    clearance_volume: f64,
}

#[inline]
fn wrap_cycle(x: f64) -> f64 {
    x - 4.0 * PI * (x / (4.0 * PI)).round()
}

/// Mass flow through an orifice (isentropic, possibly choked).
/// Returns a positive flow from upstream to downstream.
#[inline]
fn orifice(area: f64, p_up: f64, t_up: f64, p_down: f64) -> f64 {
    if area <= 0.0 || p_up <= p_down {
        return 0.0;
    }
    let pr = p_down / p_up;
    let crit = (2.0 / (GAMMA + 1.0)).powf(GAMMA / (GAMMA - 1.0));
    let flux = if pr <= crit {
        GAMMA.sqrt() * (2.0 / (GAMMA + 1.0)).powf((GAMMA + 1.0) / (2.0 * (GAMMA - 1.0)))
    } else {
        (2.0 * GAMMA / (GAMMA - 1.0) * (pr.powf(2.0 / GAMMA) - pr.powf((GAMMA + 1.0) / GAMMA))).sqrt()
    };
    area * p_up / (R * t_up).sqrt() * flux
}

/// Mass transferred from `a` to `b` during `dt` (signed), limited to the mass that
/// equalises the pressures, so the explicit scheme stays stable.
#[inline]
fn flow(area: f64, (pa, ta, va): (f64, f64, f64), (pb, tb, vb): (f64, f64, f64), dt: f64) -> f64 {
    let (rate, sign) = if pa >= pb {
        (orifice(area, pa, ta, pb), 1.0)
    } else {
        (orifice(area, pb, tb, pa), -1.0)
    };
    let eq = (pa - pb).abs() / (R * ta.max(tb)) * va * vb / (va + vb);
    sign * (rate * dt).min(eq)
}

impl EngineSim {
    pub fn new(cfg: &EngineCfg) -> Self {
        let dt = 1.0 / cfg.sim_rate;
        let piston_area = PI / 4.0 * cfg.bore * cfg.bore;
        let crank_radius = cfg.stroke / 2.0;
        let clearance_volume = piston_area * cfg.stroke / (cfg.compression_ratio - 1.0);

        let mut sim = Self {
            cfg: cfg.clone(),
            spool: 0.0,
            lag_rng: 0x2545_F491,
            idle_target: cfg.idle_rpm,
            brake_torque: 0.0,
            firing_now: false,
            pv: vec![[0.0; 2]; PV_BINS],
            boost_dbg: 1.0,
            tc_scale: 1.0,
            tc_cap: f64::INFINITY,
            clutch_avg: 0.0,
            cyl: Vec::new(),
            collectors: Vec::new(),
            runner_volume: Vec::new(),
            theta: 0.0,
            omega: 0.0,
            speed: 0.0,
            last_throttle: 0.0,
            dbg: [0.0; 4],
            filtered_omega: 0.0,
            idle_throttle: 0.0,
            limiter: false,
            plenum_m: 0.0,
            plenum_volume: 0.0,
            dt,
            piston_area,
            crank_radius,
            clearance_volume,
        };

        sim.plenum_volume = 0.5 * cfg.displacement();
        sim.plenum_m = P_ATM * sim.plenum_volume / (R * T_AMBIENT);

        for e in &cfg.exhausts {
            let volume = e.area * e.length.max(0.2);
            sim.collectors.push(Collector {
                volume,
                m: P_ATM * volume / (R * T_AMBIENT),
                t: T_AMBIENT,
            });
        }

        for c in &cfg.cylinders {
            let rv = cfg.runner_volume + cfg.primary_area * c.primary_length * cfg.pipe_volume_fraction;
            let total_len = c.primary_length + cfg.exhausts[c.exhaust].length;
            let delay_samples = (total_len / SPEED_OF_SOUND * cfg.sim_rate).round() as usize;
            let v = sim.volume(0.0);
            sim.cyl.push(Cylinder {
                m: P_ATM * v / (R * T_AMBIENT),
                t: T_AMBIENT,
                v,
                phi: 0.0,
                q_total: 0.0,
                burn_x: 0.0,
                runner_m: P_ATM * rv / (R * T_AMBIENT),
                runner_t: T_AMBIENT,
                out_mass: 0.0,
                delay: vec![0.0; delay_samples + 1],
                delay_pos: 0,
            });
            sim.runner_volume.push(rv);
        }
        sim
    }

    pub fn rpm(&self) -> f64 {
        self.omega * 60.0 / (2.0 * PI)
    }

    /// Engine torque at the crankshaft (N·m), smoothed over a fraction of a second. Negative on overrun.
    pub fn torque_nm(&self) -> f64 {
        self.brake_torque
    }

    /// Engine power at the crankshaft (kW).
    pub fn power_kw(&self) -> f64 {
        self.brake_torque * self.omega / 1000.0
    }

    /// Sets the target idle speed (rpm).
    pub fn set_idle_rpm(&mut self, rpm: f64) {
        self.idle_target = rpm;
    }

    /// Last cycle of cylinder 1: (volume m³, pressure Pa) per crank-angle slice.
    pub fn pv_loop(&self) -> &[[f32; 2]] {
        &self.pv
    }

    /// True while the ECU injects and fires (false on fuel cut, rev limiter or ignition off).
    pub fn is_firing(&self) -> bool {
        self.firing_now
    }

    /// Normalised turbine speed (0 without a turbo).
    pub fn turbo_spool(&self) -> f64 {
        self.spool
    }

    pub fn speed_kmh(&self) -> f64 {
        self.speed * 3.6
    }

    /// Actual throttle opening (user or idle governor).
    pub fn throttle(&self) -> f64 {
        self.last_throttle
    }

    /// Cylinder volume at crank angle `phi` from TDC.
    fn volume(&self, phi: f64) -> f64 {
        let a = self.crank_radius;
        let l = self.cfg.rod_length;
        let x = a * phi.cos() + (l * l - a * a * phi.sin().powi(2)).sqrt();
        self.clearance_volume + self.piston_area * (a + l - x)
    }

    fn dvolume_dphi(&self, phi: f64) -> f64 {
        let a = self.crank_radius;
        let l = self.cfg.rod_length;
        let s = phi.sin();
        let root = (l * l - a * a * s * s).sqrt();
        self.piston_area * (a * s + a * a * s * phi.cos() / root)
    }

    /// Valve lift of a raised-cosine lobe centred on `center` (rad, cycle).
    pub(crate) fn lift(phi: f64, center: f64, duration: f64, max_lift: f64) -> f64 {
        let d = wrap_cycle(phi - center);
        let x = d / (duration / 2.0);
        if x.abs() >= 1.0 {
            0.0
        } else {
            max_lift * (0.5 * (1.0 + (PI * x).cos())).powf(1.1)
        }
    }

    fn valve_area(lift: f64, diameter: f64) -> f64 {
        const CD: f64 = 0.65;
        let curtain = PI * diameter * lift;
        let port = PI * diameter * diameter / 4.0;
        CD * curtain.min(port * 0.9)
    }

    fn spark_advance(&self, rpm: f64) -> f64 {
        (10.0 + 24.0 * (rpm / 3500.0).min(1.0)).to_radians()
    }

    /// Advances one synthesizer input step. `out[i]` receives the sound flow
    /// (before filtering) of exhaust `i`.
    pub fn step(&mut self, ctl: &Controls, out: &mut [f64]) {
        let dt_sub = self.dt / SUBSTEPS as f64;
        let rpm = self.rpm();

        // Idle governor: integral + proportional, active while the engine is running.
        if ctl.ignition && !ctl.starter {
            let err = self.idle_target - rpm;
            self.idle_throttle =
                (self.idle_throttle + 1.5e-4 * err * self.dt).clamp(self.cfg.idle_throttle_min, 0.35);
        } else if ctl.starter {
            // Cranking: throttle slightly open so the engine catches.
            self.idle_throttle = 0.12;
        } else {
            self.idle_throttle = 0.0;
        }
        let anti_lag = self.cfg.anti_lag
            && self.cfg.turbo.is_some()
            && ctl.ignition
            && ctl.throttle < 0.02
            && rpm > 0.4 * self.cfg.redline_rpm
            && !self.limiter;
        let governor = (self.idle_throttle + 1.0e-4 * (self.idle_target - rpm)).clamp(0.0, 0.35);
        let mut throttle = ctl.throttle.max(governor).clamp(0.0, 1.0);
        if self.cfg.gearbox.traction_control {
            // Progressive traction control: aim for an engine torque just below what
            // the wheels can transmit, rather than letting the clutch slip.
            let target = if ctl.gear_ratio > 0.0 && ctl.clutch > 0.9 {
                let unscaled = (self.clutch_avg.abs() / self.tc_scale).max(1.0);
                (0.97 * self.tc_cap / unscaled).clamp(0.15, 1.0)
            } else {
                1.0
            };
            self.tc_scale += (target - self.tc_scale) * (self.dt / 0.12).min(1.0);
            throttle *= self.tc_scale;
        }
        if anti_lag {
            throttle = throttle.max(0.10);
        }
        self.last_throttle = throttle;

        // Turbo: turbine speed follows engine speed and load, with a lag.
        let boost = match &self.cfg.turbo {
            Some(t) => {
                let rpm_part = ((rpm - t.spool_start_rpm) / (t.spool_full_rpm - t.spool_start_rpm)).clamp(0.0, 1.0);
                let spool_throttle = if anti_lag { throttle.max(0.5) } else { throttle };
                let target = rpm_part * (0.1 + 0.9 * spool_throttle.sqrt());
                self.spool += (target - self.spool) * (self.dt / t.lag).min(1.0);
                1.0 + (t.max_boost - 1.0) * self.spool
            }
            None => 1.0,
        };
        let boost_factor = self.cfg.intake_boost * boost;
        self.boost_dbg = boost_factor;

        if rpm > self.cfg.redline_rpm {
            self.limiter = true;
        } else if rpm < self.cfg.redline_rpm - 150.0 {
            self.limiter = false;
        }
        // Fuel cut on deceleration (like a real ECU).
        let fuel_cut = ctl.throttle < 0.02 && rpm > self.idle_target * 1.5 && !anti_lag;
        let fire_enabled = ctl.ignition && !self.limiter && !fuel_cut;
        self.firing_now = fire_enabled;

        // Throttle area sized for the airflow at redline (+50 % margin).
        let max_air_flow = 1.1 * 1.2 * self.cfg.displacement() * self.cfg.redline_rpm / 120.0;
        let idle_bleed = 0.006 * (self.cfg.idle_rpm / 900.0) * (6500.0 / self.cfg.redline_rpm);
        let throttle_area = 1.5 * max_air_flow / 235.0 * (idle_bleed + (1.0 - idle_bleed) * throttle.powi(2));
        let advance = self.spark_advance(rpm);
        let mut burn_duration = (self.cfg.burn_duration_deg * (0.85 + 0.15 * (rpm / 6000.0).min(1.0))).to_radians();
        // Anti-lag: spark at 105° after TDC, just before the exhaust opens.
        let spark = if anti_lag {
            burn_duration = 20.0_f64.to_radians();
            105.0_f64.to_radians()
        } else {
            -advance
        };

        let ex_valve_d = self.cfg.exhaust_valve_frac * self.cfg.bore;
        let in_valve_d = self.cfg.intake_valve_frac * self.cfg.bore;
        let ex_center = 2.0 * PI - self.cfg.exhaust_center_deg.to_radians();
        let in_center = -(2.0 * PI - self.cfg.intake_center_deg.to_radians());
        let ex_dur = self.cfg.exhaust_duration_deg.to_radians();
        let in_dur = self.cfg.intake_duration_deg.to_radians();
        let n_cylinders = self.cfg.cylinders.len() as f64;
        let loss_coeff = 6.0 * (self.cfg.bore / 0.096).powi(2);

        let gb = &self.cfg.gearbox;
        let clutch_capacity = 180.0 * self.cfg.clutch_capacity_scale * self.cfg.displacement() * 1e3 * ctl.clutch.clamp(0.0, 1.0).powi(2);
        let (wheel_radius, mass, drag_area, traction_g, downforce_area) =
            (gb.wheel_radius, gb.vehicle_mass, gb.drag_area, gb.traction_g, gb.downforce_area);

        for cy in &mut self.cyl {
            cy.out_mass = 0.0;
        }
        let mut brake_sum = 0.0;
        for _ in 0..SUBSTEPS {
            let p_man = self.plenum_m * R * T_AMBIENT / self.plenum_volume;
            let mut plenum_in = 0.0;
            let dtheta = self.omega * dt_sub;
            let mut torque = 0.0;

            for i in 0..self.cyl.len() {
                let cc = &self.cfg.cylinders[i];
                let phi_old = self.cyl[i].phi;
                let fire = cc.fire_deg.to_radians();
                let phi = wrap_cycle(self.theta + dtheta - fire);
                let phi_mid = wrap_cycle(self.theta + 0.5 * dtheta - fire);

                let v_new = self.volume(phi);
                let ex_lift = Self::lift(phi_mid, ex_center, ex_dur, self.cfg.exhaust_lift);
                let in_lift = Self::lift(phi_mid, in_center, in_dur, self.cfg.intake_lift);
                let a_ex = Self::valve_area(ex_lift, ex_valve_d);
                let a_in = Self::valve_area(in_lift, in_valve_d);

                let rv = self.runner_volume[i];
                let ex_idx = cc.exhaust;
                let prim_area = self.cfg.primary_area;

                let cy = &mut self.cyl[i];
                let p = cy.m * R * cy.t / cy.v;
                let p_run = cy.runner_m * R * cy.runner_t / rv;

                // Flows in kg: `dm_in` enters the cylinder, `dm_out` goes to the runner.
                let dm_in = if a_in > 0.0 {
                    flow(a_in, (p_man, T_AMBIENT, self.plenum_volume), (p, cy.t, cy.v), dt_sub)
                } else {
                    0.0
                };
                let dm_out = if a_ex > 0.0 {
                    flow(a_ex, (p, cy.t, cy.v), (p_run, cy.runner_t, rv), dt_sub)
                } else {
                    0.0
                };

                let col = &mut self.collectors[ex_idx];
                let p_col = col.m * R * col.t / col.volume;
                let dm_prim = flow(
                    prim_area,
                    (p_run, cy.runner_t, rv),
                    (p_col, col.t, col.volume),
                    dt_sub,
                );

                plenum_in -= dm_in;

                // Combustion: arm the heat release when the spark fires.
                // Anti-lag: only every other cycle, for irregular bursts.
                self.lag_rng ^= self.lag_rng << 13;
                self.lag_rng ^= self.lag_rng >> 17;
                self.lag_rng ^= self.lag_rng << 5;
                let lag_skip = anti_lag && self.lag_rng & 1 == 0;
                if fire_enabled && !lag_skip && cy.q_total == 0.0 && phi_old < spark && phi >= spark {
                    let mut fuel = cy.m / (1.0 + STOICH_AFR);
                    if self.cfg.max_fuel_flow > 0.0 {
                        let share = if rpm < 10_500.0 { (0.009 * rpm + 5.5) / 100.0 } else { 1.0 };
                        let per_cycle = self.cfg.max_fuel_flow * share / (rpm.max(300.0) / 120.0 * n_cylinders);
                        fuel = fuel.min(per_cycle);
                    }
                    cy.q_total = fuel * LHV_FUEL * self.cfg.combustion_efficiency;
                    cy.burn_x = 0.0;
                }
                let mut d_q = 0.0;
                if cy.q_total > 0.0 {
                    let s = ((phi - spark) / burn_duration).clamp(0.0, 1.0);
                    let x = if s >= 1.0 { 1.0 } else { 1.0 - (-5.0 * s.powi(3)).exp() };
                    d_q = cy.q_total * (x - cy.burn_x);
                    cy.burn_x = x;
                    if s >= 1.0 {
                        cy.q_total = 0.0;
                    }
                }
                d_q -= loss_coeff * (cy.t - 450.0) * dt_sub;

                // First law: m cv dT = -p dV + dQ + sum (cp T_src - cv T) dm_in.
                let d_v = v_new - cy.v;
                let h_in = if dm_in > 0.0 { CP * T_AMBIENT } else { CP * cy.t };
                let h_ex = if dm_out < 0.0 { CP * cy.runner_t } else { CP * cy.t };
                let d_t = (-p * d_v
                    + d_q
                    + (h_in - CV * cy.t) * dm_in
                    + (h_ex - CV * cy.t) * -dm_out)
                    / (cy.m * CV);
                cy.m = (cy.m + dm_in - dm_out).max(1e-7);
                cy.t = (cy.t + d_t).clamp(250.0, 3800.0);
                cy.v = v_new;
                cy.phi = phi;
                if i == 0 {
                    let bin = ((phi + 2.0 * PI) / (4.0 * PI) * PV_BINS as f64) as usize;
                    self.pv[bin.min(PV_BINS - 1)] = [v_new as f32, p as f32];
                }
                cy.out_mass += dm_out;

                // Runner: enthalpy mixing, relaxation towards the wall temperature.
                if dm_out > 0.0 {
                    cy.runner_t =
                        (cy.runner_m * cy.runner_t + dm_out * cy.t) / (cy.runner_m + dm_out);
                }
                cy.runner_m = (cy.runner_m + dm_out - dm_prim).max(1e-8);
                cy.runner_t += (550.0 - cy.runner_t) * (dt_sub / 0.08);

                let col = &mut self.collectors[ex_idx];
                if dm_prim > 0.0 {
                    col.t = (col.m * col.t + dm_prim * cy.runner_t) / (col.m + dm_prim);
                }
                col.m += dm_prim;

                torque += (p - P_ATM) * self.dvolume_dphi(phi);
            }

            // Throttle -> intake plenum.
            let thr_flow = flow(
                throttle_area,
                (P_ATM * boost_factor, T_AMBIENT, 1.0e3),
                (p_man, T_AMBIENT, self.plenum_volume),
                dt_sub,
            );
            self.plenum_m = (self.plenum_m + thr_flow + plenum_in).max(1e-6);

            // Collectors -> atmosphere.
            for (col, ex) in self.collectors.iter_mut().zip(&self.cfg.exhausts) {
                let p_col = col.m * R * col.t / col.volume;
                let area = 0.8 * ex.area;
                let dm = flow(
                    area,
                    (p_col, col.t, col.volume),
                    (P_ATM, T_AMBIENT, 1.0e3),
                    dt_sub,
                );
                col.m = (col.m - dm).max(1e-8);
                if dm < 0.0 {
                    col.t = (col.m * col.t - dm * T_AMBIENT) / (col.m - dm);
                }
                col.t += (500.0 - col.t) * (dt_sub / 0.15);
            }

            // Clutch: viscous torque capped by the clutch capacity.
            let clutch_torque = if ctl.gear_ratio > 0.0 {
                let w_in = self.speed / wheel_radius * ctl.gear_ratio;
                // Wheel slip caps the transmitted torque like the clutch does.
                let normal_force = mass * 9.81 + 0.5 * 1.2 * downforce_area * self.speed * self.speed;
                let traction_cap = traction_g * normal_force * wheel_radius / ctl.gear_ratio;
                let cap = clutch_capacity.min(traction_cap);
                self.tc_cap = cap;
                (CLUTCH_STIFFNESS * (self.omega - w_in)).clamp(-cap, cap)
            } else {
                0.0
            };
            self.dbg = [torque, clutch_torque, self.boost_dbg, 0.999 * self.dbg[3] + 0.001 * torque];
            self.clutch_avg = 0.999 * self.clutch_avg + 0.001 * clutch_torque;
            let gas_torque = torque;
            let mut electric_torque = 0.0;
            torque -= clutch_torque;

            // MGU-K: electric torque at full load, limited in power and torque.
            if let Some(el) = &self.cfg.electric {
                if self.omega > 50.0 && ctl.ignition && !self.limiter {
                    let deploy = ((throttle - 0.6) / 0.4).clamp(0.0, 1.0);
                    electric_torque = deploy * (el.power_kw * 1e3 / self.omega).min(el.max_torque);
                    torque += electric_torque;
                }
            }

            // Vehicle: wheel thrust, drag, rolling resistance, brakes.
            let drive_force = clutch_torque * ctl.gear_ratio / wheel_radius;
            let drag = 0.5 * 1.2 * drag_area * self.speed * self.speed;
            let resist = if self.speed > 0.05 {
                drag + 0.012 * mass * 9.81 + ctl.brake.clamp(0.0, 1.0) * mass * 9.81 * 0.9
            } else {
                0.0
            };
            self.speed = (self.speed + (drive_force - resist) / mass * dt_sub).max(0.0);

            // Crankshaft dynamics.
            let rpm_now = self.omega * 60.0 / (2.0 * PI);
            let fmep = 1.5e5 + 0.12e5 * rpm_now / 1000.0;
            let displacement = self.cfg.displacement();
            let friction = self.cfg.friction_scale * fmep * displacement / (4.0 * PI);
            let starter = if ctl.starter {
                let w_start = 500.0 * 2.0 * PI / 60.0;
                80_000.0 * displacement * (1.0 - self.omega / w_start).max(0.0)
            } else {
                0.0
            };
            let friction_torque = friction * (self.omega / (self.omega + 3.0));
            brake_sum += gas_torque + electric_torque - friction_torque;
            let total = torque + starter - friction_torque;
            self.omega = (self.omega + total / self.cfg.inertia * dt_sub).max(0.0);
            self.theta = (self.theta + dtheta).rem_euclid(4.0 * PI);
        }

        self.brake_torque += (brake_sum / SUBSTEPS as f64 - self.brake_torque) * (self.dt / 0.15).min(1.0);
        self.filtered_omega += (self.omega - self.filtered_omega) * (self.dt / 0.05).min(1.0);
        let attenuation = (self.filtered_omega.abs().min(40.0)) / 40.0;
        let attenuation3 = attenuation * attenuation * attenuation;

        out.iter_mut().for_each(|o| *o = 0.0);
        for i in 0..self.cyl.len() {
            let cc = &self.cfg.cylinders[i];
            let cy = &mut self.cyl[i];
            let p_run = cy.runner_m * R * cy.runner_t / self.runner_volume[i];
            // Signed dynamic pressure ½ρv²: gives the pulses their steep front,
            // which the runner's lumped volume would round off too much.
            let rho = p_run / (R * cy.runner_t);
            let velocity = cy.out_mass / self.dt / (rho * self.cfg.primary_area);
            let dynamic = 0.5 * rho * velocity * velocity.abs();
            let flow_signal = attenuation3
                * 1600.0
                * (p_run - P_ATM + self.cfg.dynamic_pressure_gain * dynamic);

            let delayed = cy.delay[cy.delay_pos];
            cy.delay[cy.delay_pos] = flow_signal;
            cy.delay_pos = (cy.delay_pos + 1) % cy.delay.len();

            let ex = &self.cfg.exhausts[cc.exhaust];
            let length = cc.primary_length + ex.length;
            out[cc.exhaust] += cc.attenuation
                * (ex.audio_volume * delayed / self.cyl.len() as f64)
                / (length * length);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets;

    fn preset(id: &str) -> EngineCfg {
        presets::all().into_iter().find(|e| e.id() == id).unwrap_or_else(|| panic!("no preset {id}"))
    }

    fn run(sim: &mut EngineSim, cfg: &EngineCfg, ctl: Controls, seconds: f64) -> Vec<f64> {
        let mut out = vec![0.0; cfg.exhausts.len()];
        for _ in 0..(seconds * cfg.sim_rate) as usize {
            sim.step(&ctl, &mut out);
        }
        out
    }

    fn idle_ctl() -> Controls {
        Controls { ignition: true, clutch: 1.0, ..Controls::default() }
    }

    /// Cranks until the engine catches (at most 4 s), like the app does, then lets it settle at idle.
    fn started(id: &str) -> (EngineCfg, EngineSim) {
        started_with(preset(id))
    }

    fn started_with(cfg: EngineCfg) -> (EngineCfg, EngineSim) {
        let mut sim = EngineSim::new(&cfg);
        let crank = Controls { starter: true, ..idle_ctl() };
        let mut t = 0.0;
        while sim.rpm() < 500.0 && t < 4.0 {
            run(&mut sim, &cfg, crank, 0.05);
            t += 0.05;
        }
        run(&mut sim, &cfg, idle_ctl(), 2.5);
        (cfg, sim)
    }

    // --- Numerical helpers ---------------------------------------------------------------

    #[test]
    fn wrap_cycle_stays_in_half_open_cycle() {
        for k in -20..20 {
            let x = k as f64 * 0.77;
            let w = wrap_cycle(x);
            assert!((-2.0 * PI..=2.0 * PI).contains(&w), "{x} -> {w}");
            // Same position in the cycle, whatever the number of turns.
            assert!(((x - w) / (4.0 * PI)).fract().abs() < 1e-9 || (((x - w) / (4.0 * PI)).fract().abs() - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn orifice_flows_downhill_only() {
        assert_eq!(orifice(1e-3, 1.0e5, 300.0, 1.0e5), 0.0, "no pressure difference");
        assert_eq!(orifice(1e-3, 0.9e5, 300.0, 1.0e5), 0.0, "never uphill");
        assert_eq!(orifice(0.0, 2.0e5, 300.0, 1.0e5), 0.0, "closed valve");
        assert!(orifice(1e-3, 1.5e5, 300.0, 1.0e5) > 0.0);
    }

    #[test]
    fn orifice_chokes() {
        // Below the critical pressure ratio the flow no longer depends on the downstream pressure.
        let a = orifice(1e-3, 3.0e5, 300.0, 0.5e5);
        let b = orifice(1e-3, 3.0e5, 300.0, 1.0e5);
        assert!((a - b).abs() / a < 1e-9, "{a} vs {b}");
        // Subsonic: flow grows as the downstream pressure drops.
        assert!(orifice(1e-3, 1.4e5, 300.0, 1.0e5) < orifice(1e-3, 1.4e5, 300.0, 0.8e5));
    }

    #[test]
    fn flow_is_antisymmetric_and_never_overshoots() {
        let a = (2.0e5, 300.0, 1.0e-3);
        let b = (1.0e5, 300.0, 1.0e-3);
        let ab = flow(1e-3, a, b, 1e-5);
        let ba = flow(1e-3, b, a, 1e-5);
        assert!(ab > 0.0 && (ab + ba).abs() < 1e-15);
        // A huge time step is clamped to the mass that equalises the two volumes.
        let huge = flow(1.0, a, b, 10.0);
        let equal = (a.0 - b.0) / (R * 300.0) * a.2 * b.2 / (a.2 + b.2);
        assert!((huge - equal).abs() < 1e-12, "{huge} vs {equal}");
    }

    // --- Geometry and valves -------------------------------------------------------------

    #[test]
    fn piston_travels_from_tdc_to_bdc() {
        let cfg = preset("gm_ls_v8");
        assert!(cfg.piston_fraction(0.0).abs() < 1e-9);
        assert!((cfg.piston_fraction(PI) - 1.0).abs() < 1e-9);
        assert!((cfg.piston_fraction(0.7) - cfg.piston_fraction(-0.7)).abs() < 1e-12, "symmetric about TDC");
        assert!((0.0..=1.0).contains(&cfg.piston_fraction(2.2)));
        // The piston is past half stroke before 90° crank angle (rod effect).
        assert!(cfg.piston_fraction(PI / 2.0) > 0.5);
    }

    #[test]
    fn valves_open_in_their_own_strokes() {
        let cfg = preset("gm_ls_v8");
        let in_c = -(2.0 * PI - cfg.intake_center_deg.to_radians());
        let ex_c = 2.0 * PI - cfg.exhaust_center_deg.to_radians();
        let (i, e) = cfg.valve_openings(in_c);
        assert!(i > 0.99 && e < 0.01, "intake centre: {i} {e}");
        let (i, e) = cfg.valve_openings(wrap_cycle(ex_c));
        assert!(e > 0.99 && i < 0.01, "exhaust centre: {i} {e}");
        // Both closed during compression and expansion.
        for deg in [-60.0_f64, -30.0, 30.0, 60.0] {
            let (i, e) = cfg.valve_openings(deg.to_radians());
            assert!(i < 1e-9 && e < 1e-9, "{deg}°: {i} {e}");
        }
    }

    // --- Starting, idling, limiting ----------------------------------------------------------

    #[test]
    fn starts_and_idles_near_target() {
        for id in ["gm_ls_v8", "yamaha_yzf_r1_cp4", "f1_2_4l_v8", "toyota_2jz_i6"] {
            let (cfg, mut sim) = started(id);
            run(&mut sim, &cfg, idle_ctl(), 1.0);
            let rpm = sim.rpm();
            assert!(
                (rpm - cfg.idle_rpm).abs() < 0.2 * cfg.idle_rpm,
                "{id}: idle {rpm:.0} rpm, target {:.0}",
                cfg.idle_rpm
            );
            assert!(sim.is_firing(), "{id} should be firing at idle");
        }
    }

    #[test]
    fn exhaust_signal_is_finite_and_alive_when_running() {
        let (cfg, mut sim) = started("gm_ls_v8");
        let mut out = vec![0.0; cfg.exhausts.len()];
        let mut peak: f64 = 0.0;
        for _ in 0..(0.5 * cfg.sim_rate) as usize {
            sim.step(&idle_ctl(), &mut out);
            assert!(out.iter().all(|v| v.is_finite()));
            peak = peak.max(out.iter().fold(0.0, |m, v| m.max(v.abs())));
        }
        assert!(peak > 0.0, "a running engine must push air out of the exhaust");
    }

    #[test]
    fn ignition_off_stops_the_engine() {
        let (cfg, mut sim) = started("gm_ls_v8");
        let off = Controls { ignition: false, ..idle_ctl() };
        run(&mut sim, &cfg, off, 0.3);
        assert!(!sim.is_firing());
        let before = sim.rpm();
        run(&mut sim, &cfg, off, 1.5);
        assert!(sim.rpm() < before.min(cfg.idle_rpm), "engine must slow down without ignition");
    }

    #[test]
    fn rev_limiter_holds_the_redline() {
        for id in ["gm_ls_v8", "lexus_lfa_1lr_gue_v10", "yamaha_yzf_r1_cp4"] {
            let (cfg, mut sim) = started(id);
            let wot = Controls { throttle: 1.0, ..idle_ctl() };
            let mut max_rpm: f64 = 0.0;
            for _ in 0..40 {
                run(&mut sim, &cfg, wot, 0.25);
                max_rpm = max_rpm.max(sim.rpm());
            }
            assert!(max_rpm > 0.9 * cfg.redline_rpm, "{id} should reach the redline, got {max_rpm:.0}");
            assert!(max_rpm < 1.03 * cfg.redline_rpm, "{id} overshoots the limiter: {max_rpm:.0}");
        }
    }

    #[test]
    fn fuel_cut_on_lift_off_then_back_to_idle() {
        let (cfg, mut sim) = started("gm_ls_v8");
        let wot = Controls { throttle: 1.0, ..idle_ctl() };
        run(&mut sim, &cfg, wot, 1.5);
        assert!(sim.rpm() > 2.0 * cfg.idle_rpm);
        run(&mut sim, &cfg, idle_ctl(), 0.05);
        assert!(!sim.is_firing(), "fuel must be cut while coasting well above idle");
        run(&mut sim, &cfg, idle_ctl(), 6.0);
        assert!(sim.is_firing());
        assert!((sim.rpm() - cfg.idle_rpm).abs() < 0.25 * cfg.idle_rpm, "{:.0}", sim.rpm());
    }

    #[test]
    fn idle_speed_is_adjustable() {
        let (cfg, mut sim) = started("gm_ls_v8");
        sim.set_idle_rpm(1400.0);
        run(&mut sim, &cfg, idle_ctl(), 4.0);
        assert!((sim.rpm() - 1400.0).abs() < 0.15 * 1400.0, "{:.0}", sim.rpm());
    }

    #[test]
    fn throttle_reports_the_larger_of_driver_and_governor() {
        let (cfg, mut sim) = started("gm_ls_v8");
        run(&mut sim, &cfg, idle_ctl(), 0.1);
        assert!(sim.throttle() > 0.0, "the governor keeps the throttle slightly open at idle");
        run(&mut sim, &cfg, Controls { throttle: 0.7, ..idle_ctl() }, 0.2);
        assert!((sim.throttle() - 0.7).abs() < 1e-9);
    }

    #[test]
    fn reported_torque_matches_the_acceleration_of_a_free_rev() {
        // Free rev-up in neutral: torque × dt = inertia × dω, so the reading must agree with
        // the measured angular acceleration.
        // A heavy flywheel keeps the engine away from the limiter during the measurement.
        let mut heavy = preset("gm_ls_v8");
        heavy.inertia *= 8.0;
        let (cfg, mut sim) = started_with(heavy);
        let wot = Controls { throttle: 1.0, ..idle_ctl() };
        // Let the throttle transient and the smoothing settle before measuring.
        run(&mut sim, &cfg, wot, 1.5);
        let (w0, t0) = (sim.omega, 0.0);
        let mut torque_sum = 0.0;
        let mut samples = 0;
        let steps = (0.5 * cfg.sim_rate) as usize;
        let mut out = vec![0.0; cfg.exhausts.len()];
        for _ in 0..steps {
            sim.step(&wot, &mut out);
            torque_sum += sim.torque_nm();
            samples += 1;
        }
        let accel = (sim.omega - w0) / (0.5 + t0);
        let measured = cfg.inertia * accel;
        let reported = torque_sum / samples as f64;
        assert!(measured > 50.0, "the engine should be accelerating: {measured:.0} N·m");
        assert!((reported - measured).abs() < 0.2 * measured, "reported {reported:.0} vs measured {measured:.0} N·m");
    }

    #[test]
    fn power_is_torque_times_angular_speed() {
        let (cfg, mut sim) = started("gm_ls_v8");
        run(&mut sim, &cfg, Controls { throttle: 0.8, ..idle_ctl() }, 1.0);
        let expected = sim.torque_nm() * sim.omega / 1000.0;
        assert!((sim.power_kw() - expected).abs() < 1e-9);
        assert!(sim.power_kw() > 5.0, "{:.1} kW", sim.power_kw());
    }

    #[test]
    fn torque_is_negative_on_overrun_and_small_at_idle() {
        let (cfg, mut sim) = started("gm_ls_v8");
        run(&mut sim, &cfg, Controls { throttle: 1.0, ..idle_ctl() }, 1.5);
        run(&mut sim, &cfg, idle_ctl(), 0.4);
        assert!(sim.torque_nm() < 0.0, "fuel cut: engine braking, got {:.0} N·m", sim.torque_nm());
        let (cfg, mut sim) = started("gm_ls_v8");
        run(&mut sim, &cfg, idle_ctl(), 1.0);
        assert!(sim.torque_nm().abs() < 60.0, "at idle the torque only covers friction: {:.0}", sim.torque_nm());
    }

    // --- Cylinder diagnostics -------------------------------------------------------------

    #[test]
    fn pv_loop_is_physical() {
        let (cfg, mut sim) = started("gm_ls_v8");
        run(&mut sim, &cfg, Controls { throttle: 0.6, ..idle_ctl() }, 1.0);
        let pv = sim.pv_loop();
        assert_eq!(pv.len(), PV_BINS);
        let filled: Vec<_> = pv.iter().filter(|p| p[0] > 0.0).collect();
        assert!(filled.len() > PV_BINS * 9 / 10, "the cycle should be almost fully sampled");
        let swept = std::f64::consts::PI / 4.0 * cfg.bore * cfg.bore * cfg.stroke;
        let (vmin, vmax) = filled.iter().fold((f32::MAX, 0.0_f32), |(lo, hi), p| (lo.min(p[0]), hi.max(p[0])));
        let span = (vmax - vmin) as f64;
        assert!(span > 0.9 * swept && span < 1.1 * swept, "swept volume {span} vs {swept}");
        let peak = filled.iter().fold(0.0_f32, |m, p| m.max(p[1]));
        assert!(peak > 10.0e5, "firing pressure peak should exceed 10 bar, got {:.1} bar", peak / 1e5);
        assert!(filled.iter().all(|p| p[1] > 0.0 && p[1].is_finite()));
    }

    // --- Turbo and anti-lag ----------------------------------------------------------------

    #[test]
    fn turbo_spools_with_load_and_speed() {
        let cfg = preset("wrc_rally1_1_6l_turbo");
        let mut sim = EngineSim::new(&cfg);
        run(&mut sim, &cfg, Controls { starter: true, ..idle_ctl() }, 1.0);
        run(&mut sim, &cfg, idle_ctl(), 2.0);
        let at_idle = sim.turbo_spool();
        assert!(at_idle < 0.2, "{at_idle}");
        run(&mut sim, &cfg, Controls { throttle: 1.0, ..idle_ctl() }, 3.0);
        assert!(sim.turbo_spool() > 0.6, "spool {:.2}", sim.turbo_spool());
    }

    #[test]
    fn naturally_aspirated_engine_has_no_spool() {
        let (cfg, mut sim) = started("gm_ls_v8");
        run(&mut sim, &cfg, Controls { throttle: 1.0, ..idle_ctl() }, 1.0);
        assert_eq!(sim.turbo_spool(), 0.0);
    }

    #[test]
    fn anti_lag_keeps_firing_and_boost_on_lift_off() {
        let id = "wrc_rally1_1_6l_turbo";
        let (cfg, mut lag) = started(id);
        assert!(cfg.anti_lag);
        let mut plain_cfg = cfg.clone();
        plain_cfg.anti_lag = false;
        let mut plain = EngineSim::new(&plain_cfg);
        run(&mut plain, &plain_cfg, Controls { starter: true, ..idle_ctl() }, 1.0);
        run(&mut plain, &plain_cfg, idle_ctl(), 2.0);

        let wot = Controls { throttle: 1.0, ..idle_ctl() };
        for (sim, c) in [(&mut lag, &cfg), (&mut plain, &plain_cfg)] {
            run(sim, c, wot, 3.0);
        }
        for (sim, c) in [(&mut lag, &cfg), (&mut plain, &plain_cfg)] {
            run(sim, c, idle_ctl(), 0.3);
        }
        assert!(lag.is_firing(), "anti-lag keeps burning fuel after lift-off");
        assert!(!plain.is_firing(), "without anti-lag the fuel is cut");
        assert!(lag.turbo_spool() > plain.turbo_spool(), "{:.2} vs {:.2}", lag.turbo_spool(), plain.turbo_spool());
    }

    // --- Drivetrain --------------------------------------------------------------------------

    fn in_gear(id: &str, gear: usize) -> Controls {
        let gb = &preset(id).gearbox;
        Controls { gear_ratio: gb.ratios[gear - 1] * gb.final_drive, clutch: 1.0, ..idle_ctl() }
    }

    /// Pulls away like a driver: slipping clutch in first for a moment, then fully engaged.
    fn launched(id: &str) -> (EngineCfg, EngineSim) {
        let (cfg, mut sim) = started(id);
        run(&mut sim, &cfg, Controls { throttle: 0.6, clutch: 0.35, ..in_gear(id, 1) }, 1.5);
        run(&mut sim, &cfg, Controls { throttle: 0.6, ..in_gear(id, 1) }, 1.0);
        (cfg, sim)
    }

    #[test]
    fn neutral_does_not_move_the_car() {
        let (cfg, mut sim) = started("gm_ls_v8");
        run(&mut sim, &cfg, Controls { throttle: 1.0, ..idle_ctl() }, 1.0);
        assert!(sim.speed_kmh().abs() < 1e-9);
    }

    #[test]
    fn accelerates_in_gear_and_speed_matches_engine_speed() {
        let id = "gm_ls_v8";
        let (cfg, mut sim) = launched(id);
        let ctl = Controls { throttle: 0.5, ..in_gear(id, 1) };
        let mut last = sim.speed_kmh();
        for _ in 0..8 {
            run(&mut sim, &cfg, ctl, 0.5);
            assert!(sim.speed_kmh() >= last - 1e-6, "speed must not drop under power");
            last = sim.speed_kmh();
        }
        assert!(last > 20.0, "{last:.1} km/h");
        // With the clutch locked, wheel speed and engine speed are tied by the ratio.
        let gb = &cfg.gearbox;
        let rpm_from_speed = sim.speed_kmh() / 3.6 / gb.wheel_radius * ctl.gear_ratio * 60.0 / (2.0 * PI);
        assert!((sim.rpm() - rpm_from_speed).abs() < 0.2 * rpm_from_speed, "{:.0} vs {:.0}", sim.rpm(), rpm_from_speed);
    }

    #[test]
    fn brake_slows_the_car_faster_than_coasting() {
        let id = "gm_ls_v8";
        let launch = |brake: f64| {
            let (cfg, mut sim) = launched(id);
            run(&mut sim, &cfg, Controls { throttle: 0.8, ..in_gear(id, 1) }, 2.0);
            let v0 = sim.speed_kmh();
            run(&mut sim, &cfg, Controls { brake, ..in_gear(id, 1) }, 1.0);
            (v0, sim.speed_kmh())
        };
        let (v0, coast) = launch(0.0);
        let (_, braked) = launch(1.0);
        assert!(v0 > 10.0);
        assert!(braked < coast, "braked {braked:.1} vs coasting {coast:.1}");
    }

    #[test]
    fn traction_limits_acceleration() {
        // An R1-like bike on 1st gear cannot out-accelerate its grip limit.
        let id = "yamaha_yzf_r1_cp4";
        let (cfg, mut sim) = started(id);
        let ctl = Controls { throttle: 1.0, ..in_gear(id, 1) };
        run(&mut sim, &cfg, ctl, 1.0);
        let accel_g = sim.speed_kmh() / 3.6 / 1.0 / 9.81;
        assert!(accel_g < cfg.gearbox.traction_g * 1.25, "{accel_g:.2} g");
        assert!(accel_g > 0.3, "it should still accelerate hard: {accel_g:.2} g");
    }

    #[test]
    fn disengaged_clutch_lets_the_engine_rev_without_driving() {
        let id = "gm_ls_v8";
        let (cfg, mut sim) = started(id);
        let ctl = Controls { throttle: 1.0, clutch: 0.0, ..in_gear(id, 1) };
        run(&mut sim, &cfg, ctl, 0.8);
        assert!(sim.rpm() > 2.0 * cfg.idle_rpm);
        assert!(sim.speed_kmh() < 1.0, "{:.1} km/h", sim.speed_kmh());
    }

    // --- Every preset ---------------------------------------------------------------------------

    #[test]
    fn presets_are_well_formed() {
        let all = presets::all();
        let mut ids: Vec<String> = all.iter().map(|e| e.id()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all.len(), "engine ids must be unique");
        for e in &all {
            let n = e.cylinders.len();
            assert!(n >= 1 && !e.exhausts.is_empty(), "{}", e.name);
            assert!(e.idle_rpm > 0.0 && e.redline_rpm > 1.5 * e.idle_rpm, "{}: idle/redline", e.name);
            assert!(e.compression_ratio > 1.0 && e.rod_length > e.stroke / 2.0, "{}: geometry", e.name);
            assert!(e.cylinders.iter().all(|c| c.exhaust < e.exhausts.len()), "{}: exhaust index", e.name);
            assert!(e.cylinders.iter().all(|c| (0.0..720.0).contains(&c.fire_deg)), "{}: fire angle", e.name);
            let mut fires: Vec<i64> = e.cylinders.iter().map(|c| (c.fire_deg * 1000.0).round() as i64).collect();
            fires.sort();
            fires.dedup();
            assert_eq!(fires.len(), n, "{}: every cylinder fires at its own angle", e.name);
            let ratios = &e.gearbox.ratios;
            assert!(ratios.windows(2).all(|w| w[0] > w[1]), "{}: gear ratios must decrease", e.name);
            assert!(e.gearbox.final_drive > 0.0 && e.gearbox.vehicle_mass > 0.0 && e.gearbox.wheel_radius > 0.0);
            assert!(e.sim_rate >= 6000.0, "{}", e.name);
            assert!(e.power_kw > 0.0 && e.torque_nm > 0.0, "{}: nominal power/torque missing", e.name);
            // Peak power cannot exceed peak torque at the redline speed (P = T·ω), within rounding.
            let omega_redline = e.redline_rpm * 2.0 * PI / 60.0;
            assert!(e.power_kw * 1e3 <= 1.06 * e.torque_nm * omega_redline, "{}: {} kW needs more than {} N·m", e.name, e.power_kw, e.torque_nm);
            let per_litre = e.power_kw / (e.displacement() * 1e3);
            assert!((5.0..500.0).contains(&per_litre), "{}: {per_litre:.0} kW/L looks wrong", e.name);
        }
    }

    #[test]
    fn every_preset_starts_and_revs() {
        for e in presets::all() {
            let id = e.id();
            let (cfg, mut sim) = started(&id);
            assert!(sim.rpm() > 0.5 * cfg.idle_rpm, "{id} did not start: {:.0} rpm", sim.rpm());
            run(&mut sim, &cfg, Controls { throttle: 1.0, ..idle_ctl() }, 1.5);
            assert!(sim.rpm() > 1.3 * cfg.idle_rpm, "{id} does not rev: {:.0} rpm", sim.rpm());
            assert!(sim.rpm() < 1.05 * cfg.redline_rpm, "{id} above the limiter: {:.0} rpm", sim.rpm());
        }
    }
}
