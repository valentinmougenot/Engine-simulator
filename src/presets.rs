//! Ready-made engines, inspired by the engine-sim scripts (`assets/engines/*`).
//! Audio parameters (jitter, noise, HF gain, exhaust volumes) are taken as-is; the
//! geometry is simplified.

use crate::dsp::{EqBand, EqKind};
use crate::engine::*;

const INCH: f64 = 0.0254;

/// Firing order (cylinder numbers from 1) -> firing offsets.
pub fn firing(
    order: &[usize],
    spacing_deg: f64,
    exhaust: impl Fn(usize) -> usize,
    primary: impl Fn(usize) -> f64,
) -> Vec<CylinderCfg> {
    let mut cyl: Vec<CylinderCfg> = (1..=order.len())
        .map(|n| CylinderCfg {
            fire_deg: 0.0,
            exhaust: exhaust(n),
            primary_length: primary(n),
            attenuation: 1.0,
            bank: 0,
        })
        .collect();
    for (pos, &n) in order.iter().enumerate() {
        cyl[n - 1].fire_deg = pos as f64 * spacing_deg;
    }
    cyl
}

fn exhaust(length: f64, audio_volume: f64, ir: Ir) -> ExhaustCfg {
    ExhaustCfg { length, audio_volume, ir, area: std::f64::consts::PI / 4.0 * 0.057 * 0.057 }
}

pub fn all() -> Vec<EngineCfg> {
    let mut engines = vec![
        gm_ls(),
        ferrari_f136(),
        ferrari_v12(),
        lamborghini_v12(),
        bugatti_w16(),
        lexus_lfa(),
        f1_v8(),
        yamaha_r1(),
        f1_v6_hybrid(),
        toyota_2jz(),
        audi_i5(),
        subaru_ej25(),
        inline4(),
        citroen_c3(),
        rally1(),
        audi_quattro_s1(),
        koenigsegg_jesko(),
        pagani_zonda_r(),
        harley_v_twin(),
        porsche_flat6(),
        subaru_brz(),
        mazda_787b(),
        lamborghini_v10(),
        cosworth_dfv(),
        f1_v10(),
        ferrari_f40(),
        nissan_gtr(),
        bmw_m3_e46(),
        bmw_m5_v10(),
        amg_v8(),
        dodge_hellcat(),
        toyota_supra(),
        honda_s2000(),
        honda_civic_type_r(),
        mazda_mx5(),
        cummins_diesel(),
        kart_2t(),
        rolls_royce_merlin(),
    ];
    for e in &mut engines {
        apply_layout(e);
    }
    engines
}

/// Bank layout (V angle, cylinder split) for the engine views.
fn apply_layout(e: &mut EngineCfg) {
    let id = e.id();
    let (angle, alternate) = match id.as_str() {
        "gm_ls_v8" => (90.0, true),
        "ferrari_f136_v8" | "koenigsegg_jesko_v8_twin_turbo" | "f1_2_4l_v8" | "cosworth_dfv_v8" | "ferrari_f40_v8_twin_turbo" | "mercedes_amg_m178_v8_twin_turbo"
        | "dodge_hellcat_v8_supercharged" | "lamborghini_huracan_v10" | "bmw_m5_e60_s85_v10" | "nissan_gt_r_vr38_v6_twin_turbo" => (90.0, false),
        "lexus_lfa_1lr_gue_v10" => (72.0, true),
        "f1_3_0l_v10" => (72.0, false),
        "f1_1_6l_v6_turbo_hybrid" => (120.0, false),
        "ferrari_812_f140_v12" => (65.0, false),
        "lamborghini_aventador_v12" | "pagani_zonda_r_v12" | "rolls_royce_merlin_v12" => (60.0, false),
        "bugatti_chiron_w16" => (90.0, false),
        "harley_davidson_v_twin_107" => (45.0, true),
        "subaru_ej25" | "subaru_brz_fa24_boxer" | "porsche_911_gt3_flat_6" => (180.0, true),
        _ => (0.0, false),
    };
    e.bank_angle_deg = angle;
    if angle == 0.0 {
        return;
    }
    let n = e.cylinders.len();
    for (i, c) in e.cylinders.iter_mut().enumerate() {
        c.bank = if alternate { i % 2 } else { usize::from(i >= n.div_ceil(2)) };
    }
}

impl EngineCfg {
    pub fn id(&self) -> String {
        self.name
            .split('(')
            .next()
            .unwrap_or(self.name)
            .trim()
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect()
    }
}

pub fn base() -> EngineCfg {
    EngineCfg {
        name: "",
        bore: 0.09,
        stroke: 0.08,
        rod_length: 0.15,
        compression_ratio: 10.0,
        inertia: 0.3,
        redline_rpm: 6500.0,
        idle_rpm: 900.0,
        friction_scale: 1.0,
        idle_throttle_min: 0.06,
        clutch_capacity_scale: 1.0,
        combustion_efficiency: 0.62,
        max_fuel_flow: 0.0,
        burn_duration_deg: 45.0,
        intake_boost: 1.0,
        dynamic_pressure_gain: 1.0,
        pipe_volume_fraction: 1.0,
        intake_duration_deg: 230.0,
        exhaust_duration_deg: 235.0,
        intake_center_deg: 116.0,
        exhaust_center_deg: 116.0,
        intake_lift: 0.012,
        exhaust_lift: 0.012,
        intake_valve_frac: 0.5,
        exhaust_valve_frac: 0.4,
        runner_volume: 50.0e-6,
        primary_area: 1.6e-3,
        hf_gain: 0.01,
        noise: 1.0,
        jitter: 0.5,
        input_cutoff: 1900.0,
        air_noise_cutoff: 2000.0,
        eq: vec![],
        sim_rate: 10_000.0,
        cylinders: vec![],
        exhausts: vec![],
        gearbox: Gearbox::default(),
        turbo: None,
        electric: None,
        anti_lag: false,
        bank_angle_deg: 0.0,
        power_kw: 0.0,
        torque_nm: 0.0,
    }
}

pub fn gm_ls() -> EngineCfg {
    let spacing = 2.0 * INCH;
    EngineCfg {
        combustion_efficiency: 0.95,
        intake_boost: 0.88,
        power_kw: 250.0,
        torque_nm: 450.0,
        name: "GM LS V8",
        bore: 3.78 * INCH,
        stroke: 3.622 * INCH,
        rod_length: 0.160,
        compression_ratio: 10.5,
        inertia: 0.35,
        redline_rpm: 6500.0,
        intake_duration_deg: 234.0,
        exhaust_duration_deg: 235.0,
        intake_center_deg: 116.0,
        exhaust_center_deg: 116.0,
        intake_lift: 551.0e-3 * INCH,
        exhaust_lift: 551.0e-3 * INCH,
        intake_valve_frac: 0.53,
        exhaust_valve_frac: 0.41,
        jitter: 0.6,
        gearbox: Gearbox {
            ratios: vec![2.66, 1.78, 1.30, 1.0, 0.74, 0.50],
            final_drive: 3.73,
            vehicle_mass: 1550.0,
            ..Gearbox::default()
        },
        cylinders: firing(
            &[1, 8, 7, 2, 6, 5, 4, 3],
            90.0,
            |n| (n + 1) % 2,
            |n| match (n - 1) / 2 {
                0 => 3.0 * spacing + 0.02,
                1 => 2.0 * spacing + 0.01,
                2 => 1.0 * spacing + 0.03,
                _ => 0.05,
            },
        ),
        exhausts: vec![
            exhaust(100.0 * INCH, 4.0, Ir::Default0),
            exhaust(172.0 * INCH, 4.0, Ir::Default0),
        ],
        ..base()
    }
}

pub fn ferrari_f136() -> EngineCfg {
    EngineCfg {
        combustion_efficiency: 0.95,
        intake_boost: 1.3,
        power_kw: 419.0,
        torque_nm: 540.0,
        name: "Ferrari F136 V8",
        bore: 0.094,
        stroke: 0.081,
        rod_length: 0.160,
        compression_ratio: 12.0,
        inertia: 0.22,
        redline_rpm: 9000.0,
        idle_rpm: 1100.0,
        intake_duration_deg: 250.0,
        exhaust_duration_deg: 250.0,
        intake_center_deg: 110.0,
        exhaust_center_deg: 110.0,
        intake_lift: 0.012,
        exhaust_lift: 0.011,
        intake_valve_frac: 0.5,
        exhaust_valve_frac: 0.41,
        jitter: 0.15,
        gearbox: Gearbox {
            ratios: vec![3.29, 2.16, 1.61, 1.27, 1.03, 0.84],
            final_drive: 4.1,
            vehicle_mass: 1450.0,
            wheel_radius: 0.34,
            ..Gearbox::default()
        },
        cylinders: firing(
            &[1, 5, 3, 7, 4, 8, 2, 6],
            90.0,
            |n| usize::from(n > 4),
            |n| [0.02, 0.01, 0.03, 0.05][(n - 1) % 4],
        ),
        exhausts: vec![
            exhaust(100.0 * INCH, 16.0, Ir::Real0),
            exhaust(100.0 * INCH, 14.4, Ir::Real0),
        ],
        ..base()
    }
}

pub fn audi_i5() -> EngineCfg {
    EngineCfg {
        combustion_efficiency: 0.95,
        intake_boost: 1.19,
        power_kw: 120.0,
        torque_nm: 230.0,
        name: "Audi I5",
        bore: 0.0864,
        stroke: 0.0795,
        rod_length: 5.142 * INCH,
        compression_ratio: 9.3,
        inertia: 0.12,
        redline_rpm: 6000.0,
        idle_rpm: 850.0,
        intake_duration_deg: 210.0,
        exhaust_duration_deg: 215.0,
        intake_center_deg: 110.0,
        exhaust_center_deg: 110.0,
        intake_lift: 9.78e-3,
        exhaust_lift: 9.6e-3,
        jitter: 0.299,
        sim_rate: 17_000.0,
        gearbox: Gearbox {
            ratios: vec![3.5, 2.1, 1.45, 1.1, 0.9],
            final_drive: 3.9,
            vehicle_mass: 1300.0,
            wheel_radius: 0.31,
            ..Gearbox::default()
        },
        cylinders: firing(&[1, 2, 4, 5, 3], 144.0, |n| (n + 1) % 2, |n| 0.08 + 0.02 * n as f64),
        exhausts: vec![exhaust(0.9, 4.0, Ir::Real1), exhaust(0.9, 3.2, Ir::Real1)],
        ..base()
    }
}

pub fn subaru_ej25() -> EngineCfg {
    EngineCfg {
        combustion_efficiency: 0.95,
        intake_boost: 1.12,
        power_kw: 127.0,
        torque_nm: 230.0,
        name: "Subaru EJ25 (unequal headers)",
        bore: 0.0995,
        stroke: 0.079,
        rod_length: 5.142 * INCH,
        compression_ratio: 9.7,
        inertia: 0.2,
        redline_rpm: 6500.0,
        idle_rpm: 850.0,
        intake_duration_deg: 232.0,
        exhaust_duration_deg: 236.0,
        intake_center_deg: 117.0,
        exhaust_center_deg: 112.0,
        intake_lift: 9.78e-3,
        exhaust_lift: 9.6e-3,
        jitter: 0.5,
        sim_rate: 20_000.0,
        gearbox: Gearbox {
            ratios: vec![3.45, 2.06, 1.45, 1.09, 0.83, 0.68],
            final_drive: 4.44,
            vehicle_mass: 1400.0,
            wheel_radius: 0.32,
            ..Gearbox::default()
        },
        // Boxer: the unequal primary lengths make the famous "rumble".
        cylinders: firing(&[1, 3, 2, 4], 180.0, |_| 0, |n| [0.35, 0.95, 0.30, 0.85][n - 1]),
        exhausts: vec![exhaust(0.5, 1.5, Ir::Real2)],
        ..base()
    }
}

pub fn inline4() -> EngineCfg {
    EngineCfg {
        combustion_efficiency: 0.95,
        intake_boost: 1.05,
        power_kw: 110.0,
        torque_nm: 200.0,
        name: "Inline 4 2.0L",
        bore: 0.086,
        stroke: 0.086,
        rod_length: 0.14,
        compression_ratio: 10.5,
        inertia: 0.14,
        redline_rpm: 7200.0,
        idle_rpm: 850.0,
        intake_duration_deg: 240.0,
        exhaust_duration_deg: 240.0,
        intake_center_deg: 112.0,
        exhaust_center_deg: 112.0,
        jitter: 0.3,
        cylinders: firing(&[1, 3, 4, 2], 180.0, |_| 0, |_| 0.35),
        exhausts: vec![exhaust(1.6, 12.0, Ir::Real0)],
        ..base()
    }
}

pub fn lexus_lfa() -> EngineCfg {
    EngineCfg {
        combustion_efficiency: 0.95,
        intake_boost: 1.09,
        power_kw: 412.0,
        torque_nm: 480.0,
        name: "Lexus LFA 1LR-GUE V10",
        bore: 0.088,
        stroke: 0.079,
        rod_length: 0.130,
        compression_ratio: 12.0,
        inertia: 0.18,
        redline_rpm: 9000.0,
        idle_rpm: 1000.0,
        intake_duration_deg: 230.0,
        exhaust_duration_deg: 230.0,
        intake_center_deg: 90.0,
        exhaust_center_deg: 112.0,
        intake_lift: 15.95e-3,
        exhaust_lift: 15.95e-3,
        intake_valve_frac: 0.52,
        exhaust_valve_frac: 0.42,
        jitter: 0.1,
        dynamic_pressure_gain: 2.0,
        input_cutoff: 3500.0,
        sim_rate: 6_500.0,
        gearbox: Gearbox {
            ratios: vec![3.23, 2.19, 1.64, 1.28, 1.0, 0.82],
            final_drive: 3.45,
            vehicle_mass: 1480.0,
            wheel_radius: 0.34,
            drag_area: 0.68,
            shift_time: 0.5,
            traction_g: 1.2,
            downforce_area: 0.0,
            traction_control: false,
        },
        // 72° between firings, order 1-2-3-4-7-8-9-10-5-6.
        cylinders: firing(
            &[1, 2, 3, 4, 7, 8, 9, 10, 5, 6],
            72.0,
            |n| (n + 1) % 2,
            |n| 0.01 * (5 - (n - 1) / 2) as f64,
        ),
        exhausts: vec![
            exhaust(100.0 * INCH, 24.0, Ir::Real2),
            exhaust(100.5 * INCH, 24.0, Ir::Real2),
        ],
        ..base()
    }
}

/// Late naturally-aspirated F1 V8 (2.4 L, ~18,000 rpm).
pub fn f1_v8() -> EngineCfg {
    EngineCfg {
        power_kw: 560.0,
        torque_nm: 320.0,
        name: "F1 2.4L V8 (2006-2013)",
        bore: 0.098,
        stroke: 0.0398,
        rod_length: 0.100,
        compression_ratio: 13.0,
        inertia: 0.035,
        redline_rpm: 18_000.0,
        idle_rpm: 4_000.0,
        friction_scale: 0.4,
        combustion_efficiency: 0.9,
        burn_duration_deg: 32.0,
        intake_boost: 1.3,
        intake_duration_deg: 290.0,
        exhaust_duration_deg: 250.0,
        intake_center_deg: 105.0,
        exhaust_center_deg: 105.0,
        intake_lift: 14.0e-3,
        exhaust_lift: 13.0e-3,
        intake_valve_frac: 0.56,
        exhaust_valve_frac: 0.44,
        runner_volume: 40.0e-6,
        primary_area: 1.3e-3,
        jitter: 0.3,
        hf_gain: 0.0001,
        dynamic_pressure_gain: 64.0,
        input_cutoff: 9000.0,
        air_noise_cutoff: 5000.0,
        sim_rate: 20_000.0,
        gearbox: Gearbox {
            ratios: vec![3.3, 2.4, 1.9, 1.55, 1.3, 1.12, 1.0],
            final_drive: 5.7,
            vehicle_mass: 650.0,
            wheel_radius: 0.33,
            drag_area: 0.95,
            shift_time: 0.2,
            traction_g: 1.35,
            downforce_area: 3.0,
            traction_control: false,
        },
        // Flat-plane crank: even firing every 90°.
        cylinders: firing(
            &[1, 5, 3, 7, 4, 8, 2, 6],
            90.0,
            |n| usize::from(n > 4),
            |_| 0.30,
        ),
        exhausts: vec![
            exhaust(0.55, 0.2, Ir::Real1),
            // Weaker right bank: this imbalance (mic closer to one side) produces the
            // half-firing-frequency component of the reference recording.
            exhaust(0.55, 0.1, Ir::Real1),
        ],
        // Light EQ, tuned with the rest against the spectrum of an F1 V8 recording
        // at 8,400, 12,600 and 17,500 rpm.
        eq: vec![
            EqBand { kind: EqKind::LowShelf, freq: 300.0, gain_db: -4.0 },
            EqBand { kind: EqKind::Peak, freq: 4000.0, gain_db: -5.0 },
        ],
        ..base()
    }
}

pub fn toyota_2jz() -> EngineCfg {
    EngineCfg {
        combustion_efficiency: 0.95,
        intake_boost: 1.19,
        power_kw: 170.0,
        torque_nm: 290.0,
        name: "Toyota 2JZ I6",
        bore: 0.086,
        stroke: 0.086,
        rod_length: 0.142,
        compression_ratio: 10.0,
        inertia: 0.22,
        redline_rpm: 6000.0,
        idle_rpm: 800.0,
        intake_duration_deg: 240.0,
        exhaust_duration_deg: 240.0,
        intake_center_deg: 110.0,
        exhaust_center_deg: 110.0,
        jitter: 0.23,
        gearbox: Gearbox {
            ratios: vec![3.83, 2.36, 1.69, 1.31, 1.0, 0.79],
            final_drive: 3.27,
            vehicle_mass: 1550.0,
            ..Gearbox::default()
        },
        cylinders: firing(&[1, 5, 3, 6, 2, 4], 120.0, |n| usize::from(n > 3), |n| 0.30 + 0.05 * ((n - 1) % 3) as f64),
        exhausts: vec![
            exhaust(100.0 * INCH, 4.0, Ir::Real1),
            exhaust(100.0 * INCH, 4.0, Ir::Real1),
        ],
        ..base()
    }
}

/// Yamaha YZF-R1 (CP4 crossplane crank): inline-four with uneven firing
/// (270°, 180°, 90°, 180°), hence its V4-like growl.
pub fn yamaha_r1() -> EngineCfg {
    let order = [1usize, 3, 2, 4];
    let fire = [0.0, 270.0, 450.0, 540.0];
    let mut cylinders: Vec<CylinderCfg> = (1..=4)
        .map(|n| CylinderCfg { fire_deg: 0.0, exhaust: 0, primary_length: 0.45 + 0.03 * n as f64, attenuation: 1.0, bank: 0 })
        .collect();
    for (&n, &deg) in order.iter().zip(&fire) {
        cylinders[n - 1].fire_deg = deg;
    }
    EngineCfg {
        power_kw: 147.0,
        torque_nm: 113.0,
        name: "Yamaha YZF-R1 CP4",
        bore: 0.079,
        stroke: 0.0509,
        rod_length: 0.104,
        compression_ratio: 13.0,
        inertia: 0.045,
        redline_rpm: 14_000.0,
        idle_rpm: 1_500.0,
        friction_scale: 0.4,
        idle_throttle_min: 0.015,
        combustion_efficiency: 0.95,
        burn_duration_deg: 36.0,
        intake_boost: 1.3,
        intake_duration_deg: 260.0,
        exhaust_duration_deg: 255.0,
        intake_center_deg: 105.0,
        exhaust_center_deg: 108.0,
        intake_lift: 9.0e-3,
        exhaust_lift: 8.5e-3,
        intake_valve_frac: 0.58,
        exhaust_valve_frac: 0.47,
        runner_volume: 30.0e-6,
        primary_area: 8.0e-4,
        jitter: 0.2,
        dynamic_pressure_gain: 4.0,
        input_cutoff: 4000.0,
        sim_rate: 20_000.0,
        gearbox: Gearbox {
            // Primary drive (1.564) folded into the gear ratios.
            ratios: vec![4.07, 3.23, 2.68, 2.31, 2.04, 1.82],
            final_drive: 2.53,
            vehicle_mass: 280.0,
            wheel_radius: 0.31,
            drag_area: 0.30,
            shift_time: 0.35,
            traction_g: 1.1,
            downforce_area: 0.0,
            traction_control: true,
        },
        cylinders,
        exhausts: vec![ExhaustCfg {
            length: 0.75,
            audio_volume: 1.0,
            ir: Ir::Real0,
            area: std::f64::consts::PI / 4.0 * 0.045 * 0.045,
        }],
        ..base()
    }
}

/// F1 hybrid turbo V6 (1.6 L, 15,000 rpm, since 2014): much more muted than the V8,
/// with a whistling turbine and an MGU-K adding torque at full load.
pub fn f1_v6_hybrid() -> EngineCfg {
    EngineCfg {
        power_kw: 745.0,
        torque_nm: 600.0,
        name: "F1 1.6L V6 turbo hybrid (2021-2025)",
        bore: 0.080,
        stroke: 0.0531,
        rod_length: 0.105,
        compression_ratio: 16.0,
        inertia: 0.05,
        redline_rpm: 15_000.0,
        idle_rpm: 4_500.0,
        friction_scale: 0.35,
        idle_throttle_min: 0.03,
        clutch_capacity_scale: 3.0,
        combustion_efficiency: 0.62,
        // 100 kg/h: fuel flow limit set by the regulations.
        max_fuel_flow: 100.0 / 3600.0,
        burn_duration_deg: 34.0,
        intake_boost: 1.0,
        intake_duration_deg: 270.0,
        exhaust_duration_deg: 255.0,
        intake_center_deg: 105.0,
        exhaust_center_deg: 108.0,
        intake_lift: 13.0e-3,
        exhaust_lift: 12.0e-3,
        intake_valve_frac: 0.55,
        exhaust_valve_frac: 0.44,
        runner_volume: 60.0e-6,
        primary_area: 1.2e-3,
        jitter: 0.15,
        hf_gain: 0.003,
        dynamic_pressure_gain: 8.0,
        input_cutoff: 3500.0,
        air_noise_cutoff: 3000.0,
        sim_rate: 20_000.0,
        gearbox: Gearbox {
            ratios: vec![3.1, 2.4, 1.95, 1.62, 1.38, 1.2, 1.06, 0.95],
            final_drive: 5.6,
            vehicle_mass: 798.0,
            wheel_radius: 0.33,
            drag_area: 1.05,
            shift_time: 0.2,
            traction_g: 1.35,
            downforce_area: 3.2,
            traction_control: false,
        },
        // Two banks of three cylinders, even firing every 120°.
        cylinders: firing(
            &[1, 4, 2, 5, 3, 6],
            120.0,
            |_| 0,
            |n| 0.35 + 0.02 * ((n - 1) % 3) as f64,
        ),
        // All the gas goes through the turbine, then a single central exhaust.
        exhausts: vec![ExhaustCfg {
            length: 1.1,
            audio_volume: 1.0,
            ir: Ir::Real2,
            area: std::f64::consts::PI / 4.0 * 0.06 * 0.06,
        }],
        turbo: Some(Turbo { max_boost: 4.5, spool_start_rpm: 5_000.0, spool_full_rpm: 11_000.0, lag: 0.12, whine: 1.0 }),
        electric: Some(Electric { power_kw: 120.0, max_torque: 200.0 }),
        ..base()
    }
}

/// Ferrari 812 Superfast (F140): 6.5 L 65° V12, 8,900 rpm.
pub fn ferrari_v12() -> EngineCfg {
    EngineCfg {
        power_kw: 588.0,
        torque_nm: 718.0,
        name: "Ferrari 812 F140 V12",
        bore: 0.094,
        stroke: 0.078,
        rod_length: 0.150,
        compression_ratio: 13.6,
        inertia: 0.25,
        redline_rpm: 8_900.0,
        idle_rpm: 1_000.0,
        friction_scale: 0.8,
        clutch_capacity_scale: 1.6,
        intake_duration_deg: 255.0,
        exhaust_duration_deg: 250.0,
        intake_center_deg: 108.0,
        exhaust_center_deg: 110.0,
        intake_lift: 13.0e-3,
        exhaust_lift: 12.0e-3,
        intake_valve_frac: 0.52,
        exhaust_valve_frac: 0.42,
        intake_boost: 1.5,
        jitter: 0.1,
        dynamic_pressure_gain: 3.0,
        input_cutoff: 4000.0,
        sim_rate: 6_500.0,
        gearbox: Gearbox {
            ratios: vec![3.08, 2.19, 1.63, 1.29, 1.03, 0.84, 0.69],
            final_drive: 3.9,
            vehicle_mass: 1525.0,
            wheel_radius: 0.345,
            drag_area: 0.65,
            shift_time: 0.25,
            traction_g: 1.2,
            downforce_area: 0.3,
            traction_control: true,
        },
        // Firing every 60°, alternating banks: 1-7-5-11-3-9-6-12-2-8-4-10.
        cylinders: firing(
            &[1, 7, 5, 11, 3, 9, 6, 12, 2, 8, 4, 10],
            60.0,
            |n| usize::from(n > 6),
            |n| 0.012 * (1 + (n - 1) % 6) as f64,
        ),
        exhausts: vec![
            exhaust(90.0 * INCH, 22.0, Ir::Real2),
            exhaust(90.5 * INCH, 22.0, Ir::Real2),
        ],
        ..base()
    }
}

/// Lamborghini Aventador (L539): 6.5 L 60° V12, 8,500 rpm, raspier.
pub fn lamborghini_v12() -> EngineCfg {
    EngineCfg {
        power_kw: 544.0,
        torque_nm: 690.0,
        name: "Lamborghini Aventador V12",
        bore: 0.095,
        stroke: 0.0764,
        rod_length: 0.145,
        compression_ratio: 11.8,
        inertia: 0.27,
        redline_rpm: 8_500.0,
        idle_rpm: 950.0,
        friction_scale: 0.8,
        clutch_capacity_scale: 1.6,
        intake_duration_deg: 245.0,
        exhaust_duration_deg: 245.0,
        intake_center_deg: 112.0,
        exhaust_center_deg: 112.0,
        intake_lift: 12.5e-3,
        exhaust_lift: 11.5e-3,
        intake_valve_frac: 0.52,
        exhaust_valve_frac: 0.42,
        intake_boost: 1.5,
        jitter: 0.2,
        noise: 1.2,
        dynamic_pressure_gain: 2.5,
        input_cutoff: 3200.0,
        sim_rate: 6_500.0,
        gearbox: Gearbox {
            ratios: vec![3.91, 2.44, 1.81, 1.46, 1.19, 0.97, 0.89],
            final_drive: 3.2,
            vehicle_mass: 1575.0,
            wheel_radius: 0.35,
            drag_area: 0.70,
            shift_time: 0.2,
            traction_g: 1.25,
            downforce_area: 0.3,
            traction_control: true,
        },
        // Order 1-7-4-10-2-8-6-12-3-9-5-11, shorter collectors.
        cylinders: firing(
            &[1, 7, 4, 10, 2, 8, 6, 12, 3, 9, 5, 11],
            60.0,
            |n| usize::from(n > 6),
            |n| 0.010 * (1 + (n - 1) % 6) as f64,
        ),
        exhausts: vec![
            exhaust(70.0 * INCH, 24.0, Ir::Real1),
            exhaust(71.0 * INCH, 24.0, Ir::Real1),
        ],
        ..base()
    }
}

/// Bugatti Chiron: 8.0 L quad-turbo W16, 6,700 rpm.
pub fn bugatti_w16() -> EngineCfg {
    EngineCfg {
        power_kw: 1103.0,
        torque_nm: 1600.0,
        name: "Bugatti Chiron W16",
        bore: 0.086,
        stroke: 0.086,
        rod_length: 0.150,
        compression_ratio: 9.0,
        inertia: 0.45,
        redline_rpm: 6_700.0,
        idle_rpm: 750.0,
        friction_scale: 0.8,
        clutch_capacity_scale: 3.0,
        combustion_efficiency: 0.62,
        intake_duration_deg: 240.0,
        exhaust_duration_deg: 240.0,
        intake_center_deg: 112.0,
        exhaust_center_deg: 112.0,
        intake_lift: 11.0e-3,
        exhaust_lift: 10.0e-3,
        intake_valve_frac: 0.5,
        exhaust_valve_frac: 0.42,
        jitter: 0.15,
        dynamic_pressure_gain: 2.0,
        input_cutoff: 2600.0,
        sim_rate: 6_500.0,
        gearbox: Gearbox {
            ratios: vec![3.35, 2.35, 1.75, 1.37, 1.08, 0.85, 0.68],
            final_drive: 3.2,
            vehicle_mass: 1996.0,
            wheel_radius: 0.36,
            drag_area: 0.72,
            shift_time: 0.2,
            traction_g: 1.3,
            downforce_area: 0.5,
            traction_control: true,
        },
        // Order 1-14-9-4-7-12-15-6-13-8-3-16-11-2-5-10, firing every 45°.
        cylinders: firing(
            &[1, 14, 9, 4, 7, 12, 15, 6, 13, 8, 3, 16, 11, 2, 5, 10],
            45.0,
            |n| usize::from(n > 8),
            |n| 0.015 + 0.01 * ((n - 1) % 4) as f64,
        ),
        // The turbos muffle and soften the exhaust.
        exhausts: vec![
            exhaust(110.0 * INCH, 20.0, Ir::Real2),
            exhaust(110.5 * INCH, 20.0, Ir::Real2),
        ],
        turbo: Some(Turbo { max_boost: 3.6, spool_start_rpm: 1_800.0, spool_full_rpm: 3_800.0, lag: 0.25, whine: 0.5 }),
        ..base()
    }
}

// Additional engines. `intake_boost` is tuned to hit each engine's real
// power (see `--probe`).

/// Common starting point: geometry + bore-dependent settings.
fn eng(name: &'static str, bore: f64, stroke: f64, cr: f64, redline: f64, idle: f64, inertia: f64) -> EngineCfg {
    let k = bore / 0.09;
    EngineCfg {
        name,
        bore,
        stroke,
        rod_length: 1.75 * stroke,
        compression_ratio: cr,
        inertia,
        redline_rpm: redline,
        idle_rpm: idle,
        friction_scale: 0.8,
        combustion_efficiency: 0.95,
        intake_lift: 0.15 * bore,
        exhaust_lift: 0.14 * bore,
        intake_duration_deg: 240.0,
        exhaust_duration_deg: 240.0,
        intake_center_deg: 112.0,
        exhaust_center_deg: 112.0,
        runner_volume: 50.0e-6 * k * k * k,
        primary_area: 1.6e-3 * k * k,
        jitter: 0.15,
        dynamic_pressure_gain: 2.0,
        input_cutoff: 3500.0,
        sim_rate: if redline > 12_000.0 { 20_000.0 } else { 10_000.0 },
        ..base()
    }
}

fn gearbox(ratios: &[f64], final_drive: f64, mass: f64, wheel_radius: f64, drag_area: f64, shift_time: f64) -> Gearbox {
    Gearbox {
        ratios: ratios.to_vec(),
        final_drive,
        vehicle_mass: mass,
        wheel_radius,
        drag_area,
        shift_time,
        traction_g: 1.15,
        downforce_area: 0.0,
        traction_control: true,
    }
}

fn pair(length_in: f64, volume: f64, ir: Ir) -> Vec<ExhaustCfg> {
    vec![exhaust(length_in * INCH, volume, ir), exhaust((length_in + 0.5) * INCH, volume, ir)]
}

fn turbo(max_boost: f64, start: f64, full: f64, lag: f64, whine: f64) -> Option<Turbo> {
    Some(Turbo { max_boost, spool_start_rpm: start, spool_full_rpm: full, lag, whine })
}

/// Harley-Davidson Milwaukee-Eight 107: 45° V-twin, firing at 315° / 405°.
pub fn harley_v_twin() -> EngineCfg {
    EngineCfg {
        power_kw: 62.0,
        torque_nm: 163.0,
        intake_boost: 0.86,
        friction_scale: 1.0,
        idle_throttle_min: 0.04,
        burn_duration_deg: 55.0,
        jitter: 0.5,
        hf_gain: 0.02,
        input_cutoff: 2200.0,
        gearbox: Gearbox { traction_g: 0.9, ..gearbox(&[2.69, 1.86, 1.37, 1.10, 0.92, 0.79], 2.4, 400.0, 0.34, 0.6, 0.4) },
        cylinders: {
            let mut c = firing(&[1, 2], 315.0, |_| 0, |n| 0.45 + 0.1 * (n - 1) as f64);
            c[0].exhaust = 0;
            c[1].exhaust = 1;
            c
        },
        exhausts: vec![exhaust(0.9, 12.0, Ir::Real1), exhaust(1.0, 12.0, Ir::Real1)],
        ..eng("Harley-Davidson V-twin 107", 0.1, 0.111, 10.0, 5_800.0, 900.0, 0.55)
    }
}

/// Porsche 911 GT3 (9A1): 4.0 L flat-six, 9,000 rpm.
pub fn porsche_flat6() -> EngineCfg {
    EngineCfg {
        power_kw: 375.0,
        torque_nm: 470.0,
        intake_boost: 1.21,
        compression_ratio: 13.3,
        intake_duration_deg: 255.0,
        exhaust_duration_deg: 250.0,
        input_cutoff: 4500.0,
        gearbox: Gearbox { downforce_area: 0.6, ..gearbox(&[3.75, 2.38, 1.72, 1.34, 1.08, 0.88, 0.68], 3.97, 1435.0, 0.35, 0.65, 0.2) },
        cylinders: firing(&[1, 6, 2, 4, 3, 5], 120.0, |n| usize::from(n % 2 == 0), |n| 0.30 + 0.02 * ((n - 1) / 2) as f64),
        exhausts: pair(55.0, 20.0, Ir::Real2),
        ..eng("Porsche 911 GT3 flat-6", 0.102, 0.0814, 13.3, 9_000.0, 1_000.0, 0.17)
    }
}

/// Subaru BRZ (FA24): 2.4 L boxer, unequal primary tubes.
pub fn subaru_brz() -> EngineCfg {
    EngineCfg {
        power_kw: 175.0,
        torque_nm: 250.0,
        intake_boost: 1.15,
        jitter: 0.4,
        gearbox: gearbox(&[3.63, 2.19, 1.62, 1.27, 1.0, 0.77], 4.10, 1290.0, 0.32, 0.62, 0.4),
        cylinders: firing(&[1, 3, 2, 4], 180.0, |_| 0, |n| [0.38, 0.95, 0.33, 0.88][n - 1]),
        exhausts: vec![exhaust(0.6, 1.6, Ir::Real2)],
        ..eng("Subaru BRZ FA24 boxer", 0.094, 0.086, 12.5, 7_400.0, 800.0, 0.2)
    }
}

/// Mazda 787B (R26B): 4 rotors. Each rotor fires once per shaft revolution,
/// modelled as two "cylinders" 360° apart; 8 firings per 720° cycle.
pub fn mazda_787b() -> EngineCfg {
    EngineCfg {
        power_kw: 530.0,
        torque_nm: 607.0,
        intake_boost: 1.41,
        burn_duration_deg: 70.0,
        intake_duration_deg: 300.0,
        exhaust_duration_deg: 300.0,
        intake_center_deg: 120.0,
        exhaust_center_deg: 120.0,
        hf_gain: 0.02,
        jitter: 0.25,
        dynamic_pressure_gain: 4.0,
        input_cutoff: 4500.0,
        gearbox: Gearbox { downforce_area: 2.5, ..gearbox(&[2.4, 1.75, 1.4, 1.17, 1.0], 3.6, 830.0, 0.33, 0.7, 0.25) },
        cylinders: firing(&[1, 2, 3, 4, 5, 6, 7, 8], 90.0, |n| usize::from(n % 2 == 0), |n| 0.2 + 0.03 * ((n - 1) / 2) as f64),
        exhausts: vec![exhaust(0.8, 30.0, Ir::Real1), exhaust(0.85, 30.0, Ir::Real1)],
        ..eng("Mazda 787B R26B 4 rotors", 0.1, 0.0833, 9.0, 9_000.0, 2_000.0, 0.18)
    }
}

/// Lamborghini Huracán / Audi R8: 5.2 L 90° V10 (order 1-6-5-10-2-7-3-8-4-9).
pub fn lamborghini_v10() -> EngineCfg {
    EngineCfg {
        power_kw: 470.0,
        torque_nm: 600.0,
        intake_boost: 1.29,
        intake_duration_deg: 250.0,
        exhaust_duration_deg: 245.0,
        input_cutoff: 3800.0,
        noise: 1.2,
        gearbox: Gearbox { downforce_area: 0.4, ..gearbox(&[3.82, 2.19, 1.57, 1.19, 0.94, 0.77, 0.65], 3.85, 1422.0, 0.34, 0.66, 0.2) },
        cylinders: firing(&[1, 6, 5, 10, 2, 7, 3, 8, 4, 9], 72.0, |n| usize::from(n > 5), |n| 0.012 * (1 + (n - 1) % 5) as f64),
        exhausts: pair(60.0, 24.0, Ir::Real1),
        ..eng("Lamborghini Huracan V10", 0.0845, 0.0925, 12.7, 8_500.0, 1_000.0, 0.2)
    }
}

/// Cosworth DFV: 3.0 L V8 from 1970s-80s F1.
pub fn cosworth_dfv() -> EngineCfg {
    EngineCfg {
        power_kw: 350.0,
        torque_nm: 400.0,
        intake_boost: 1.3,
        burn_duration_deg: 40.0,
        intake_duration_deg: 260.0,
        exhaust_duration_deg: 255.0,
        jitter: 0.3,
        hf_gain: 0.003,
        input_cutoff: 5000.0,
        idle_throttle_min: 0.03,
        gearbox: Gearbox { traction_g: 1.0, ..gearbox(&[2.35, 1.72, 1.36, 1.12, 0.95], 4.1, 640.0, 0.34, 0.9, 0.3) },
        cylinders: firing(&[1, 8, 3, 6, 4, 5, 2, 7], 90.0, |n| usize::from(n > 4), |n| 0.25 + 0.03 * ((n - 1) % 4) as f64),
        exhausts: vec![exhaust(0.55, 18.0, Ir::Real0), exhaust(0.6, 18.0, Ir::Real0)],
        ..eng("Cosworth DFV V8", 0.0854, 0.0645, 11.0, 10_500.0, 2_500.0, 0.06)
    }
}

/// F1 V10 (2000-2005): 3.0 L, 18,000 rpm.
pub fn f1_v10() -> EngineCfg {
    EngineCfg {
        power_kw: 710.0,
        torque_nm: 400.0,
        intake_boost: 1.64,
        friction_scale: 0.35,
        idle_throttle_min: 0.03,
        burn_duration_deg: 32.0,
        intake_duration_deg: 270.0,
        exhaust_duration_deg: 250.0,
        intake_center_deg: 105.0,
        exhaust_center_deg: 108.0,
        jitter: 0.2,
        hf_gain: 0.001,
        dynamic_pressure_gain: 16.0,
        input_cutoff: 7000.0,
        air_noise_cutoff: 4000.0,
        clutch_capacity_scale: 2.0,
        gearbox: Gearbox { downforce_area: 3.0, traction_g: 1.35, ..gearbox(&[2.7, 2.1, 1.75, 1.5, 1.3, 1.15, 1.03], 4.8, 600.0, 0.33, 0.9, 0.12) },
        cylinders: firing(&[1, 6, 5, 10, 2, 7, 3, 8, 4, 9], 72.0, |n| usize::from(n > 5), |n| 0.30 + 0.02 * ((n - 1) % 5) as f64),
        exhausts: vec![exhaust(0.9, 12.0, Ir::Real1), exhaust(0.95, 11.0, Ir::Real1)],
        eq: vec![EqBand { kind: EqKind::LowShelf, freq: 300.0, gain_db: -4.0 }],
        ..eng("F1 3.0L V10 (2000-2005)", 0.097, 0.0406, 12.5, 18_000.0, 4_500.0, 0.04)
    }
}

/// Ferrari F40: 2.9 L twin-turbo V8.
pub fn ferrari_f40() -> EngineCfg {
    EngineCfg {
        power_kw: 350.0,
        torque_nm: 577.0,
        intake_boost: 1.43,
        clutch_capacity_scale: 2.0,
        intake_duration_deg: 240.0,
        gearbox: gearbox(&[2.9, 1.94, 1.43, 1.09, 0.85], 3.9, 1250.0, 0.34, 0.7, 0.5),
        cylinders: firing(&[1, 5, 3, 7, 4, 8, 2, 6], 90.0, |n| usize::from(n > 4), |n| 0.10 + 0.02 * ((n - 1) % 4) as f64),
        exhausts: pair(60.0, 18.0, Ir::Real0),
        turbo: turbo(2.2, 2_500.0, 4_500.0, 0.4, 0.6),
        ..eng("Ferrari F40 V8 twin-turbo", 0.082, 0.069, 7.8, 7_800.0, 1_000.0, 0.2)
    }
}

/// Nissan GT-R (VR38DETT): 3.8 L twin-turbo V6.
pub fn nissan_gtr() -> EngineCfg {
    EngineCfg {
        power_kw: 420.0,
        torque_nm: 637.0,
        intake_boost: 1.31,
        clutch_capacity_scale: 2.0,
        gearbox: gearbox(&[4.06, 2.30, 1.59, 1.25, 1.0, 0.80], 3.7, 1740.0, 0.35, 0.7, 0.18),
        cylinders: firing(&[1, 4, 2, 5, 3, 6], 120.0, |n| usize::from(n > 3), |n| 0.15 + 0.02 * ((n - 1) % 3) as f64),
        exhausts: pair(70.0, 22.0, Ir::Real2),
        turbo: turbo(2.0, 2_000.0, 3_800.0, 0.3, 0.5),
        ..eng("Nissan GT-R VR38 V6 twin-turbo", 0.095, 0.0895, 9.0, 7_100.0, 750.0, 0.22)
    }
}

/// BMW M3 E46 (S54): 3.2 L inline-six.
pub fn bmw_m3_e46() -> EngineCfg {
    EngineCfg {
        power_kw: 252.0,
        torque_nm: 365.0,
        intake_boost: 1.17,
        intake_duration_deg: 250.0,
        exhaust_duration_deg: 245.0,
        input_cutoff: 4000.0,
        gearbox: gearbox(&[4.23, 2.53, 1.67, 1.23, 1.0, 0.83], 3.62, 1570.0, 0.33, 0.65, 0.4),
        cylinders: firing(&[1, 5, 3, 6, 2, 4], 120.0, |_| 0, |n| 0.30 + 0.03 * ((n - 1) % 3) as f64),
        exhausts: vec![exhaust(0.85, 2.0, Ir::Real1)],
        ..eng("BMW M3 E46 S54 I6", 0.087, 0.091, 11.5, 8_000.0, 850.0, 0.2)
    }
}

/// BMW M5 E60 (S85): 5.0 L V10, 8,250 rpm.
pub fn bmw_m5_v10() -> EngineCfg {
    EngineCfg {
        power_kw: 373.0,
        torque_nm: 520.0,
        intake_boost: 1.01,
        intake_duration_deg: 250.0,
        input_cutoff: 3800.0,
        gearbox: gearbox(&[4.06, 2.4, 1.58, 1.19, 1.0, 0.87, 0.69], 3.62, 1830.0, 0.35, 0.72, 0.2),
        cylinders: firing(&[1, 6, 5, 10, 2, 7, 3, 8, 4, 9], 72.0, |n| usize::from(n > 5), |n| 0.01 * (1 + (n - 1) % 5) as f64),
        exhausts: pair(75.0, 22.0, Ir::Real2),
        ..eng("BMW M5 E60 S85 V10", 0.092, 0.075, 12.0, 8_250.0, 900.0, 0.22)
    }
}

/// Mercedes-AMG M178: 4.0 L twin-turbo cross-plane V8.
pub fn amg_v8() -> EngineCfg {
    EngineCfg {
        power_kw: 450.0,
        torque_nm: 700.0,
        intake_boost: 1.41,
        clutch_capacity_scale: 2.0,
        dynamic_pressure_gain: 3.0,
        input_cutoff: 3000.0,
        gearbox: gearbox(&[3.59, 2.19, 1.41, 1.0, 0.83, 0.66, 0.57], 3.27, 1650.0, 0.35, 0.68, 0.2),
        cylinders: firing(&[1, 5, 4, 8, 6, 3, 7, 2], 90.0, |n| usize::from(n > 4), |n| 0.08 + 0.02 * ((n - 1) % 4) as f64),
        exhausts: pair(90.0, 24.0, Ir::Real2),
        turbo: turbo(1.9, 1_800.0, 3_500.0, 0.25, 0.3),
        ..eng("Mercedes-AMG M178 V8 twin-turbo", 0.083, 0.092, 8.6, 7_200.0, 750.0, 0.25)
    }
}

/// Dodge Challenger Hellcat: 6.2 L supercharged V8 (the whine follows the rpm).
pub fn dodge_hellcat() -> EngineCfg {
    EngineCfg {
        power_kw: 527.0,
        torque_nm: 881.0,
        intake_boost: 0.97,
        clutch_capacity_scale: 2.0,
        jitter: 0.4,
        gearbox: gearbox(&[4.71, 3.14, 2.10, 1.67, 1.29, 1.0, 0.84, 0.67], 3.09, 2020.0, 0.36, 0.85, 0.3),
        cylinders: firing(&[1, 8, 4, 3, 6, 5, 7, 2], 90.0, |n| usize::from(n > 4), |n| 0.30 + 0.02 * ((n - 1) % 4) as f64),
        exhausts: pair(100.0, 26.0, Ir::Real1),
        turbo: turbo(1.7, 600.0, 1_800.0, 0.03, 1.2),
        ..eng("Dodge Hellcat V8 supercharged", 0.103, 0.0928, 9.5, 6_200.0, 700.0, 0.35)
    }
}

/// Toyota Supra A90 (B58): 3.0 L turbo inline-six.
pub fn toyota_supra() -> EngineCfg {
    EngineCfg {
        power_kw: 285.0,
        torque_nm: 500.0,
        intake_boost: 0.98,
        clutch_capacity_scale: 2.0,
        gearbox: gearbox(&[5.25, 3.36, 2.17, 1.72, 1.32, 1.0, 0.82, 0.64], 3.15, 1550.0, 0.34, 0.62, 0.25),
        cylinders: firing(&[1, 5, 3, 6, 2, 4], 120.0, |_| 0, |n| 0.25 + 0.02 * ((n - 1) % 3) as f64),
        exhausts: vec![exhaust(1.0, 2.0, Ir::Real2)],
        turbo: turbo(2.0, 1_500.0, 3_200.0, 0.2, 0.6),
        ..eng("Toyota Supra A90 B58 I6 turbo", 0.082, 0.0946, 10.2, 7_000.0, 750.0, 0.2)
    }
}

/// Honda S2000 (F20C): 2.0 L inline-four, 9,000 rpm.
pub fn honda_s2000() -> EngineCfg {
    EngineCfg {
        power_kw: 177.0,
        torque_nm: 208.0,
        intake_boost: 1.34,
        intake_duration_deg: 255.0,
        exhaust_duration_deg: 250.0,
        input_cutoff: 4500.0,
        gearbox: gearbox(&[3.13, 2.05, 1.48, 1.16, 0.97, 0.81], 4.1, 1250.0, 0.32, 0.62, 0.4),
        cylinders: firing(&[1, 3, 4, 2], 180.0, |_| 0, |n| 0.3 + 0.02 * ((n - 1) % 2) as f64),
        exhausts: vec![exhaust(0.9, 2.0, Ir::Real2)],
        ..eng("Honda S2000 F20C I4", 0.087, 0.084, 11.0, 9_000.0, 900.0, 0.12)
    }
}

/// Honda Civic Type R (B16B): 1.6 L inline-four, 8,400 rpm (fixed high-lift cam, no VTEC switch).
pub fn honda_civic_type_r() -> EngineCfg {
    EngineCfg {
        power_kw: 118.0,
        torque_nm: 160.0,
        intake_boost: 1.02,
        intake_duration_deg: 255.0,
        exhaust_duration_deg: 245.0,
        input_cutoff: 4500.0,
        gearbox: gearbox(&[3.23, 2.11, 1.52, 1.17, 0.95, 0.77], 4.75, 1100.0, 0.30, 0.62, 0.4),
        cylinders: firing(&[1, 3, 4, 2], 180.0, |_| 0, |n| 0.3 + 0.02 * ((n - 1) % 2) as f64),
        exhausts: vec![exhaust(0.9, 2.2, Ir::Real2)],
        ..eng("Honda Civic Type R B16B I4", 0.081, 0.077, 10.8, 8_400.0, 900.0, 0.1)
    }
}

/// Mazda MX-5 ND (SkyActiv-G 2.0).
pub fn mazda_mx5() -> EngineCfg {
    EngineCfg {
        power_kw: 135.0,
        torque_nm: 205.0,
        intake_boost: 1.13,
        gearbox: gearbox(&[3.76, 2.27, 1.65, 1.18, 1.0, 0.83], 3.45, 1060.0, 0.31, 0.62, 0.4),
        cylinders: firing(&[1, 3, 4, 2], 180.0, |_| 0, |n| 0.3 + 0.02 * ((n - 1) % 2) as f64),
        exhausts: vec![exhaust(1.0, 2.0, Ir::Real2)],
        ..eng("Mazda MX-5 ND SkyActiv I4", 0.0835, 0.0912, 13.0, 7_500.0, 850.0, 0.15)
    }
}

/// Cummins ISB 6.7 L: inline-six diesel (harsh combustion, "clatter").
pub fn cummins_diesel() -> EngineCfg {
    EngineCfg {
        power_kw: 224.0,
        torque_nm: 900.0,
        intake_boost: 1.42,
        burn_duration_deg: 22.0,
        hf_gain: 0.04,
        noise: 2.0,
        jitter: 0.6,
        input_cutoff: 2500.0,
        idle_throttle_min: 0.05,
        clutch_capacity_scale: 2.5,
        gearbox: Gearbox { traction_g: 0.7, ..gearbox(&[7.0, 4.1, 2.7, 1.9, 1.35, 1.0], 3.7, 4200.0, 0.40, 3.2, 0.7) },
        cylinders: firing(&[1, 5, 3, 6, 2, 4], 120.0, |_| 0, |n| 0.25 + 0.02 * ((n - 1) % 3) as f64),
        exhausts: vec![exhaust(2.5, 6.0, Ir::Real2)],
        ..eng("Cummins ISB diesel 6.7L I6", 0.107, 0.124, 17.3, 3_200.0, 750.0, 0.9)
    }
}

/// 125 cc two-stroke kart: one firing per revolution, modelled as two "cylinders" 360° apart.
pub fn kart_2t() -> EngineCfg {
    EngineCfg {
        power_kw: 28.0,
        torque_nm: 22.0,
        intake_boost: 1.35,
        burn_duration_deg: 40.0,
        intake_duration_deg: 200.0,
        exhaust_duration_deg: 200.0,
        hf_gain: 0.03,
        noise: 2.0,
        jitter: 0.5,
        input_cutoff: 5000.0,
        gearbox: Gearbox { traction_g: 1.0, ..gearbox(&[1.0], 2.9, 175.0, 0.14, 0.35, 0.3) },
        cylinders: firing(&[1, 2], 360.0, |_| 0, |_| 0.15),
        exhausts: vec![exhaust(0.6, 3.0, Ir::Real0)],
        ..eng("Kart 125cc 2-stroke", 0.038, 0.0545, 12.0, 14_000.0, 3_500.0, 0.04)
    }
}

/// Rolls-Royce Merlin: 27 L 60° V12 (Spitfire), 3,000 rpm.
pub fn rolls_royce_merlin() -> EngineCfg {
    EngineCfg {
        power_kw: 1100.0,
        torque_nm: 3500.0,
        intake_boost: 1.0,
        jitter: 0.3,
        noise: 1.5,
        dynamic_pressure_gain: 3.0,
        input_cutoff: 2000.0,
        idle_throttle_min: 0.04,
        turbo: turbo(1.8, 500.0, 2_000.0, 0.05, 0.8),
        gearbox: Gearbox { traction_g: 0.8, ..gearbox(&[1.0], 2.0, 3000.0, 0.4, 2.2, 0.5) },
        cylinders: firing(&[1, 12, 4, 9, 2, 11, 6, 7, 3, 10, 5, 8], 60.0, |n| usize::from(n > 6), |n| 0.25 + 0.03 * ((n - 1) % 6) as f64),
        exhausts: vec![exhaust(0.25, 20.0, Ir::Real1), exhaust(0.25, 20.0, Ir::Real1)],
        // The inertia includes the propeller; without it the limiter cannot hold the redline.
        ..eng("Rolls-Royce Merlin V12", 0.1372, 0.152, 6.0, 3_000.0, 700.0, 6.0)
    }
}

/// Citroën C3: PureTech 1.2 L three-cylinder turbo (110 hp), firing every 240°.
pub fn citroen_c3() -> EngineCfg {
    EngineCfg {
        power_kw: 81.0,
        torque_nm: 205.0,
        intake_boost: 0.87,
        clutch_capacity_scale: 2.0,
        jitter: 0.4,
        input_cutoff: 3000.0,
        gearbox: gearbox(&[3.73, 1.96, 1.28, 0.93, 0.74, 0.62], 4.2, 1090.0, 0.30, 0.62, 0.45),
        cylinders: firing(&[1, 2, 3], 240.0, |_| 0, |n| 0.28 + 0.03 * (n - 1) as f64),
        exhausts: vec![exhaust(0.9, 2.5, Ir::Real2)],
        turbo: turbo(1.6, 1_500.0, 3_000.0, 0.25, 0.3),
        ..eng("Citroen C3 PureTech 1.2 3-cylinder turbo", 0.075, 0.0905, 10.5, 6_000.0, 800.0, 0.14)
    }
}

/// WRC Rally1 (Hyundai i20 N / Ford Puma): restricted 1.6 L turbo four, sequential gearbox.
pub fn rally1() -> EngineCfg {
    EngineCfg {
        power_kw: 280.0,
        torque_nm: 450.0,
        intake_boost: 1.27,
        clutch_capacity_scale: 2.5,
        idle_throttle_min: 0.05,
        jitter: 0.3,
        hf_gain: 0.02,
        dynamic_pressure_gain: 4.0,
        input_cutoff: 4000.0,
        gearbox: Gearbox { traction_g: 1.0, ..gearbox(&[2.9, 2.0, 1.5, 1.2, 1.0], 4.6, 1260.0, 0.32, 0.8, 0.15) },
        cylinders: firing(&[1, 3, 4, 2], 180.0, |_| 0, |n| 0.22 + 0.02 * ((n - 1) % 2) as f64),
        exhausts: vec![exhaust(0.8, 4.0, Ir::Real1)],
        turbo: turbo(2.4, 3_000.0, 5_000.0, 0.15, 0.9),
        anti_lag: true,
        ..eng("WRC Rally1 1.6L turbo", 0.083, 0.0738, 10.0, 8_500.0, 1_500.0, 0.09)
    }
}

/// Audi Sport Quattro S1 (Group B): 2.1 L turbo inline-five, order 1-2-4-5-3.
pub fn audi_quattro_s1() -> EngineCfg {
    EngineCfg {
        power_kw: 440.0,
        torque_nm: 600.0,
        intake_boost: 2.8,
        clutch_capacity_scale: 2.5,
        jitter: 0.5,
        hf_gain: 0.02,
        noise: 1.5,
        input_cutoff: 3200.0,
        gearbox: Gearbox { traction_g: 1.0, ..gearbox(&[2.9, 2.0, 1.45, 1.1, 0.88], 4.1, 1090.0, 0.33, 0.7, 0.4) },
        cylinders: firing(&[1, 2, 4, 5, 3], 144.0, |_| 0, |n| 0.25 + 0.02 * (n - 1) as f64),
        exhausts: vec![exhaust(0.9, 4.0, Ir::Real2)],
        turbo: turbo(2.8, 3_000.0, 6_000.0, 0.45, 0.5),
        anti_lag: true,
        ..eng("Audi Sport Quattro S1 I5 turbo", 0.0793, 0.0864, 8.0, 8_000.0, 1_000.0, 0.15)
    }
}

/// Koenigsegg Jesko: 5.0 L twin-turbo flat-plane V8, 8,500 rpm, 9-speed multi-clutch gearbox.
pub fn koenigsegg_jesko() -> EngineCfg {
    EngineCfg {
        intake_boost: 2.19,
        power_kw: 955.0,
        torque_nm: 1370.0,
        clutch_capacity_scale: 3.0,
        intake_duration_deg: 250.0,
        exhaust_duration_deg: 245.0,
        intake_center_deg: 108.0,
        exhaust_center_deg: 110.0,
        jitter: 0.15,
        dynamic_pressure_gain: 3.0,
        input_cutoff: 3800.0,
        sim_rate: 10_000.0,
        gearbox: Gearbox {
            downforce_area: 2.0,
            traction_g: 1.3,
            ..gearbox(&[3.20, 2.40, 1.90, 1.55, 1.30, 1.10, 0.95, 0.82, 0.70], 3.2, 1420.0, 0.36, 0.72, 0.1)
        },
        // Flat-plane crank: even firing every 90°, banks alternating.
        cylinders: firing(&[1, 5, 3, 7, 4, 8, 2, 6], 90.0, |n| usize::from(n > 4), |n| 0.15 + 0.02 * ((n - 1) % 4) as f64),
        exhausts: pair(60.0, 18.0, Ir::Real0),
        turbo: turbo(2.3, 2_500.0, 5_000.0, 0.2, 0.6),
        ..eng("Koenigsegg Jesko V8 twin-turbo", 0.095, 0.088, 9.3, 8_500.0, 1_000.0, 0.18)
    }
}

/// Pagani Zonda R: 6.0 L naturally aspirated 60° V12 (AMG M297), 7,500 rpm peak power.
pub fn pagani_zonda_r() -> EngineCfg {
    EngineCfg {
        intake_boost: 1.32,
        power_kw: 559.0,
        torque_nm: 710.0,
        clutch_capacity_scale: 1.6,
        intake_duration_deg: 260.0,
        exhaust_duration_deg: 255.0,
        intake_center_deg: 108.0,
        exhaust_center_deg: 110.0,
        jitter: 0.2,
        noise: 1.2,
        dynamic_pressure_gain: 3.0,
        input_cutoff: 4200.0,
        sim_rate: 8_000.0,
        gearbox: Gearbox {
            downforce_area: 1.5,
            traction_g: 1.25,
            ..gearbox(&[3.0, 2.1, 1.65, 1.35, 1.12, 0.95], 3.1, 1070.0, 0.34, 0.70, 0.15)
        },
        // Even firing every 60°; short quad-pipe exhaust, raw and loud.
        cylinders: firing(
            &[1, 7, 4, 10, 2, 8, 6, 12, 3, 9, 5, 11],
            60.0,
            |n| usize::from(n > 6),
            |n| 0.012 * (1 + (n - 1) % 6) as f64,
        ),
        exhausts: pair(30.0, 26.0, Ir::Real2),
        ..eng("Pagani Zonda R V12", 0.089, 0.0802, 12.0, 8_000.0, 1_000.0, 0.25)
    }
}
