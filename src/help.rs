//! Keyboard and mouse shortcut lists, shown in the help window (`h` / `?`) of the TUI and GUI.

/// (keys, action). Key names are spelled out: arrows are missing from some fonts.
pub const SHORTCUTS: &[(&str, &str)] = &[
    ("Up / Down", "Throttle +5 % / -5 %"),
    ("0 to 9", "Throttle 0 to 90 %"),
    ("f / x", "Full throttle / throttle off"),
    ("a or Right", "Shift up"),
    ("z or Left", "Shift down"),
    ("n", "Neutral"),
    ("c", "Clutch (toggle)"),
    ("Space", "Brake (TUI: toggle; GUI: hold)"),
    ("s", "Starter"),
    ("i", "Ignition on/off"),
    ("[ / ]", "Volume - / +"),
    ("- / =", "Convolution - / +"),
    (", / .", "Idle speed - / + (50 rpm)"),
    ("t", "Animation speed (auto, real time, slow motion)"),
    ("e / E", "Next / previous view (scope, cylinders, top, front)"),
    ("v", "Microphone position"),
    ("m / Tab", "Engine list (TUI: j k move, / filter, < > sort, r reverse; GUI: M opens the picker)"),
    ("r", "Reload the TOML files"),
    ("h / ? / F1", "Help (TUI: j k to scroll, / to filter)"),
    ("q", "Quit"),
];

/// Mouse (TUI only).
pub const MOUSE: &[(&str, &str)] = &[
    ("Wheel", "Throttle (over Gear: shift; over Audio: volume, convolution, idle)"),
    ("Left / right click", "Gear + / - on the Gear panel; next / previous view on the right; click a tab to pick a view"),
    ("Middle click", "Neutral (on the Gear panel)"),
    ("Gauges", "Throttle: click or drag; clutch: click; brake: hold the button"),
    ("Engine name, bottom bar", "Click to open the engine list; the bar keys are clickable"),
    ("Engine list", "Wheel to scroll, click an engine to select it"),
];
