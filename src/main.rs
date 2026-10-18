mod ambience;
mod app;
mod config;
mod driver;
mod dsp;
mod engine;
mod help;
#[cfg(feature = "gui")]
mod gui;
mod pipeline;
mod pilot;
mod presets;
mod synth;
mod ui;
mod view;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use driver::Driver;
use engine::{Controls, EngineCfg};
use pipeline::Pipeline;

fn usage() -> ! {
    eprintln!(
        "usage: engine-sim-rs [--gui] [--engine ID] [--engines DIR|FILE] [--list]\n\
         \x20      engine-sim-rs --wav FILE.wav [--engine ID] [--seconds N] [--drive] [--steady RPM] [--mic exhaust|cabin|bay]\n\
         \x20      engine-sim-rs --probe | --accel | --dump-toml ID | --dump ID FILE\n\
         \n  --list           list the available engines\n  \
         --gui            graphical interface (egui) instead of the TUI\n  \
         --engine ID      choose the engine (default: gm_ls_v8)\n  \
         --engines PATH   extra directory or TOML file of engines (before --list)\n  \
         --wav FILE       render a scenario offline instead of playing live\n  \
         --seconds N      render length (default: 20)\n  \
         --drive          scenario with gearbox: start, first gear, upshifts\n  \
         --steady RPM     hold the given rpm in neutral\n  \
         --mic NAME       microphone position: exhaust, cabin or bay\n  \
         --set key=value  fine tuning for the offline render (hf_gain, noise, jitter, input_cutoff,\n                   air_noise_cutoff, dynamic_pressure_gain, pipe_volume_fraction, ir, ...)\n  \
         --probe          free-rev torque and power curve of the chosen engine\n  \
         --accel          0-100 / 0-200 / 0-300 km/h times of the chosen engine\n  \
         --dump-toml ID   print an engine as TOML (a starting point for your own)\n  \
         --dump ID FILE   raw source signal of an engine"
    );
    std::process::exit(2);
}

fn main() {
    let mut engine_id = String::from("gm_ls_v8");
    let mut wav: Option<String> = None;
    let mut seconds = 20.0f64;
    let mut drive = false;
    let mut gui = false;
    let mut mic = 0usize;
    let mut probe = false;
    let mut accel = false;
    let mut steady: Option<f64> = None;
    let mut overrides: Vec<String> = Vec::new();

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--list" => {
                let (engines, errors) = config::load_all();
                for e in &errors {
                    eprintln!("warning: {e}");
                }
                for e in engines {
                    println!("{:<28} {}", e.id(), e.name);
                }
                return;
            }
            "--engine" => engine_id = args.next().unwrap_or_else(|| usage()),
            "--wav" => wav = Some(args.next().unwrap_or_else(|| usage())),
            "--seconds" => {
                seconds = args.next().and_then(|s| s.parse().ok()).unwrap_or_else(|| usage())
            }
            "--drive" => drive = true,
            "--gui" => gui = true,
            "--mic" => {
                mic = match args.next().as_deref() {
                    Some("cabin") => 1,
                    Some("bay") => 2,
                    _ => 0,
                }
            }
            "--steady" => {
                steady = Some(args.next().and_then(|s| s.parse().ok()).unwrap_or_else(|| usage()))
            }
            "--set" => overrides.push(args.next().unwrap_or_else(|| usage())),
            "--dump" => {
                let id = args.next().unwrap_or_else(|| usage());
                let path = args.next().unwrap_or_else(|| usage());
                let all = config::load_all().0;
                dump_source(all.iter().find(|e| e.id() == id).unwrap(), &path);
                return;
            }
            "--engines" => config::add_dir(args.next().unwrap_or_else(|| usage()).into()),
            "--dump-toml" => {
                let id = args.next().unwrap_or_else(|| usage());
                match config::load_all().0.iter().find(|e| e.id() == id) {
                    Some(e) => print!("{}", config::dump(e)),
                    None => eprintln!("unknown engine: {id}"),
                }
                return;
            }
            "--probe" => probe = true,
            "--accel" => accel = true,
            _ => usage(),
        }
    }

    let (presets, errors) = config::load_all();
    for e in &errors {
        eprintln!("warning: {e}");
    }
    let index = presets.iter().position(|e| e.id() == engine_id).unwrap_or_else(|| {
        eprintln!("unknown engine: {engine_id} (see --list)");
        std::process::exit(1);
    });

    let mut cfg = presets[index].clone();
    for o in &overrides {
        apply_override(&mut cfg, o);
    }

    if probe {
        free_rev_probe(&cfg);
        return;
    }
    if accel {
        accel_probe(&cfg);
        return;
    }

    match wav {
        Some(path) => render_scenario(&cfg, &path, seconds, drive, steady, mic),
        None => play_live(presets, index, gui),
    }
}

fn play_live(presets: Vec<EngineCfg>, index: usize, gui: bool) {
    let shared = Arc::new(app::Shared::new());
    shared.engine_index.store(index, Ordering::Relaxed);

    let (stream, thread) = match app::start(presets.clone(), shared.clone()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("audio unavailable: {e}");
            std::process::exit(1);
        }
    };

    let result: Result<(), String> = if gui {
        #[cfg(feature = "gui")]
        {
            gui::run(presets, shared.clone()).map_err(|e| e.to_string())
        }
        #[cfg(not(feature = "gui"))]
        {
            Err("built without the `gui` feature".to_string())
        }
    } else {
        let terminal = ratatui::init();
        let result = ui::run(terminal, presets, shared.clone());
        ratatui::restore();
        result.map_err(|e| e.to_string())
    };

    shared.quit.store(true, Ordering::Relaxed);
    let _ = thread.join();
    drop(stream);
    if let Err(e) = result {
        eprintln!("interface error: {e}");
    }
}

/// Offline scenario. Without `--drive`: idle, then a free rev-up.
/// With `--drive`: first gear, then upshifts at 85 % of redline.
fn render_scenario(cfg: &EngineCfg, path: &str, seconds: f64, drive: bool, steady: Option<f64>, mic: usize) {
    const RATE: u32 = 44_100;
    let mut pipe = Pipeline::new(cfg, RATE as f64);
    pipe.ambience.mic = ambience::Mic::from_index(mic);
    let mut driver = Driver::new(&cfg.gearbox);
    let mut writer = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 1,
            sample_rate: RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .expect("cannot create the WAV file");

    let chunk = (cfg.sim_rate / 50.0) as usize;
    let dt = chunk as f64 / cfg.sim_rate;
    let shift_rpm = cfg.redline_rpm * 0.85;
    let mut out = Vec::new();
    let mut t = 0.0;
    let mut next_report = 0.0;
    let mut last_shift = 0.0;
    let mut hold = 0.2f64;
    while t < seconds {
        let rpm = pipe.sim.rpm();
        let throttle = if let Some(target) = steady {
            // PI control of the free-rev speed (neutral) after starting.
            if t < 4.0 {
                0.0
            } else {
                hold = (hold + 3.0e-4 * (target - rpm) * dt).clamp(0.0, 1.0);
                (hold + 1.0e-4 * (target - rpm)).clamp(0.0, 1.0)
            }
        } else if drive {
            match t {
                t if t < 4.0 => 0.0,
                // A small throttle blip to pull away, then full throttle.
                t if t < 4.8 => 0.35,
                _ => 1.0,
            }
        } else {
            match t {
                t if t < 5.0 => 0.0,
                t if t < seconds * 0.75 => ((t - 5.0) / (seconds * 0.75 - 5.0)).clamp(0.0, 1.0),
                _ => 0.0,
            }
        };

        if drive {
            if t >= 4.0 && driver.gear() == 0 && last_shift == 0.0 {
                driver.request(1);
                last_shift = t;
            } else if rpm > shift_rpm && t - last_shift > 1.0 && driver.gear() < driver.gear_count() {
                driver.request(driver.gear() + 1);
                last_shift = t;
            }
        }

        let engagement = driver.update(dt, 0.0, pipe.sim.speed_kmh() / 3.6);
        let ctl = Controls {
            ignition: t > 0.3,
            starter: t < 4.0 && rpm < 450.0 && t > 0.3,
            throttle: if drive { driver.shape_throttle(throttle) } else { throttle },
            gear_ratio: driver.gear_ratio(),
            clutch: engagement,
            brake: 0.0,
        };
        out.clear();
        pipe.run(&ctl, chunk, &mut out);
        for &s in &out {
            writer.write_sample(s).unwrap();
        }
        t += dt;
        if t >= next_report {
            println!(
                "t={:5.1}s  rpm={:6.0}  throttle={:3.0}%  gear={}  speed={:5.1} km/h  clutch={:.2}  gain={:.4}",
                t,
                pipe.sim.rpm(),
                pipe.sim.throttle() * 100.0,
                driver.gear(),
                pipe.sim.speed_kmh(),
                engagement,
                pipe.synth.leveler_gain()
            );
            next_report += 1.0;
        }
    }
    writer.finalize().unwrap();
    println!("written: {path}");
}

#[allow(dead_code)]
fn free_rev_probe(cfg: &EngineCfg) {
    // Net torque = inertia × angular acceleration during a full-throttle free rev-up.
    let mut pipe = Pipeline::new(cfg, 44_100.0);
    let mut out = Vec::new();
    let chunk = (cfg.sim_rate / 200.0) as usize;
    let dt = chunk as f64 / cfg.sim_rate;
    let mut t = 0.0;
    let mut hist: Vec<(f64, f64)> = Vec::new();
    while t < 8.0 {
        let throttle = if t < 3.0 { 0.0 } else { 1.0 };
        let ctl = Controls { throttle, ignition: t > 0.3, starter: t < 2.0 && pipe.sim.rpm() < 450.0, gear_ratio: 0.0, clutch: 1.0, brake: 0.0 };
        out.clear();
        pipe.run(&ctl, chunk, &mut out);
        t += dt;
        if t > 3.0 {
            hist.push((t, pipe.sim.rpm()));
        }
    }
    let max_rpm = hist.iter().map(|h| h.1).fold(0.0, f64::max);
    println!("{} — net torque / free-rev power (full throttle); max rpm reached {max_rpm:.0} (redline {:.0})", cfg.name, cfg.redline_rpm);
    let step = 6;
    let mut last_report = 0.0;
    for w in hist.windows(step + 1).step_by(1) {
        let (t0, r0) = w[0];
        let (t1, r1) = w[step];
        let rpm = 0.5 * (r0 + r1);
        if rpm >= last_report + 1500.0 && t1 > t0 {
            let alpha = (r1 - r0) * 2.0 * std::f64::consts::PI / 60.0 / (t1 - t0);
            let torque = cfg.inertia * alpha;
            let power = torque * rpm * 2.0 * std::f64::consts::PI / 60.0;
            println!("  {rpm:6.0} rpm  {torque:6.0} N·m  {:5.0} kW", power / 1000.0);
            last_report = rpm;
        }
    }
}

#[allow(dead_code)]
fn dump_source(cfg: &EngineCfg, path: &str) {
    use engine::EngineSim;
    let mut sim = EngineSim::new(cfg);
    let mut flows = vec![0.0; cfg.exhausts.len()];
    let mut dump = String::new();
    let steps = (cfg.sim_rate * 6.0) as usize;
    for i in 0..steps {
        let t = i as f64 / cfg.sim_rate;
        let ctl = Controls { throttle: if t < 2.5 { 0.0 } else { 1.0 }, ignition: t > 0.3, starter: t < 2.0 && sim.rpm() < 450.0, gear_ratio: 0.0, clutch: 1.0, brake: 0.0 };
        sim.step(&ctl, &mut flows);
        if t > 4.0 {
            dump.push_str(&format!("{:.6}\n", flows[0]));
        }
        if i % (cfg.sim_rate as usize) == 0 { eprintln!("t={t:.0} rpm={:.0}", sim.rpm()); }
    }
    std::fs::write(path, dump).unwrap();
}

/// Fine tuning for experiments: `--set key=value` (offline render only).
fn apply_override(cfg: &mut EngineCfg, spec: &str) {
    let (key, value) = spec.split_once('=').unwrap_or_else(|| usage());
    let v: f64 = value.parse().unwrap_or_else(|_| usage());
    match key {
        "hf_gain" => cfg.hf_gain = v as f32,
        "noise" => cfg.noise = v as f32,
        "jitter" => cfg.jitter = v as f32,
        "input_cutoff" => cfg.input_cutoff = v as f32,
        "air_noise_cutoff" => cfg.air_noise_cutoff = v as f32,
        "audio_volume" => cfg.exhausts.iter_mut().for_each(|e| e.audio_volume = v),
        "combustion_efficiency" => cfg.combustion_efficiency = v,
        "burn_duration_deg" => cfg.burn_duration_deg = v,
        "intake_boost" => cfg.intake_boost = v,
        "dynamic_pressure_gain" => cfg.dynamic_pressure_gain = v,
        "pipe_volume_fraction" => cfg.pipe_volume_fraction = v,
        "friction_scale" => cfg.friction_scale = v,
        "inertia" => cfg.inertia = v,
        "intake_lift" => cfg.intake_lift = v,
        "exhaust_lift" => cfg.exhaust_lift = v,
        "intake_valve_frac" => cfg.intake_valve_frac = v,
        "exhaust_valve_frac" => cfg.exhaust_valve_frac = v,
        "intake_duration_deg" => cfg.intake_duration_deg = v,
        "exhaust_duration_deg" => cfg.exhaust_duration_deg = v,
        "primary_area" => cfg.primary_area = v,
        "compression_ratio" => cfg.compression_ratio = v,
        "eq_off" => cfg.eq.clear(),
        "ir" => cfg.exhausts.iter_mut().for_each(|e| {
            e.ir = [engine::Ir::Default0, engine::Ir::Real0, engine::Ir::Real1, engine::Ir::Real2][v as usize]
        }),
        "imbalance" => {
            let base = cfg.exhausts[0].audio_volume;
            for e in cfg.exhausts.iter_mut().skip(1) {
                e.audio_volume = base * v;
            }
        }
        k if k.starts_with("eq") => {
            use dsp::{EqBand, EqKind};
            if cfg.eq.is_empty() {
                cfg.eq = vec![
                    EqBand { kind: EqKind::LowShelf, freq: 300.0, gain_db: 0.0 },
                    EqBand { kind: EqKind::HighShelf, freq: 1400.0, gain_db: 0.0 },
                    EqBand { kind: EqKind::HighShelf, freq: 6500.0, gain_db: 0.0 },
                    EqBand { kind: EqKind::Peak, freq: 500.0, gain_db: 0.0 },
                ];
            }
            let band = (k.as_bytes()[2] - b'0') as usize;
            match &k[4..] {
                "gain" => cfg.eq[band].gain_db = v,
                "freq" => cfg.eq[band].freq = v,
                _ => usage(),
            }
        }
        _ => {
            eprintln!("unknown key: {key}");
            std::process::exit(2);
        }
    }
}

/// Standing start at full throttle with automatic shifts: time to reach 100/200/300 km/h.
fn accel_probe(cfg: &EngineCfg) {
    use engine::EngineSim;
    let mut sim = EngineSim::new(cfg);
    let mut driver = Driver::new(&cfg.gearbox);
    let mut flows = vec![0.0; cfg.exhausts.len()];
    let dt = 1.0 / cfg.sim_rate;
    let chunk = (cfg.sim_rate / 50.0) as usize;
    let mut t = 0.0;
    let mut launch = None::<f64>;
    let mut marks = [(100.0, None::<f64>), (200.0, None), (300.0, None)];
    let mut last_shift = 0.0;
    let mut engagement = 1.0;
    let mut peak_g: f64 = 0.0;
    let mut pending: Option<(f64, usize)> = None;
    let mut shift_min = f64::MAX;
    let mut last_speed = 0.0;
    let mut g_at: Vec<(f64, f64)> = Vec::new();
    while t < 40.0 {
        let rpm = sim.rpm();
        let go = t > 4.0;
        if go && launch.is_none() {
            driver.request(1);
            launch = Some(t);
            last_shift = t;
        } else if launch.is_some()
            && engagement > 0.99
            && rpm > cfg.redline_rpm * 0.92
            && t - last_shift > 0.6
            && driver.gear() < driver.gear_count()
        {
            if std::env::var("SHIFTS").is_ok() {
                println!("  shift {} -> {} : {:.0} rpm at {:.0} km/h", driver.gear(), driver.gear() + 1, rpm, sim.speed_kmh());
                pending = Some((t + 0.7, driver.gear() + 1));
            }
            driver.request(driver.gear() + 1);
            last_shift = t;
        }
        if pending.is_some() {
            shift_min = shift_min.min(sim.rpm());
        } else {
            shift_min = f64::MAX;
        }
        if let Some((when, g)) = pending {
            if t >= when {
                println!("      gear {g} engaged: minimum rpm during the shift {:.0}, then {:.0} rpm at {:.0} km/h", shift_min, sim.rpm(), sim.speed_kmh());
                pending = None;
            }
        }
        for _ in 0..chunk {
            engagement = driver.update(dt, 0.0, sim.speed_kmh() / 3.6);
            let ctl = Controls {
                // Gentle start while the clutch bites, then full throttle.
                throttle: driver.shape_throttle(match launch {
                    Some(l) if t - l < 1.1 => 0.45,
                    Some(_) => 1.0,
                    None => 0.0,
                }),
                ignition: t > 0.3,
                starter: t < 4.0 && sim.rpm() < 450.0 && t > 0.3,
                gear_ratio: driver.gear_ratio(),
                clutch: engagement,
                brake: 0.0,
            };
            sim.step(&ctl, &mut flows);
            t += dt;
        }
        let _ = engagement;
        let v = sim.speed_kmh();
        if let Some(l) = launch {
            let g = (v - last_speed) / 3.6 / (chunk as f64 * dt) / 9.81;
            peak_g = peak_g.max(g);
            if ((t - l) * 2.0).fract() < chunk as f64 * dt * 2.0 {
                g_at.push((v, g));
                if std::env::var("TRACE").is_ok() {
                    println!("  t={:5.1} v={:5.0} km/h  rpm={:6.0}  gear={}  a={:4.2} g  clutch={:.2}  Tavg={:5.0} Tclutch={:5.0} boost={:.2}", t - l, v, sim.rpm(), driver.gear(), g, engagement, sim.dbg[3], sim.dbg[1], sim.dbg[2]);
                }
            }
            for m in marks.iter_mut() {
                if m.1.is_none() && v >= m.0 {
                    m.1 = Some(t - l);
                }
            }
        }
        last_speed = v;
        if marks[2].1.is_some() {
            break;
        }
    }
    let fmt = |x: Option<f64>| x.map_or("   - ".to_string(), |v| format!("{v:5.1}"));
    println!(
        "{:<38} 0-100: {} s   0-200: {} s   0-300: {} s   peak {:.2} g   vmax {:.0} km/h",
        cfg.name, fmt(marks[0].1), fmt(marks[1].1), fmt(marks[2].1), peak_g, sim.speed_kmh()
    );
    let _ = g_at;
}
