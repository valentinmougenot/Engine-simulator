use std::io::Cursor;

use crate::ambience::{Ambience, Ambient};
use crate::engine::{Controls, EngineCfg, EngineSim};
use crate::synth::{prepare_impulse_response, AudioParams, Synthesizer};

/// Full chain: engine simulation -> synthesizer.
pub struct Pipeline {
    pub sim: EngineSim,
    pub synth: Synthesizer,
    flows: Vec<f64>,
    audio_rate: f64,
    whine_phase: f64,
    whine_level: f64,
    pub ambience: Ambience,
}

impl Pipeline {
    pub fn new(cfg: &EngineCfg, audio_rate: f64) -> Self {
        let irs = cfg
            .exhausts
            .iter()
            .map(|e| {
                let mut reader = hound::WavReader::new(Cursor::new(e.ir.bytes())).expect("IR invalide");
                let rate = reader.spec().sample_rate;
                let samples: Vec<i16> = reader.samples::<i16>().map(|s| s.unwrap()).collect();
                prepare_impulse_response(&samples, rate, audio_rate, e.ir.volume())
            })
            .collect();

        let params = AudioParams {
            df_f_mix: cfg.hf_gain,
            air_noise: cfg.noise,
            input_sample_noise: cfg.jitter,
            input_cutoff: cfg.input_cutoff,
            air_noise_cutoff: cfg.air_noise_cutoff,
            ..AudioParams::default()
        };

        Self {
            sim: EngineSim::new(cfg),
            synth: Synthesizer::new(audio_rate, cfg.sim_rate, params, irs, &cfg.eq),
            flows: vec![0.0; cfg.exhausts.len()],
            audio_rate,
            whine_phase: 0.0,
            whine_level: cfg.turbo.as_ref().map_or(0.0, |t| t.whine),
            ambience: Ambience::new(cfg, audio_rate),
        }
    }

    /// Runs `n` input steps and appends the audio to `out`.
    pub fn run(&mut self, ctl: &Controls, n: usize, out: &mut Vec<i16>) {
        let start = out.len();
        for _ in 0..n {
            self.sim.step(ctl, &mut self.flows);
            self.synth.write_input(&self.flows, out);
        }
        let state = Ambient {
            rpm: self.sim.rpm(),
            speed_kmh: self.sim.speed_kmh(),
            throttle: self.sim.throttle(),
            user_throttle: ctl.throttle,
            gear_ratio: ctl.gear_ratio,
            clutch: ctl.clutch,
            spool: self.sim.turbo_spool(),
        };
        self.ambience.process(&mut out[start..], &state);
        self.add_turbo_whine(&mut out[start..]);
    }

    /// Turbine whine added after the leveler: its pitch and level follow
    /// the turbine speed (real turbines spin above 100,000 rpm).
    fn add_turbo_whine(&mut self, samples: &mut [i16]) {
        if self.whine_level <= 0.0 {
            return;
        }
        let spool = self.sim.turbo_spool();
        let freq = 1800.0 + 5200.0 * spool;
        let amplitude = self.whine_level * self.ambience.mic.whine_scale() * 1800.0 * spool * spool;
        let step = 2.0 * std::f64::consts::PI * freq / self.audio_rate;
        for s in samples {
            self.whine_phase = (self.whine_phase + step) % (2.0 * std::f64::consts::PI);
            // Two harmonics for a less pure tone than a sine.
            let tone = self.whine_phase.sin() + 0.35 * (2.0 * self.whine_phase).sin();
            // The main signal is attenuated slightly to leave headroom for the whine.
            *s = (*s as f64 * 0.92 + amplitude * tone).clamp(-32768.0, 32767.0) as i16;
        }
    }
}
