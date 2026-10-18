//! DSP building blocks, ported faithfully from the original C++ engine
//! (4th-order Butterworth, RC low-pass, derivative, jitter, convolution, leveler).

use std::f64::consts::PI;

/// xorshift64* PRNG, good enough for audio noise.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    #[inline]
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in [0, 1).
    #[inline]
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform in [-1, 1).
    #[inline]
    pub fn signed(&mut self) -> f32 {
        2.0 * self.unit() - 1.0
    }
}

/// 4th-order Butterworth low-pass (direct form, coefficients from the original).
#[derive(Clone)]
pub struct Butterworth {
    a: [f64; 5],
    f4: f64,
    x: [f64; 4],
    y: [f64; 4],
}

impl Butterworth {
    pub fn new(cutoff: f64, sample_rate: f64) -> Self {
        let mut f = Self { a: [1.0; 5], f4: 0.0, x: [0.0; 4], y: [0.0; 4] };
        f.set_cutoff(cutoff, sample_rate);
        f
    }

    pub fn set_cutoff(&mut self, fc: f64, sample_rate: f64) {
        let f = (PI * fc / sample_rate).tan();
        let f2 = f * f;
        let f3 = f2 * f;
        let f4 = f2 * f2;
        let m = -2.0 * (5.0 * PI / 8.0).cos();
        let n = -2.0 * (7.0 * PI / 8.0).cos();

        let a0 = 1.0 + (m + n) * f + (2.0 + n * m) * f2 + (m + n) * f3 + f4;
        self.a[0] = a0;
        self.a[1] = (-4.0 - 2.0 * (n + m) * f + 2.0 * (m + n) * f3 + 4.0 * f4) / a0;
        self.a[2] = (6.0 - 2.0 * (2.0 + m * n) * f2 + 6.0 * f4) / a0;
        self.a[3] = (-4.0 + 2.0 * (m + n) * f - 2.0 * (m + n) * f3 + 4.0 * f4) / a0;
        self.a[4] = (1.0 - (n + m) * f + (2.0 + m * n) * f2 - (m + n) * f3 + f4) / a0;
        self.f4 = f4;
    }

    #[inline]
    pub fn process(&mut self, s: f64) -> f64 {
        let n = self.f4 / self.a[0]
            * (s + 4.0 * self.x[0] + 6.0 * self.x[1] + 4.0 * self.x[2] + self.x[3]);
        let d = -self.a[1] * self.y[0]
            - self.a[2] * self.y[1]
            - self.a[3] * self.y[2]
            - self.a[4] * self.y[3];
        let y = n + d;

        self.x = [s, self.x[0], self.x[1], self.x[2]];
        self.y = [y, self.y[0], self.y[1], self.y[2]];
        y
    }
}

/// First-order RC low-pass.
pub struct LowPass {
    dt: f32,
    rc: f32,
    y: f32,
}

impl LowPass {
    pub fn new(cutoff: f32, dt: f32) -> Self {
        Self { dt, rc: 1.0 / (cutoff * 2.0 * PI as f32), y: 0.0 }
    }

    #[inline]
    pub fn process(&mut self, s: f32) -> f32 {
        let alpha = self.dt / (self.rc + self.dt);
        self.y = alpha * s + (1.0 - alpha) * self.y;
        self.y
    }
}

/// Discrete derivative (x[n] - x[n-1]) / dt.
pub struct Derivative {
    dt: f32,
    prev: f32,
}

impl Derivative {
    pub fn new(dt: f32) -> Self {
        Self { dt, prev: 0.0 }
    }

    #[inline]
    pub fn process(&mut self, s: f32) -> f32 {
        let d = (s - self.prev) / self.dt;
        self.prev = s;
        d
    }
}

/// Random fractional delay: the sampling instant wobbles slightly,
/// which gives the engine its "organic" feel.
pub struct Jitter {
    history: Vec<f32>,
    offset: usize,
    noise: Butterworth,
    scale: f32,
}

impl Jitter {
    const MAX: usize = 10;

    pub fn new(noise_cutoff: f64, sample_rate: f64) -> Self {
        Self {
            history: vec![0.0; Self::MAX],
            offset: 0,
            noise: Butterworth::new(noise_cutoff, sample_rate),
            scale: 0.0,
        }
    }

    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale;
    }

    #[inline]
    pub fn process(&mut self, sample: f32, rng: &mut Rng) -> f32 {
        const MAXF: f32 = (Jitter::MAX - 1) as f32;
        self.history[self.offset] = sample;
        self.offset += 1;
        if self.offset >= Self::MAX {
            self.offset = 0;
        }

        let r = rng.unit() * MAXF;
        let s = self.noise.process((r * self.scale) as f64) as f32;
        let s0 = s.floor().clamp(0.0, MAXF);
        let s1 = s.ceil().clamp(0.0, MAXF);
        let frac = s - s0;

        let wrap = |i: usize| if i >= Self::MAX { i - Self::MAX } else { i };
        let v0 = self.history[wrap(s0 as usize + self.offset)];
        let v1 = self.history[wrap(s1 as usize + self.offset)];
        v1 * frac + v0 * (1.0 - frac)
    }
}

/// Direct convolution with the exhaust impulse response.
/// The shift register is doubled so the dot product is contiguous
/// (and therefore vectorisable).
pub struct Convolution {
    ir: Vec<f32>,
    hist: Vec<f32>,
    offset: usize,
}

impl Convolution {
    pub fn new(ir: Vec<f32>) -> Self {
        let n = ir.len().max(1);
        let mut ir = ir;
        ir.resize(n, 0.0);
        Self { hist: vec![0.0; 2 * n], offset: 0, ir }
    }

    #[inline]
    pub fn process(&mut self, sample: f32) -> f32 {
        let n = self.ir.len();
        self.hist[self.offset] = sample;
        self.hist[self.offset + n] = sample;

        let window = &self.hist[self.offset..self.offset + n];
        let mut acc = [0.0f32; 16];
        let mut ir_chunks = self.ir.chunks_exact(16);
        let mut w_chunks = window.chunks_exact(16);
        for (a, b) in (&mut ir_chunks).zip(&mut w_chunks) {
            for k in 0..16 {
                acc[k] += a[k] * b[k];
            }
        }
        let mut result: f32 = acc.iter().sum();
        for (a, b) in ir_chunks.remainder().iter().zip(w_chunks.remainder()) {
            result += a * b;
        }

        self.offset = (self.offset + n - 1) % n;
        result
    }
}

/// Automatic gain control: brings the peak towards `target`.
pub struct Leveler {
    peak: f32,
    attenuation: f32,
    pub target: f32,
    pub min_level: f32,
    pub max_level: f32,
}

impl Leveler {
    pub fn new(target: f32, min_level: f32, max_level: f32) -> Self {
        Self { peak: 30000.0, attenuation: 1.0, target, min_level, max_level }
    }

    pub fn attenuation(&self) -> f32 {
        self.attenuation
    }

    #[inline]
    pub fn process(&mut self, s: f32) -> f32 {
        self.peak *= 0.999;
        if s.abs() > self.peak {
            self.peak = s.abs();
        }
        if self.peak == 0.0 {
            return 0.0;
        }
        let a = (self.target / self.peak).clamp(self.min_level, self.max_level);
        self.attenuation = 0.9 * self.attenuation + 0.1 * a;
        s * self.attenuation
    }
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EqKind {
    LowShelf,
    HighShelf,
    Peak,
}

/// EQ band: shape, frequency (Hz), gain (dB).
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EqBand {
    pub kind: EqKind,
    pub freq: f64,
    pub gain_db: f64,
}

/// Biquad (direct form I) from the "Audio EQ Cookbook" formulas.
#[derive(Clone)]
pub struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
}

impl Biquad {
    pub fn new(band: EqBand, sample_rate: f64) -> Self {
        let a = 10f64.powf(band.gain_db / 40.0);
        let w0 = 2.0 * PI * band.freq / sample_rate;
        let (sin, cos) = w0.sin_cos();
        let (b0, b1, b2, a0, a1, a2) = match band.kind {
            EqKind::Peak => {
                let alpha = sin / (2.0 * 0.9);
                (
                    1.0 + alpha * a,
                    -2.0 * cos,
                    1.0 - alpha * a,
                    1.0 + alpha / a,
                    -2.0 * cos,
                    1.0 - alpha / a,
                )
            }
            EqKind::LowShelf | EqKind::HighShelf => {
                let alpha = sin / 2.0 * 2f64.sqrt();
                let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
                if matches!(band.kind, EqKind::LowShelf) {
                    (
                        a * ((a + 1.0) - (a - 1.0) * cos + two_sqrt_a_alpha),
                        2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                        a * ((a + 1.0) - (a - 1.0) * cos - two_sqrt_a_alpha),
                        (a + 1.0) + (a - 1.0) * cos + two_sqrt_a_alpha,
                        -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                        (a + 1.0) + (a - 1.0) * cos - two_sqrt_a_alpha,
                    )
                } else {
                    (
                        a * ((a + 1.0) + (a - 1.0) * cos + two_sqrt_a_alpha),
                        -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                        a * ((a + 1.0) + (a - 1.0) * cos - two_sqrt_a_alpha),
                        (a + 1.0) - (a - 1.0) * cos + two_sqrt_a_alpha,
                        2.0 * ((a - 1.0) - (a + 1.0) * cos),
                        (a + 1.0) - (a - 1.0) * cos - two_sqrt_a_alpha,
                    )
                }
            }
        };
        Self {
            b: [b0 / a0, b1 / a0, b2 / a0],
            a: [a1 / a0, a2 / a0],
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }

    #[inline]
    pub fn process(&mut self, s: f64) -> f64 {
        let y = self.b[0] * s + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [s, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }
}
