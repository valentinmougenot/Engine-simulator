//! State shared between the UI, the simulation/synthesis thread and the audio output.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::traits::*;
use ringbuf::HeapRb;

use crate::driver::Driver;
use crate::engine::{Controls, EngineCfg};
use crate::pipeline::Pipeline;

pub struct F32(AtomicU32);

impl F32 {
    pub fn new(v: f32) -> Self {
        Self(AtomicU32::new(v.to_bits()))
    }
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
    pub fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed)
    }
}

/// Scope points: one sample out of `SCOPE_DECIMATION`.
pub const SCOPE_DECIMATION: usize = 8;
pub const SCOPE_POINTS: usize = 600;

pub struct Shared {
    // Commands (UI -> simulation)
    pub throttle: F32,
    pub volume: F32,
    pub convolution: F32,
    pub clutch_pedal: F32,
    pub brake: F32,
    pub ignition: AtomicBool,
    pub starter: AtomicBool,
    pub quit: AtomicBool,
    pub gear_request: AtomicI32,
    pub engine_index: AtomicUsize,
    pub mic: AtomicUsize,
    /// Bumped by the UI to request a reload of the TOML files.
    pub reload_gen: AtomicUsize,
    /// Target idle speed (rpm), set by the UI; reset to the engine's idle on every engine change.
    pub idle_rpm: F32,
    // Telemetry (simulation -> UI)
    pub rpm: F32,
    pub speed_kmh: F32,
    pub gear: AtomicI32,
    pub clutch_engagement: F32,
    pub throttle_effective: F32,
    pub leveler_gain: F32,
    /// Engine torque (N·m) and power (kW) at the crankshaft.
    pub torque: F32,
    pub power: F32,
    pub active_engine: AtomicUsize,
    pub scope: Mutex<Vec<f32>>,
    /// P-V diagram of cylinder 1: (volume m³, pressure Pa).
    pub pv: Mutex<Vec<[f32; 2]>>,
    pub firing: AtomicBool,
}

impl Shared {
    pub fn new() -> Self {
        Self {
            throttle: F32::new(0.0),
            volume: F32::new(0.7),
            convolution: F32::new(1.0),
            clutch_pedal: F32::new(0.0),
            brake: F32::new(0.0),
            ignition: AtomicBool::new(true),
            starter: AtomicBool::new(true),
            quit: AtomicBool::new(false),
            gear_request: AtomicI32::new(0),
            engine_index: AtomicUsize::new(0),
            mic: AtomicUsize::new(0),
            reload_gen: AtomicUsize::new(0),
            idle_rpm: F32::new(0.0),
            rpm: F32::new(0.0),
            speed_kmh: F32::new(0.0),
            gear: AtomicI32::new(0),
            clutch_engagement: F32::new(1.0),
            throttle_effective: F32::new(0.0),
            torque: F32::new(0.0),
            power: F32::new(0.0),
            leveler_gain: F32::new(1.0),
            active_engine: AtomicUsize::new(0),
            scope: Mutex::new(Vec::with_capacity(SCOPE_POINTS)),
            pv: Mutex::new(Vec::new()),
            firing: AtomicBool::new(false),
        }
    }
}

/// Starts the audio output and the simulation thread. Returns the stream to keep alive
/// and the thread handle.
pub fn start(
    presets: Vec<EngineCfg>,
    shared: Arc<Shared>,
) -> Result<(cpal::Stream, std::thread::JoinHandle<()>), String> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("no audio output device")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let config: cpal::StreamConfig = supported.clone().into();
    let rate = config.sample_rate as f64;
    let channels = config.channels as usize;

    // ~250 ms buffer; the producer only fills up to ~60 ms to keep
    // latency low between a key press and the sound.
    let ring = HeapRb::<i16>::new((rate * 0.25) as usize);
    let (producer, consumer) = ring.split();
    let target_fill = (rate * 0.06) as usize;

    let thread = {
        let shared = shared.clone();
        std::thread::spawn(move || simulation_loop(presets, shared, producer, rate, target_fill))
    };

    let err_fn = |e: cpal::Error| eprintln!("audio error: {e}");
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => build_stream::<f32>(&device, config, channels, consumer, err_fn),
        cpal::SampleFormat::I16 => build_stream::<i16>(&device, config, channels, consumer, err_fn),
        f => return Err(format!("unsupported sample format: {f}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, thread))
}

fn build_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    mut consumer: impl Consumer<Item = i16> + Send + 'static,
    err_fn: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                for frame in data.chunks_mut(channels) {
                    let s = consumer.try_pop().unwrap_or(0) as f32 / 32768.0;
                    let v = T::from_sample(s);
                    frame.iter_mut().for_each(|o| *o = v);
                }
            },
            err_fn,
            None,
        )
        .map_err(|e| e.to_string())
}

fn simulation_loop(
    mut presets: Vec<EngineCfg>,
    shared: Arc<Shared>,
    mut producer: impl Producer<Item = i16>,
    rate: f64,
    target_fill: usize,
) {
    let mut index = shared.engine_index.load(Ordering::Relaxed);
    let mut cfg = presets[index].clone();
    let mut pipe = Pipeline::new(&cfg, rate);
    let mut driver = Driver::new(&cfg.gearbox);
    let mut out = Vec::with_capacity(4096);
    let mut starter_since = Instant::now();
    let mut was_starting = false;
    let mut scope_counter = 0usize;
    let mut last_request = 0;
    let mut last_generation = shared.reload_gen.load(Ordering::Relaxed);
    shared.active_engine.store(index, Ordering::Relaxed);
    shared.idle_rpm.set(cfg.idle_rpm as f32);

    while !shared.quit.load(Ordering::Relaxed) {
        if producer.occupied_len() >= target_fill {
            std::thread::sleep(Duration::from_micros(500));
            continue;
        }

        // Hot engine switch.
        let wanted = shared.engine_index.load(Ordering::Relaxed);
        let generation = shared.reload_gen.load(Ordering::Relaxed);
        let reload = generation != last_generation;
        if reload {
            last_generation = generation;
            presets = crate::config::load_all().0;
        }
        if wanted != index || reload {
            index = wanted.min(presets.len() - 1);
            cfg = presets[index].clone();
            pipe = Pipeline::new(&cfg, rate);
            driver = Driver::new(&cfg.gearbox);
            last_request = 0;
            shared.gear_request.store(0, Ordering::Relaxed);
            shared.active_engine.store(index, Ordering::Relaxed);
            shared.idle_rpm.set(cfg.idle_rpm as f32);
            was_starting = false;
            shared.starter.store(true, Ordering::Relaxed);
        }

        let starting = shared.starter.load(Ordering::Relaxed);
        if starting && !was_starting {
            starter_since = Instant::now();
        }
        was_starting = starting;
        let starting = starting && pipe.sim.rpm() < 500.0 && starter_since.elapsed() < Duration::from_secs(4);
        if !starting {
            shared.starter.store(false, Ordering::Relaxed);
        }

        let request = shared.gear_request.load(Ordering::Relaxed);
        if request != last_request {
            last_request = request;
            driver.request(request.max(0) as usize);
        }

        let chunk = (cfg.sim_rate / 500.0).max(1.0) as usize;
        let dt = chunk as f64 / cfg.sim_rate;
        let engagement = driver.update(dt, shared.clutch_pedal.get() as f64, pipe.sim.speed_kmh() / 3.6);
        let ctl = Controls {
            throttle: driver.shape_throttle(shared.throttle.get() as f64),
            ignition: shared.ignition.load(Ordering::Relaxed),
            starter: starting,
            gear_ratio: driver.gear_ratio(),
            clutch: engagement,
            brake: shared.brake.get() as f64,
        };
        pipe.ambience.mic = crate::ambience::Mic::from_index(shared.mic.load(Ordering::Relaxed));
        pipe.sim.set_idle_rpm(shared.idle_rpm.get().max(100.0) as f64);
        pipe.synth.params.volume = shared.volume.get();
        pipe.synth.params.convolution = shared.convolution.get();

        out.clear();
        pipe.run(&ctl, chunk, &mut out);

        if let Ok(mut scope) = shared.scope.try_lock() {
            for &s in &out {
                scope_counter += 1;
                if scope_counter % SCOPE_DECIMATION == 0 {
                    if scope.len() >= SCOPE_POINTS {
                        scope.remove(0);
                    }
                    scope.push(s as f32 / 32768.0);
                }
            }
        }

        let mut written = 0;
        while written < out.len() && !shared.quit.load(Ordering::Relaxed) {
            written += producer.push_slice(&out[written..]);
            if written < out.len() {
                std::thread::sleep(Duration::from_micros(500));
            }
        }

        shared.rpm.set(pipe.sim.rpm() as f32);
        shared.speed_kmh.set(pipe.sim.speed_kmh() as f32);
        shared.gear.store(driver.gear() as i32, Ordering::Relaxed);
        shared.clutch_engagement.set(engagement as f32);
        shared.throttle_effective.set(pipe.sim.throttle() as f32);
        shared.leveler_gain.set(pipe.synth.leveler_gain());
        shared.torque.set(pipe.sim.torque_nm() as f32);
        shared.power.set(pipe.sim.power_kw() as f32);
        shared.firing.store(pipe.sim.is_firing(), Ordering::Relaxed);
        if let Ok(mut pv) = shared.pv.try_lock() {
            pv.clear();
            pv.extend_from_slice(pipe.sim.pv_loop());
        }
    }
}
