//! Engines described in TOML, loaded at startup (and on demand with `r` in the TUI).
//!
//! A file describes one engine: `base = "some_engine_id"` starts from an existing engine and
//! only redefines what changes; without `base`, the defaults of `presets::base()` apply.
//! `--dump-toml ID` prints the full description of an existing engine.
//!
//! To describe the cylinders more easily, the `[firing]` table generates the firing order:
//! `order = [1, 5, 3, ...]`, `spacing` (degrees between firings, 720/n by default),
//! `exhaust_map = "single" | "halves" | "alternate"`, `bank_map` (same values, for the views;
//! defaults to `exhaust_map`, with `bank_angle_deg` at the engine level),
//! `primary = [...]` (m, one per cylinder) or `primary_base` + `primary_step` + `primary_cycle`.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use toml::{Table, Value};

use crate::engine::{CylinderCfg, EngineCfg};
use crate::presets;

static EXTRA_DIRS: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();

/// Adds a directory (or file) to scan in addition to the default ones.
pub fn add_dir(path: PathBuf) {
    EXTRA_DIRS.get_or_init(Default::default).lock().unwrap().push(path);
}

fn search_paths() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from("engines")];
    if let Some(home) = std::env::var_os("HOME") {
        v.push(PathBuf::from(home).join(".config/engine-sim-rs/engines"));
    }
    if let Some(extra) = EXTRA_DIRS.get() {
        v.extend(extra.lock().unwrap().iter().cloned());
    }
    v
}

/// Built-in engines, then the engines from TOML files; a file with the same id
/// replaces the existing engine. Also returns the errors met.
pub fn load_all() -> (Vec<EngineCfg>, Vec<String>) {
    let mut engines = presets::all();
    let mut errors = Vec::new();
    for dir in search_paths() {
        let mut files: Vec<PathBuf> = if dir.is_file() {
            vec![dir]
        } else {
            match std::fs::read_dir(&dir) {
                Ok(rd) => rd
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().is_some_and(|x| x == "toml"))
                    .collect(),
                Err(_) => continue,
            }
        };
        files.sort();
        for path in files {
            match load_file(&path, &engines) {
                Ok(cfg) => match engines.iter().position(|e| e.id() == cfg.id()) {
                    Some(i) => engines[i] = cfg,
                    None => engines.push(cfg),
                },
                Err(e) => errors.push(format!("{} : {e}", path.display())),
            }
        }
    }
    (engines, errors)
}

fn load_file(path: &Path, known: &[EngineCfg]) -> Result<EngineCfg, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    parse(&text, known)
}

pub fn parse(text: &str, known: &[EngineCfg]) -> Result<EngineCfg, String> {
    let mut user: Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;

    let base_cfg = match user.remove("base") {
        Some(Value::String(id)) => known
            .iter()
            .find(|e| e.id() == id)
            .ok_or_else(|| format!("unknown base: {id}"))?
            .clone(),
        Some(_) => return Err("`base` must be a string (engine id)".into()),
        None => presets::base(),
    };
    let mut merged = Value::try_from(&base_cfg).map_err(|e| e.to_string())?;

    if let Some(firing) = user.remove("firing") {
        let cylinders = build_firing(&firing)?;
        user.insert("cylinders".into(), Value::try_from(&cylinders).map_err(|e| e.to_string())?);
    }
    for key in ["turbo", "electric"] {
        if matches!(user.get(key), Some(Value::String(s)) if s == "none") {
            user.remove(key);
            if let Value::Table(t) = &mut merged {
                t.remove(key);
            }
        }
    }
    if !user.contains_key("name") {
        if let Some(Value::String(name)) = merged.get("name") {
            user.insert("name".into(), Value::String(format!("{name} (custom)")));
        }
    }

    merge(&mut merged, Value::Table(user));
    let cfg: EngineCfg = merged.try_into().map_err(|e: toml::de::Error| e.to_string())?;
    validate(&cfg)?;
    Ok(cfg)
}

/// Recursive merge: tables merge key by key, anything else is replaced.
fn merge(base: &mut Value, over: Value) {
    match (base, over) {
        (Value::Table(b), Value::Table(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(slot) => merge(slot, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (slot, v) => *slot = v,
    }
}

fn build_firing(v: &Value) -> Result<Vec<CylinderCfg>, String> {
    let t = v.as_table().ok_or("`firing` must be a table")?;
    let num = |key: &str| t.get(key).and_then(|x| x.as_float().or_else(|| x.as_integer().map(|i| i as f64)));
    let order: Vec<usize> = t
        .get("order")
        .and_then(|o| o.as_array())
        .ok_or("firing.order is missing (list of cylinders in firing order)")?
        .iter()
        .map(|x| x.as_integer().map(|i| i as usize).ok_or("firing.order: integers expected"))
        .collect::<Result<_, _>>()?;
    let n = order.len();
    let mut seen = order.clone();
    seen.sort_unstable();
    if n == 0 || seen != (1..=n).collect::<Vec<_>>() {
        return Err(format!("firing.order must contain each cylinder from 1 to {n} exactly once"));
    }
    let spacing = num("spacing").unwrap_or(720.0 / n as f64);
    let exhaust_map = t.get("exhaust_map").and_then(|x| x.as_str()).unwrap_or("single").to_string();
    let primary: Option<Vec<f64>> = t.get("primary").and_then(|p| p.as_array()).map(|a| {
        a.iter().map(|x| x.as_float().or_else(|| x.as_integer().map(|i| i as f64)).unwrap_or(0.3)).collect()
    });
    let (base, step) = (num("primary_base").unwrap_or(0.3), num("primary_step").unwrap_or(0.0));
    let cycle = num("primary_cycle").map(|c| c as usize).unwrap_or(n).max(1);
    let split = |map: &str, c: usize| match map {
        "halves" => usize::from(c > n / 2),
        "alternate" => usize::from(c % 2 == 0),
        _ => 0,
    };
    let exhaust = |c: usize| split(exhaust_map.as_str(), c);
    let bank_map = t.get("bank_map").and_then(|x| x.as_str()).unwrap_or(&exhaust_map).to_string();
    let mut cylinders = presets::firing(
        &order,
        spacing,
        exhaust,
        |c| match &primary {
            Some(p) => p.get(c - 1).copied().unwrap_or(base),
            None => base + step * ((c - 1) % cycle) as f64,
        },
    );
    for (i, c) in cylinders.iter_mut().enumerate() {
        c.bank = split(&bank_map, i + 1);
    }
    Ok(cylinders)
}

fn validate(c: &EngineCfg) -> Result<(), String> {
    if c.cylinders.is_empty() {
        return Err("no cylinders (use `[firing]` or `[[cylinders]]`)".into());
    }
    if c.exhausts.is_empty() {
        return Err("no exhaust (`[[exhausts]]`)".into());
    }
    if let Some(bad) = c.cylinders.iter().find(|y| y.exhaust >= c.exhausts.len()) {
        return Err(format!("a cylinder uses exhaust {} but there are only {}", bad.exhaust, c.exhausts.len()));
    }
    if c.gearbox.ratios.is_empty() {
        return Err("gearbox.ratios is empty".into());
    }
    if c.bore <= 0.0 || c.stroke <= 0.0 || c.rod_length <= c.stroke / 2.0 || c.compression_ratio <= 1.0 {
        return Err("invalid geometry (bore, stroke, rod_length, compression_ratio)".into());
    }
    if c.idle_rpm <= 0.0 || c.redline_rpm <= c.idle_rpm {
        return Err("need 0 < idle_rpm < redline_rpm".into());
    }
    if c.sim_rate < 2_000.0 || c.inertia <= 0.0 {
        return Err("invalid sim_rate (>= 2000) or inertia".into());
    }
    Ok(())
}

/// Full TOML description of an engine.
pub fn dump(cfg: &EngineCfg) -> String {
    toml::to_string_pretty(cfg).unwrap_or_else(|e| format!("# error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all_presets() {
        let all = presets::all();
        for e in &all {
            let text = format!("base = \"{}\"\nname = \"{}\"\n", e.id(), e.name);
            let back = parse(&text, &all).unwrap();
            assert_eq!(back.cylinders.len(), e.cylinders.len(), "{}", e.name);
            assert_eq!(back.redline_rpm, e.redline_rpm);
            // The full dump can be read back without a base.
            let again = parse(&dump(e), &all).unwrap();
            assert_eq!(again.gearbox.ratios, e.gearbox.ratios);
            assert_eq!(again.turbo.is_some(), e.turbo.is_some());
        }
    }

    #[test]
    fn override_and_firing() {
        let all = presets::all();
        let cfg = parse(
            "base = \"gm_ls_v8\"\nname = \"LS test\"\nredline_rpm = 7200\n[gearbox]\nfinal_drive = 3.9\n\
             [firing]\norder = [1, 8, 4, 3, 6, 5, 7, 2]\nexhaust_map = \"halves\"\nprimary_base = 0.3\n",
            &all,
        )
        .unwrap();
        assert_eq!(cfg.redline_rpm, 7200.0);
        assert_eq!(cfg.gearbox.final_drive, 3.9);
        assert_eq!(cfg.gearbox.ratios.len(), 6);
        assert_eq!(cfg.cylinders[7].exhaust, 1);
        assert!(parse("base = \"gm_ls_v8\"\nredlin = 1\n", &all).is_err());
    }
}
