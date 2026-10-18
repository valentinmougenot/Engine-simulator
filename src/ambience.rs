//! Ambient sounds added after the synthesizer: intake, turbo blow-off, transmission,
//! tyres and wind, plus the microphone position (exhaust, cabin, engine bay).

use std::f64::consts::PI;

use crate::dsp::Rng;
use crate::engine::EngineCfg;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mic {
    Exhaust,
    Cabin,
    EngineBay,
}

impl Mic {
    pub const ALL: [Mic; 3] = [Mic::Exhaust, Mic::Cabin, Mic::EngineBay];

    pub fn from_index(i: usize) -> Mic {
        Self::ALL[i % Self::ALL.len()]
    }

    pub fn name(self) -> &'static str {
        match self {
            Mic::Exhaust => "Exhaust",
            Mic::Cabin => "Cabin",
            Mic::EngineBay => "Engine bay",
        }
    }

    /// Relative levels: intake, tyres, wind, transmission, blow-off, turbo whine.
    fn levels(self) -> [f64; 6] {
        match self {
            Mic::Exhaust => [0.25, 0.15, 0.10, 0.0, 0.5, 1.0],
            Mic::Cabin => [0.5, 1.0, 1.0, 1.0, 0.8, 0.7],
            Mic::EngineBay => [1.2, 0.3, 0.2, 0.5, 1.2, 1.8],
        }
    }

    pub fn whine_scale(self) -> f64 {
        self.levels()[5]
    }
}

/// Simulation values the sounds depend on.
pub struct Ambient {
    pub rpm: f64,
    pub speed_kmh: f64,
    pub throttle: f64,
    pub user_throttle: f64,
    pub gear_ratio: f64,
    pub clutch: f64,
    pub spool: f64,
}

#[derive(Clone, Copy, Default)]
struct OnePole {
    a: f64,
    y: f64,
}

impl OnePole {
    fn new(cutoff: f64, rate: f64) -> Self {
        Self { a: 1.0 - (-2.0 * PI * cutoff / rate).exp(), y: 0.0 }
    }

    #[inline]
    fn lp(&mut self, x: f64) -> f64 {
        self.y += self.a * (x - self.y);
        self.y
    }

    #[inline]
    fn hp(&mut self, x: f64) -> f64 {
        x - self.lp(x)
    }
}

pub struct Ambience {
    pub mic: Mic,
    rate: f64,
    rng: Rng,
    cylinders: f64,
    redline: f64,
    blow_off: bool,
    main_lp: [OnePole; 2],
    main_split: OnePole,
    intake: [OnePole; 2],
    road: [OnePole; 2],
    wind: [OnePole; 2],
    blow: [OnePole; 2],
    intake_phase: f64,
    trans_phase: f64,
    blow_env: f64,
    prev_throttle: f64,
}

impl Ambience {
    pub fn new(cfg: &EngineCfg, rate: f64) -> Self {
        Self {
            mic: Mic::Exhaust,
            rate,
            rng: Rng::new(0x9E37_79B9_7F4A_7C15),
            cylinders: cfg.cylinders.len() as f64,
            redline: cfg.redline_rpm,
            blow_off: cfg.turbo.as_ref().is_some_and(|t| t.lag >= 0.1) && !cfg.anti_lag,
            main_lp: [OnePole::new(1500.0, rate); 2],
            main_split: OnePole::new(1500.0, rate),
            intake: [OnePole::new(1400.0, rate), OnePole::new(250.0, rate)],
            road: [OnePole::new(700.0, rate), OnePole::new(80.0, rate)],
            wind: [OnePole::new(3000.0, rate), OnePole::new(300.0, rate)],
            blow: [OnePole::new(6000.0, rate), OnePole::new(1500.0, rate)],
            intake_phase: 0.0,
            trans_phase: 0.0,
            blow_env: 0.0,
            prev_throttle: 0.0,
        }
    }

    pub fn process(&mut self, samples: &mut [i16], st: &Ambient) {
        let [intake_l, road_l, wind_l, trans_l, blow_l, _] = self.mic.levels();
        let n = samples.len().max(1) as f64;
        let chunk_s = n / self.rate;

        // Blow-off: throttle slammed shut while the turbo is boosting.
        if self.blow_off && st.user_throttle < 0.05 && self.prev_throttle > 0.4 && st.spool > 0.25 {
            self.blow_env = st.spool;
        }
        self.prev_throttle = st.user_throttle;
        let blow_end = self.blow_env * (-chunk_s / 0.28).exp();

        let rev = (st.rpm / self.redline).clamp(0.0, 1.2);
        let intake_amp = 2500.0 * intake_l * st.throttle.clamp(0.0, 1.0).powf(1.5) * rev.powf(1.2);
        let v = st.speed_kmh.abs();
        let road_amp = 1100.0 * road_l * (v / 100.0).min(2.0).powf(1.2);
        let wind_amp = 2200.0 * wind_l * (v / 200.0).min(1.5).powi(2);
        let loaded = if st.gear_ratio > 0.0 { st.clutch.clamp(0.0, 1.0) } else { 0.0 };
        let trans_amp = 450.0 * trans_l * loaded * (0.4 + 0.6 * st.throttle.clamp(0.0, 1.0));
        let intake_step = 2.0 * PI * (st.rpm / 60.0) * self.cylinders / 2.0 / self.rate;
        let trans_step = 2.0 * PI * (st.rpm / 60.0) * 7.0 / self.rate;
        let blow_amp = 5000.0 * blow_l;
        let blow_start = self.blow_env;

        for (i, s) in samples.iter_mut().enumerate() {
            let x = *s as f64;
            let mut y = match self.mic {
                Mic::Exhaust => x,
                Mic::Cabin => {
                    let l0 = self.main_lp[0].lp(x);
                    let l = self.main_lp[1].lp(l0);
                    0.75 * l + 0.1 * x
                }
                Mic::EngineBay => 0.75 * x + 0.5 * self.main_split.hp(x),
            };

            let white = self.rng.signed() as f64;
            // Intake: broadband noise modulated at the firing frequency.
            self.intake_phase = (self.intake_phase + intake_step) % (2.0 * PI);
            let am = 1.0 + 0.5 * self.intake_phase.sin();
            let b0 = self.intake[0].lp(white);
            let band = self.intake[1].hp(b0);
            y += intake_amp * am * band * 3.0;

            let r0 = self.road[0].lp(self.rng.signed() as f64);
            let road = self.road[1].hp(r0);
            y += road_amp * road * 6.0;
            let w0 = self.wind[0].lp(self.rng.signed() as f64);
            let wind = self.wind[1].hp(w0);
            y += wind_amp * wind * 3.5;

            if trans_amp > 0.0 {
                self.trans_phase = (self.trans_phase + trans_step) % (2.0 * PI);
                y += trans_amp * (self.trans_phase.sin() + 0.4 * (2.0 * self.trans_phase).sin());
            }

            if blow_start > 0.001 {
                let env = blow_start + (blow_end - blow_start) * (i as f64 / n);
                let h0 = self.blow[0].lp(self.rng.signed() as f64);
                let hiss = self.blow[1].hp(h0);
                y += blow_amp * env * hiss * 4.0;
            }
            *s = y.clamp(-32768.0, 32767.0) as i16;
        }
        self.blow_env = blow_end;
    }
}
