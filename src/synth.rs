//! Audio synthesizer: turns the exhaust flow (at the simulation rate) into sound,
//! like `Synthesizer` in engine-sim.
//!
//! Per-exhaust-channel chain:
//!   linear resampling -> 1900 Hz low-pass -> jitter -> DC filter -> derivative
//!   -> air noise -> dF/F mix -> convolution (impulse response)
//! then channel sum -> anti-aliasing -> optional EQ -> leveler -> volume -> i16.

use crate::dsp::*;

#[derive(Clone, Copy, Debug)]
pub struct AudioParams {
    pub volume: f32,
    pub convolution: f32,
    pub df_f_mix: f32,
    pub input_sample_noise: f32,
    pub input_sample_noise_cutoff: f32,
    pub air_noise: f32,
    pub air_noise_cutoff: f32,
    /// Low-pass applied to the resampled input (1900 Hz in the original).
    pub input_cutoff: f32,
    pub leveler_target: f32,
    pub leveler_max_gain: f32,
    pub leveler_min_gain: f32,
}

impl Default for AudioParams {
    fn default() -> Self {
        Self {
            volume: 1.0,
            convolution: 1.0,
            df_f_mix: 0.01,
            input_sample_noise: 0.5,
            input_sample_noise_cutoff: 10000.0,
            air_noise: 1.0,
            air_noise_cutoff: 2000.0,
            input_cutoff: 1900.0,
            leveler_target: 30000.0,
            leveler_max_gain: 1.9,
            leveler_min_gain: 0.00001,
        }
    }
}

struct Channel {
    last_input: f32,
    input_antialias: Butterworth,
    jitter: Jitter,
    dc: LowPass,
    derivative: Derivative,
    air_noise: Butterworth,
    convolution: Convolution,
}

pub struct Synthesizer {
    pub params: AudioParams,
    audio_rate: f64,
    input_rate: f64,
    /// Fractional position of the next audio sample, in input samples.
    resample_pos: f64,
    channels: Vec<Channel>,
    antialias: Butterworth,
    eq: Vec<Biquad>,
    leveler: Leveler,
    rng: Rng,
}

impl Synthesizer {
    /// `impulse_responses`: one response per channel (already at `audio_rate`, volume applied).
    pub fn new(
        audio_rate: f64,
        input_rate: f64,
        params: AudioParams,
        impulse_responses: Vec<Vec<f32>>,
        eq: &[EqBand],
    ) -> Self {
        let dt = (1.0 / audio_rate) as f32;
        let channels = impulse_responses
            .into_iter()
            .map(|ir| Channel {
                last_input: 0.0,
                input_antialias: Butterworth::new(params.input_cutoff as f64, audio_rate),
                jitter: Jitter::new(params.input_sample_noise_cutoff as f64, audio_rate),
                dc: LowPass::new(10.0, dt),
                derivative: Derivative::new(dt),
                air_noise: Butterworth::new(params.air_noise_cutoff as f64, audio_rate),
                convolution: Convolution::new(ir),
            })
            .collect();

        Self {
            params,
            audio_rate,
            input_rate,
            resample_pos: 1.0,
            channels,
            antialias: Butterworth::new(audio_rate * 0.45, audio_rate),
            eq: eq.iter().map(|&b| Biquad::new(b, audio_rate)).collect(),
            leveler: Leveler::new(
                params.leveler_target,
                params.leveler_min_gain,
                params.leveler_max_gain,
            ),
            rng: Rng::new(0x9E37_79B9_7F4A_7C15),
        }
    }

    pub fn leveler_gain(&self) -> f32 {
        self.leveler.attenuation()
    }

    /// Consumes one input sample per channel and appends the matching audio samples
    /// (about `audio_rate / input_rate`) to `out`.
    pub fn write_input(&mut self, input: &[f64], out: &mut Vec<i16>) {
        debug_assert_eq!(input.len(), self.channels.len());
        let step = self.input_rate / self.audio_rate;

        self.leveler.target = self.params.leveler_target;
        for ch in &mut self.channels {
            ch.jitter.set_scale(self.params.input_sample_noise);
            ch.air_noise
                .set_cutoff(self.params.air_noise_cutoff as f64, self.audio_rate);
        }

        while self.resample_pos <= 1.0 {
            let f = self.resample_pos as f32;
            let mut signal = 0.0f32;
            for (ch, &x1) in self.channels.iter_mut().zip(input) {
                let x = ch.last_input * (1.0 - f) + x1 as f32 * f;
                let x = ch.input_antialias.process(x as f64) as f32;
                signal += Self::render_channel(ch, x, &self.params, &mut self.rng);
            }
            out.push(self.finish(signal));
            self.resample_pos += step;
        }
        self.resample_pos -= 1.0;

        for (ch, &x1) in self.channels.iter_mut().zip(input) {
            ch.last_input = x1 as f32;
        }
    }

    #[inline]
    fn render_channel(ch: &mut Channel, sample: f32, p: &AudioParams, rng: &mut Rng) -> f32 {
        let jittered = ch.jitter.process(sample, rng);

        let f_in = jittered;
        let f_dc = ch.dc.process(f_in);
        let f = f_in - f_dc;
        let f_p = ch.derivative.process(f_in);

        let noise = ch.air_noise.process(rng.signed() as f64) as f32;
        let r_mixed = p.air_noise * noise + (1.0 - p.air_noise);

        let mut v_in = f_p * p.df_f_mix + f * r_mixed * (1.0 - p.df_f_mix);
        if v_in.is_subnormal() {
            v_in = 0.0;
        }

        p.convolution * ch.convolution.process(v_in) + (1.0 - p.convolution) * v_in
    }

    #[inline]
    fn finish(&mut self, signal: f32) -> i16 {
        let mut shaped = self.antialias.process(signal as f64);
        for band in &mut self.eq {
            shaped = band.process(shaped);
        }
        let signal = shaped as f32;
        let v = soft_clip(self.leveler.process(signal) * self.params.volume);
        (v.round() as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16
    }
}

/// Soft clip above 20,000: the original leveler reacts with a slight lag,
/// which hard-clipped transients; here they are simply rounded.
#[inline]
fn soft_clip(v: f32) -> f32 {
    const KNEE: f32 = 20_000.0;
    let a = v.abs();
    if a <= KNEE {
        v
    } else {
        v.signum() * (KNEE + (32_767.0 - KNEE) * ((a - KNEE) / (32_767.0 - KNEE)).tanh())
    }
}

/// Prepares an impulse response like `initializeImpulseResponse`:
/// trims the silent tail (|x| <= 100), 10,000 samples max,
/// then scales by `volume`. Resampled if `rate` != 44100 Hz.
pub fn prepare_impulse_response(samples: &[i16], source_rate: u32, rate: f64, volume: f32) -> Vec<f32> {
    let clipped = samples
        .iter()
        .rposition(|s| s.unsigned_abs() > 100)
        .map_or(0, |i| i + 1);
    let n = clipped.min(10_000);
    let base: Vec<f32> = samples[..n]
        .iter()
        .map(|&s| volume * s as f32 / i16::MAX as f32)
        .collect();

    let ratio = rate / source_rate as f64;
    if (ratio - 1.0).abs() < 1e-6 {
        return base;
    }
    let out_len = (base.len() as f64 * ratio).round() as usize;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 / ratio;
            let i0 = pos.floor() as usize;
            let frac = (pos - i0 as f64) as f32;
            let a = base.get(i0).copied().unwrap_or(0.0);
            let b = base.get(i0 + 1).copied().unwrap_or(0.0);
            // Preserves energy: a response at a higher rate has more samples.
            (a * (1.0 - frac) + b * frac) / ratio as f32
        })
        .collect()
}
