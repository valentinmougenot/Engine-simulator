//! Engine views for the terminal: slow-motion animated cylinders (pistons, valves, spark plug,
//! combustion) and the P-V diagram of cylinder 1 from the simulation.

use std::f64::consts::PI;

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Circle, Line as CLine, Points};
use ratatui::widgets::{Axis, Block, Chart, Dataset, GraphType, Paragraph};
use ratatui::Frame;

use crate::engine::EngineCfg;

const CELL_W: usize = 7;
const GAP: usize = 1;

fn wrap_cycle(x: f64) -> f64 {
    x - 4.0 * PI * (x / (4.0 * PI)).round()
}

fn mix(a: (u8, u8, u8), b: (u8, u8, u8), t: f64) -> Color {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t) as u8;
    Color::Rgb(f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

fn stroke_name(phi: f64) -> &'static str {
    let deg = phi.to_degrees();
    if deg < -180.0 {
        "INT"
    } else if deg < 0.0 {
        "CMP"
    } else if deg < 180.0 {
        "PWR"
    } else {
        "EXH"
    }
}

/// One cell = one cylinder, `chamber + 3` lines of `CELL_W` characters.
fn cell(cfg: &EngineCfg, number: usize, phi: f64, firing: bool, load: f64, chamber: usize) -> Vec<Line<'static>> {
    let frac = cfg.piston_fraction(phi);
    let (in_open, ex_open) = cfg.valve_openings(phi);
    let deg = phi.to_degrees();
    let stroke = stroke_name(phi);
    let wall = Style::new().fg(Color::Gray);

    let spark = firing && load > 0.02 && (-30.0..8.0).contains(&deg);
    let burning = firing && load > 0.02 && (0.0..150.0).contains(&deg);

    // Gas colour and pattern by cycle phase.
    let (gas_char, gas_color) = match stroke {
        "INT" => ('░', Color::Rgb(60, 110, 220)),
        "CMP" => ('▒', mix((60, 110, 220), (240, 170, 40), 1.0 - frac)),
        "PWR" if burning => ('█', mix((255, 240, 120), (150, 20, 10), deg / 150.0)),
        "PWR" => ('▒', mix((240, 170, 40), (80, 80, 90), deg / 180.0)),
        _ => ('░', Color::Rgb(110, 110, 120)),
    };

    let mut lines = Vec::with_capacity(chamber + 3);
    let valve = |open: f64, hot: Color| {
        if open > 0.05 {
            Span::styled("▼", Style::new().fg(hot).bold())
        } else {
            Span::styled("▽", Style::new().fg(Color::DarkGray))
        }
    };
    lines.push(Line::from(vec![
        Span::raw(" "),
        valve(in_open, Color::Green),
        Span::raw(" "),
        if spark { Span::styled("╥", Style::new().fg(Color::Yellow).bold()) } else { Span::styled("╥", Style::new().fg(Color::DarkGray)) },
        Span::raw(" "),
        valve(ex_open, Color::Red),
        Span::raw(" "),
    ]));

    let piston_row = (frac * (chamber as f64 - 1.0)).round() as usize;
    for r in 0..chamber {
        let inner = if r < piston_row {
            Span::styled(gas_char.to_string().repeat(CELL_W - 2), Style::new().fg(gas_color))
        } else if r == piston_row {
            Span::styled("▀".repeat(CELL_W - 2), Style::new().fg(Color::White).bold())
        } else {
            Span::styled("  ┃  ".to_string(), Style::new().fg(Color::DarkGray))
        };
        lines.push(Line::from(vec![Span::styled("│", wall), inner, Span::styled("│", wall)]));
    }

    // Crank pin: turns with the crankshaft.
    let pin = (3.0 + 2.0 * phi.sin()).round().clamp(1.0, 5.0) as usize;
    let crank: String = (0..CELL_W).map(|i| if i == pin { '●' } else if i == 3 { '·' } else { ' ' }).collect();
    lines.push(Line::from(Span::styled(crank, Style::new().fg(Color::Cyan))));

    let label_style = if stroke == "PWR" { Style::new().fg(Color::Yellow).bold() } else { Style::new().dim() };
    lines.push(Line::from(Span::styled(format!("{:^w$}", format!("{number} {stroke}"), w = CELL_W), label_style)));
    lines
}

/// Draws all cylinders, grouped by bank, at the visual angle `theta`.
pub fn draw_cylinders(f: &mut Frame, area: Rect, cfg: &EngineCfg, theta: f64, speed: &str, firing: bool, load: f64) {
    let title = format!(
        " Engine — {} · crankshaft {:>3.0}° ",
        speed,
        (theta.to_degrees() % 360.0 + 360.0) % 360.0
    );
    let block = Block::bordered().title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width < CELL_W as u16 || inner.height < 5 {
        return;
    }

    // Display order: by bank then by number, with a gap between banks.
    let n = cfg.cylinders.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| (cfg.cylinders[i].bank, i));
    let per_row = ((inner.width as usize + GAP) / (CELL_W + GAP)).max(1);
    let rows: Vec<&[usize]> = order.chunks(per_row).collect();
    // Balances the rows (avoids 8 + 4 for 12 cylinders on 2 rows).
    let balanced = n.div_ceil(rows.len());
    let rows: Vec<&[usize]> = order.chunks(balanced).collect();

    let avail = inner.height as usize / rows.len();
    let chamber = avail.saturating_sub(3).clamp(2, 8);
    let mut text: Vec<Line> = Vec::new();
    for row in rows {
        let cells: Vec<Vec<Line>> = row
            .iter()
            .map(|&i| {
                let phi = wrap_cycle(theta - cfg.cylinders[i].fire_deg.to_radians());
                cell(cfg, i + 1, phi, firing, load, chamber)
            })
            .collect();
        for l in 0..chamber + 3 {
            let mut spans: Vec<Span> = Vec::new();
            for (k, c) in cells.iter().enumerate() {
                if k > 0 {
                    let bank_break = cfg.cylinders[row[k]].bank != cfg.cylinders[row[k - 1]].bank;
                    spans.push(Span::raw(" ".repeat(if bank_break { GAP + 2 } else { GAP })));
                }
                spans.extend(c[l].spans.clone());
            }
            text.push(Line::from(spans).centered());
        }
    }
    f.render_widget(Paragraph::new(text), inner);
}

/// Pressure-volume diagram of cylinder 1 (last simulated cycle).
pub fn draw_pv(f: &mut Frame, area: Rect, pv: &[[f32; 2]]) {
    let points: Vec<(f64, f64)> = pv
        .iter()
        .filter(|p| p[0] > 0.0)
        .map(|p| (p[0] as f64 * 1e6, p[1] as f64 / 1e5))
        .collect();
    let xmax = points.iter().map(|p| p.0).fold(1.0, f64::max) * 1.05;
    let ymax = points.iter().map(|p| p.1).fold(5.0, f64::max) * 1.1;
    let peak = points.iter().map(|p| p.1).fold(0.0, f64::max);
    let dataset = Dataset::default()
        .marker(Marker::Braille)
        .graph_type(GraphType::Scatter)
        .style(Style::new().fg(Color::LightRed))
        .data(&points);
    let chart = Chart::new(vec![dataset])
        .block(Block::bordered().title(format!(" P-V diagram (cyl. 1) · peak {peak:.0} bar ")))
        .x_axis(Axis::default().title("cm³").bounds([0.0, xmax]))
        .y_axis(Axis::default().title("bar").bounds([0.0, ymax]));
    f.render_widget(chart, area);
}

/// Gas colour and state (power stroke, burning) of a cylinder.
pub(crate) fn cylinder_state(cfg: &EngineCfg, i: usize, theta: f64, firing: bool, load: f64) -> (f64, &'static str, bool, Color) {
    let phi = wrap_cycle(theta - cfg.cylinders[i].fire_deg.to_radians());
    let deg = phi.to_degrees();
    let stroke = stroke_name(phi);
    let burning = stroke == "PWR" && firing && load > 0.02 && deg < 150.0;
    let color = match stroke {
        "INT" => Color::Rgb(60, 110, 220),
        "CMP" => mix((60, 110, 220), (240, 170, 40), 1.0 - cfg.piston_fraction(phi)),
        "PWR" if burning => mix((255, 240, 120), (150, 20, 10), deg / 150.0),
        "PWR" => mix((240, 170, 40), (80, 80, 90), deg / 180.0),
        _ => Color::Rgb(110, 110, 120),
    };
    (phi, stroke, burning, color)
}

/// Top view: the cylinders of each bank on either side of the crankshaft; the one on
/// its power stroke (the active piston) is highlighted, with the firing order below.
pub fn draw_top(f: &mut Frame, area: Rect, cfg: &EngineCfg, theta: f64, firing: bool, load: f64) {
    let n = cfg.cylinders.len();
    let banks = cfg.cylinders.iter().map(|c| c.bank).max().unwrap_or(0) + 1;
    let mut active: Vec<usize> = Vec::new();
    for i in 0..n {
        if cylinder_state(cfg, i, theta, firing, load).1 == "PWR" {
            active.push(i + 1);
        }
    }
    let shape = match (banks, cfg.bank_angle_deg) {
        (1, _) => format!("inline, {n} cyl."),
        (_, a) if a >= 179.0 => format!("flat, {n} cyl."),
        (_, a) => format!("V{n} at {a:.0}°"),
    };
    let block = Block::bordered().title(format!(
        " Top view — {shape} · power stroke: {} ",
        active.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" ")
    ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let per_bank: Vec<Vec<usize>> = (0..banks.min(2))
        .map(|b| (0..n).filter(|&i| cfg.cylinders[i].bank.min(1) == b).collect())
        .collect();
    let cols = per_bank.iter().map(Vec::len).max().unwrap_or(1).max(1);
    let cell_w = ((inner.width as usize).saturating_sub(2) / cols).clamp(7, 11);
    let arrows = ['↑', '↗', '→', '↘', '↓', '↙', '←', '↖'];

    let bank_lines = |bank: &[usize]| -> Vec<Line<'static>> {
        let mut rows: Vec<Vec<Span<'static>>> = vec![Vec::new(); 4];
        for &i in bank {
            let (_, stroke, burning, color) = cylinder_state(cfg, i, theta, firing, load);
            let border = if stroke == "PWR" { Style::new().fg(Color::Yellow).bold() } else { Style::new().fg(Color::Gray) };
            let inner_w = cell_w - 2;
            let fill_char = if burning { '█' } else if stroke == "PWR" { '▒' } else { '░' };
            let fill = Span::styled(fill_char.to_string().repeat(inner_w), Style::new().fg(color));
            let label = format!("{:^w$}", format!("{} {}", i + 1, stroke), w = inner_w);
            let label_style = if stroke == "PWR" { Style::new().fg(Color::Yellow).bold() } else { Style::new().dim() };
            rows[0].push(Span::styled(format!("┌{}┐ ", "─".repeat(inner_w)), border));
            rows[1].push(Span::styled("│", border));
            rows[1].push(fill.clone());
            rows[1].push(Span::styled("│ ", border));
            rows[2].push(Span::styled("│", border));
            rows[2].push(Span::styled(label, label_style));
            rows[2].push(Span::styled("│ ", border));
            rows[3].push(Span::styled(format!("└{}┘ ", "─".repeat(inner_w)), border));
        }
        rows.into_iter().map(Line::from).collect()
    };

    // Crankshaft: one arrow per cylinder shows where the crank pin is.
    let crank_line = |bank: &[usize], flip: bool| -> Line<'static> {
        let mut spans: Vec<Span<'static>> = Vec::new();
        for &i in bank {
            let phi = wrap_cycle(theta - cfg.cylinders[i].fire_deg.to_radians());
            let mut k = ((phi.rem_euclid(2.0 * PI)) / (2.0 * PI) * 8.0).round() as usize % 8;
            if flip {
                k = (k + 4) % 8;
            }
            let half = (cell_w - 1) / 2;
            spans.push(Span::styled("═".repeat(half), Style::new().fg(Color::Cyan)));
            spans.push(Span::styled(arrows[k].to_string(), Style::new().fg(Color::LightCyan).bold()));
            spans.push(Span::styled("═".repeat(cell_w + 1 - half - 1), Style::new().fg(Color::Cyan)));
        }
        Line::from(spans)
    };

    let mut text: Vec<Line> = Vec::new();
    text.push(Line::from(""));
    text.extend(bank_lines(&per_bank[0]));
    text.push(crank_line(&per_bank[0], false));
    if per_bank.len() > 1 {
        text.extend(bank_lines(&per_bank[1]));
        text.push(Line::from(""));
    }
    let mut order: Vec<(f64, usize)> = cfg.cylinders.iter().enumerate().map(|(i, c)| (c.fire_deg, i + 1)).collect();
    order.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut spans = vec![Span::raw("Firing order: ")];
    for (k, (_, c)) in order.iter().enumerate() {
        if k > 0 {
            spans.push(Span::styled(" → ", Style::new().dim()));
        }
        let style = if active.contains(c) { Style::new().black().on_yellow().bold() } else { Style::new() };
        spans.push(Span::styled(c.to_string(), style));
    }
    text.push(Line::from(spans));
    f.render_widget(Paragraph::new(text), inner);
}

const PALETTE: [Color; 8] = [
    Color::LightCyan,
    Color::LightGreen,
    Color::LightMagenta,
    Color::LightBlue,
    Color::White,
    Color::Cyan,
    Color::Green,
    Color::Magenta,
];

/// Front view (in the plane perpendicular to the crankshaft): crankshaft, rods, pistons and
/// combustion chambers of all cylinders, bank by bank.
pub fn draw_front(f: &mut Frame, area: Rect, cfg: &EngineCfg, theta: f64, firing: bool, load: f64) {
    let block = Block::bordered().title(" Front view — crankshaft, rods and pistons ");
    let inner = block.inner(area);
    let a = cfg.stroke / 2.0;
    let l = cfg.rod_length;
    let bore = cfg.bore;
    let half_h = 0.25 * bore;
    let reach = l + a + half_h + 0.05 * bore;
    // Orthonormal frame: a braille dot is 1/2 column by 1/4 row.
    let aspect = (inner.width.max(1) as f64 * 2.0) / (inner.height.max(1) as f64 * 4.0);
    let mut y_half = reach * 1.08;
    let mut x_half = y_half * aspect;
    if x_half < reach * 1.08 {
        x_half = reach * 1.08;
        y_half = x_half / aspect;
    }

    let banks = cfg.cylinders.iter().map(|c| c.bank.min(1)).max().unwrap_or(0) + 1;
    let angle = cfg.bank_angle_deg.to_radians();
    let dir = |b: usize| -> (f64, f64) {
        let t = if banks == 1 { 0.0 } else if b == 0 { -angle / 2.0 } else { angle / 2.0 };
        (t.sin(), t.cos())
    };

    let canvas = Canvas::default()
        .block(block)
        .marker(Marker::Braille)
        .x_bounds([-x_half, x_half])
        .y_bounds([-y_half, y_half])
        .paint(|ctx| {
            for b in 0..banks {
                let (ux, uy) = dir(b);
                let (px, py) = (uy, -ux);
                let d0 = l - a - half_h;
                let d1 = l + a + half_h + 0.05 * bore;
                let at = |d: f64, side: f64| (ux * d + px * side * bore / 2.0, uy * d + py * side * bore / 2.0);
                for side in [-1.0, 1.0] {
                    let (x1, y1) = at(d0, side);
                    let (x2, y2) = at(d1, side);
                    ctx.draw(&CLine { x1, y1, x2, y2, color: Color::Gray });
                }
                let (x1, y1) = at(d1, -1.0);
                let (x2, y2) = at(d1, 1.0);
                ctx.draw(&CLine { x1, y1, x2, y2, color: Color::Gray });
            }
            ctx.draw(&Circle { x: 0.0, y: 0.0, radius: a, color: Color::DarkGray });
            ctx.draw(&Circle { x: 0.0, y: 0.0, radius: a * 0.3, color: Color::Gray });

            for (i, cyl) in cfg.cylinders.iter().enumerate() {
                let (ux, uy) = dir(cyl.bank.min(1));
                let (px, py) = (uy, -ux);
                let (phi, stroke, _, gas) = cylinder_state(cfg, i, theta, firing, load);
                let ph = phi.rem_euclid(2.0 * PI);
                let color = if stroke == "PWR" { Color::Yellow } else { PALETTE[i % PALETTE.len()] };
                let (cx, cy) = (a * (ph.cos() * ux + ph.sin() * px), a * (ph.cos() * uy + ph.sin() * py));
                let xp = a * ph.cos() + (l * l - a * a * ph.sin().powi(2)).sqrt();
                let (pinx, piny) = (xp * ux, xp * uy);
                ctx.draw(&CLine { x1: 0.0, y1: 0.0, x2: cx, y2: cy, color });
                ctx.draw(&CLine { x1: cx, y1: cy, x2: pinx, y2: piny, color });
                ctx.draw(&Circle { x: cx, y: cy, radius: a * 0.22, color });
                let corner = |du: f64, side: f64| {
                    (pinx + ux * du * half_h + px * side * bore / 2.0, piny + uy * du * half_h + py * side * bore / 2.0)
                };
                let pts = [corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)];
                for k in 0..4 {
                    let (x1, y1) = pts[k];
                    let (x2, y2) = pts[(k + 1) % 4];
                    ctx.draw(&CLine { x1, y1, x2, y2, color });
                }
                let head = l + a + half_h + 0.05 * bore;
                let crown = xp + half_h;
                if head > crown + 0.02 * bore && stroke != "EXH" {
                    let mut coords = Vec::new();
                    let steps = 5;
                    for s in 0..=steps {
                        for t in 0..=steps {
                            let d = crown + (head - crown) * (s as f64 / steps as f64);
                            let side = (t as f64 / steps as f64 - 0.5) * 0.9 * bore;
                            coords.push((ux * d + px * side, uy * d + py * side));
                        }
                    }
                    ctx.draw(&Points { coords: &coords, color: gas });
                }
                ctx.print(cx * 1.35, cy * 1.35, Span::styled((i + 1).to_string(), Style::new().fg(color).bold()));
            }
        });
    f.render_widget(canvas, area);
}
