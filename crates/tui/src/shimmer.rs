//! Shimmer animation — animated shine-sweep across text.
//!
//! Codex-style effect: a Gaussian band of highlight sweeps across text
//! on a 2-second period, producing a subtle "breathing" animation.

use std::sync::OnceLock;
use std::time::Instant;

use crossterm::style::Color;

/// Process start time — deterministic across restarts.
fn start_time() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

/// Base color for the spinner label (dim cyan).
const BASE: Color = Color::Rgb { r: 100, g: 140, b: 160 };
/// Highlight color at the center of the sweep.
const HIGHLIGHT: Color = Color::Rgb { r: 180, g: 230, b: 255 };

/// Sweep period in seconds.
const SWEEP_SECS: f32 = 2.0;
/// Gaussian band half-width in character positions.
const BAND_HALF: f32 = 5.0;
/// Padding (virtual chars) before/after text for smooth entry/exit.
const PADDING: usize = 10;

/// Produce a Vec of (char, Color) for shimmer-animated text.
/// Each character gets a color based on its distance from the sweep center.
pub fn shimmer_chars(text: &str) -> Vec<(char, Color)> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }

    let elapsed = start_time().elapsed().as_secs_f32();
    let period = chars.len() + PADDING * 2;
    let pos_f = (elapsed % SWEEP_SECS) / SWEEP_SECS * (period as f32);
    let pos = pos_f as isize;

    chars
        .iter()
        .enumerate()
        .map(|(i, &ch)| {
            let dist = ((i as isize) - pos).unsigned_abs() as f32;
            let t = if dist <= BAND_HALF {
                let x = std::f32::consts::PI * (dist / BAND_HALF);
                0.5 * (1.0 + x.cos()) * 0.9
            } else {
                0.0
            };
            let color = blend(HIGHLIGHT, BASE, t);
            (ch, color)
        })
        .collect()
}

/// Linearly blend two RGB colors. t=1.0 → a, t=0.0 → b.
fn blend(a: Color, b: Color, t: f32) -> Color {
    match (a, b) {
        (
            Color::Rgb { r: ar, g: ag, b: ab },
            Color::Rgb { r: br, g: bg, b: bb },
        ) => Color::Rgb {
            r: lerp_u8(br, ar, t),
            g: lerp_u8(bg, ag, t),
            b: lerp_u8(bb, ab, t),
        },
        _ => b,
    }
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    let v = a as f32 + (b as f32 - a as f32) * t;
    v.round().clamp(0.0, 255.0) as u8
}
