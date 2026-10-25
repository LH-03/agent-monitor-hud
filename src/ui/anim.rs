//! Time-based equivalents of the WPF storyboards in `HudAnimations`. Each
//! animation is evaluated from its start time, so frames are only produced
//! while something is actually moving.

use super::color::Color;
use super::layout::Fx;
use super::style::{EffectProfile, effect_profile};
use crate::config::Settings;
use crate::engine::Recipe;

#[derive(Clone, Debug)]
enum Track {
    /// `DoubleAnimation(from, to, half) { AutoReverse, RepeatBehavior = n }`.
    AutoReverse { from: f64, to: f64, half: f64, repeat: f64 },
    /// Linear key frames over `duration` (repeated `repeat` times).
    Keys { duration: f64, frames: Vec<(f64, f64)>, repeat: f64, hold_end: bool },
    /// `DoubleAnimation(from, to, ms)` without reverse.
    Linear { from: f64, to: f64, duration: f64, repeat: f64 },
}

impl Track {
    fn total(&self) -> f64 {
        match self {
            Track::AutoReverse { half, repeat, .. } => half * 2.0 * repeat,
            Track::Keys { duration, repeat, .. } => duration * repeat,
            Track::Linear { duration, repeat, .. } => duration * repeat,
        }
    }

    /// Value at `t` ms, or `None` once finished without hold.
    fn value(&self, t: f64) -> Option<f64> {
        if t < 0.0 {
            return None;
        }
        match self {
            Track::AutoReverse { from, to, half, repeat } => {
                if t >= half * 2.0 * repeat {
                    return None;
                }
                let phase = (t % (half * 2.0)) / half;
                let p = if phase <= 1.0 { phase } else { 2.0 - phase };
                Some(from + (to - from) * p)
            }
            Track::Keys { duration, frames, repeat, hold_end } => {
                if t >= duration * repeat {
                    return if *hold_end { frames.last().map(|f| f.1) } else { None };
                }
                let p = (t % duration) / duration;
                let mut previous = (0.0, frames.first().map(|f| f.1).unwrap_or(1.0));
                for &(at, value) in frames {
                    if p <= at {
                        let span = (at - previous.0).max(1e-9);
                        return Some(previous.1 + (value - previous.1) * ((p - previous.0) / span));
                    }
                    previous = (at, value);
                }
                Some(previous.1)
            }
            Track::Linear { from, to, duration, repeat } => {
                if t >= duration * repeat {
                    return None;
                }
                let p = (t % duration) / duration;
                Some(from + (to - from) * p)
            }
        }
    }
}

/// One running effect on one element.
#[derive(Clone, Debug)]
pub struct Anim {
    start: f64,
    opacity: Option<Track>,
    scale: Option<Track>,
    glow: Option<(Color, f64, Track, f64)>,
    flow: Option<(Vec<(f32, Color)>, Track, f32)>,
    /// When the target keeps the residual glow/flow state after the timeline
    /// (WPF effects persist until the attention is reset).
    until: f64,
}

fn repeat(value: f64) -> f64 {
    value.ceil().max(2.0)
}

impl Anim {
    pub fn finished(&self, now: f64) -> bool {
        now >= self.until
    }

    pub fn fx(&self, now: f64) -> Fx {
        let t = now - self.start;
        let mut fx = Fx::identity();
        if let Some(track) = &self.opacity {
            if let Some(v) = track.value(t) {
                fx.opacity = v as f32;
            }
        }
        if let Some(track) = &self.scale {
            if let Some(v) = track.value(t) {
                fx.scale = v as f32;
            }
        }
        if let Some((color, blur, track, base)) = &self.glow {
            let opacity = track.value(t).unwrap_or(*base);
            fx.glow = Some((*color, *blur as f32, opacity as f32));
        }
        if let Some((stops, track, thickness)) = &self.flow {
            // After the sweep the gradient rests outside the element (WPF
            // returns the transform to its base value).
            let offset = track.value(t).unwrap_or(match track {
                Track::Linear { from, .. } => *from,
                _ => -1.4,
            });
            fx.flow = Some((stops.clone(), offset as f32, *thickness));
        }
        fx
    }

}

fn profile(s: &Settings, color: &str, opacity: f64, blur: f64) -> EffectProfile {
    effect_profile(s, color, opacity, blur)
}

fn flow_stops(p: &EffectProfile) -> Vec<(f32, Color)> {
    let c = p.color;
    vec![
        (0.0, c.with_alpha(0)),
        (0.40, c.with_alpha(0x22)),
        (0.50, p.flow_core),
        (0.60, c.with_alpha(p.flow_shoulder_alpha)),
        (1.0, c.with_alpha(0)),
    ]
}

/// Status-dot reminder (`StartDot`).
pub fn dot(s: &Settings, dot_color: &str, now: f64, until: f64) -> Option<Anim> {
    if !s.attention.dot_enabled || !s.show_status_dot {
        return None;
    }
    let cycle = match s.attention.dot_speed.as_str() {
        "slow" => 1050.0,
        "fast" => 460.0,
        _ => 700.0,
    };
    let (minimum, glow, blur, scale): (f64, f64, f64, f64) = match s.attention.dot_brightness.as_str() {
        "subtle" => (0.58, 0.40, 8.0, 1.10),
        "bright" => (0.10, 1.00, 18.0, 1.28),
        _ => (0.28, 0.76, 13.0, 1.18),
    };
    let reps = repeat(s.attention.duration_seconds as f64 * 1000.0 / cycle);
    let frames = match s.attention.dot_pattern.as_str() {
        "soft" => vec![(0.0, 1.0), (0.5, minimum), (1.0, 1.0)],
        "beacon" => vec![(0.0, minimum), (0.7, 1.0), (1.0, minimum)],
        _ => vec![(0.0, 1.0), (0.16, minimum), (0.32, 1.0), (0.46, (minimum + 0.16).min(0.92)), (0.62, 1.0), (1.0, 1.0)],
    };
    let p = profile(s, dot_color, glow, blur);
    Some(Anim {
        start: now,
        opacity: Some(Track::Keys { duration: cycle, frames, repeat: reps, hold_end: false }),
        scale: s.attention.dot_breathing.then(|| Track::AutoReverse { from: 1.0, to: scale, half: cycle / 2.0, repeat: reps }),
        glow: Some((p.color, p.blur, Track::AutoReverse { from: p.minimum, to: p.peak, half: cycle / 2.0, repeat: reps }, p.minimum)),
        flow: None,
        until,
    })
}

/// Surface reminder (`StartSurface`): halo / flow / breathe / focus.
pub fn surface(s: &Settings, mode: &str, now: f64, until: f64) -> Option<Anim> {
    let duration = s.attention.duration_seconds as f64;
    let reps = repeat(duration / 0.9);
    let mut anim = Anim { start: now, opacity: None, scale: None, glow: None, flow: None, until };
    match mode {
        "off" => return None,
        "halo" => {
            let p = profile(s, &s.accent, 0.72, 20.0);
            anim.glow = Some((p.color, p.blur, Track::AutoReverse { from: p.minimum, to: p.peak, half: 620.0, repeat: reps }, p.minimum));
        }
        "flow" => {
            let p = profile(s, &s.accent, 0.78, 20.0);
            anim.flow = Some((flow_stops(&p), Track::Linear { from: -1.3, to: 1.3, duration: 1050.0, repeat: repeat(duration / 1.05) }, 2.0));
        }
        _ => {
            let strong = mode == "focus";
            let half = if strong { 300.0 } else { 560.0 };
            anim.scale = Some(Track::AutoReverse { from: 1.0, to: if strong { 1.035 } else { 1.016 }, half, repeat: reps });
            anim.opacity = Some(Track::AutoReverse { from: if strong { 0.48 } else { 0.78 }, to: 1.0, half, repeat: reps });
        }
    }
    Some(anim)
}

/// Context threshold alert (`StartContext`).
pub fn context(s: &Settings, level: i32, now: f64, until: f64) -> Anim {
    let (color, glow, blur, scale, cycle) = match level {
        1 => ("#FF0A84FF", 0.42, 14.0, 1.012, 760.0),
        2 => ("#FFFF9F0A", 0.74, 28.0, 1.028, 580.0),
        _ => ("#FFFF453A", 1.0, 42.0, 1.05, 420.0),
    };
    let p = profile(s, color, glow, blur);
    let reps = repeat(s.attention.duration_seconds as f64 * 1000.0 / cycle);
    Anim {
        start: now,
        opacity: None,
        scale: Some(Track::AutoReverse { from: 1.0, to: scale, half: cycle / 2.0, repeat: reps }),
        glow: Some((p.color, p.blur, Track::AutoReverse { from: p.minimum, to: p.peak, half: cycle / 2.0, repeat: reps }, p.minimum)),
        flow: None,
        until,
    }
}

pub fn default_recipe(s: &Settings) -> Recipe {
    let layers: Vec<&str> = match s.notices.mode.as_str() {
        "halo" => vec!["glow"],
        "breathe" => vec!["glow", "breathe"],
        "flow" => vec!["glow", "flow"],
        _ => vec!["glow", "pulse", "breathe"],
    };
    let (intensity, radius, scale) = match s.notices.intensity.as_str() {
        "subtle" => (0.42, 18.0, 1.012),
        "strong" => (0.95, 44.0, 1.055),
        _ => (0.70, 30.0, 1.028),
    };
    Recipe {
        layers: layers.into_iter().map(String::from).collect(),
        color: s.notices.color.clone(),
        intensity,
        tempo_ms: 720.0,
        cycles: ((s.notices.duration_seconds as f64 * 1000.0 / 720.0).ceil() as u32).clamp(1, 8),
        glow_radius: radius,
        scale,
        right_to_left: false,
    }
}

/// Agent notice choreography (`StartAgent`).
pub fn agent(s: &Settings, recipe: Option<&Recipe>, now: f64, until: f64) -> Anim {
    let fallback = default_recipe(s);
    let r = recipe.unwrap_or(&fallback);
    let p = profile(s, &r.color, r.intensity, r.glow_radius);
    let reps = r.cycles as f64;
    let has = |layer: &str| r.layers.iter().any(|l| l == layer);
    let from = if r.right_to_left { 1.4 } else { -1.4 };
    Anim {
        start: now,
        opacity: has("pulse").then(|| Track::AutoReverse { from: (1.0 - r.intensity * 0.42).max(0.5), to: 1.0, half: r.tempo_ms, repeat: reps }),
        scale: has("breathe").then(|| Track::AutoReverse { from: 1.0, to: r.scale, half: r.tempo_ms, repeat: reps }),
        glow: has("glow").then(|| (p.color, p.blur, Track::AutoReverse { from: p.minimum, to: p.peak, half: r.tempo_ms, repeat: reps }, p.minimum)),
        flow: has("flow").then(|| {
            (
                flow_stops(&p),
                Track::Linear { from, to: -from, duration: r.tempo_ms, repeat: reps },
                (0.8 + r.intensity * 1.7).clamp(1.0, 2.5) as f32,
            )
        }),
        until,
    }
}

/// Completion departure cue (`StartTerminalExit`); holds the final frame.
pub fn exit(s: &Settings, mode: &str, status_color: &str, now: f64, remaining_ms: f64) -> Anim {
    let max = match mode {
        "fade" => 1200.0,
        "focus" => 3600.0,
        "beacon" => 5000.0,
        _ => 2400.0,
    };
    let duration = remaining_ms.clamp(200.0, max);
    let opacity = match mode {
        "fade" => vec![(0.0, 1.0), (0.28, 1.0), (1.0, 0.0)],
        "focus" => vec![(0.0, 1.0), (0.16, 0.72), (0.30, 1.0), (0.46, 0.78), (0.60, 1.0), (0.80, 0.92), (1.0, 0.0)],
        "beacon" => vec![(0.0, 1.0), (0.10, 0.58), (0.20, 1.0), (0.31, 0.66), (0.42, 1.0), (0.54, 0.62), (0.66, 1.0), (0.82, 0.94), (1.0, 0.0)],
        _ => vec![(0.0, 1.0), (0.20, 0.82), (0.38, 1.0), (0.58, 0.86), (0.76, 1.0), (0.88, 0.94), (1.0, 0.0)],
    };
    let scale = match mode {
        "fade" => vec![(0.0, 1.0), (0.75, 1.0), (1.0, 0.985)],
        "focus" => vec![(0.0, 1.0), (0.18, 1.035), (0.34, 1.0), (0.50, 1.035), (0.68, 1.0), (0.84, 1.018), (1.0, 0.985)],
        "beacon" => vec![(0.0, 1.0), (0.12, 1.055), (0.24, 1.0), (0.36, 1.055), (0.48, 1.0), (0.60, 1.055), (0.73, 1.0), (0.86, 1.024), (1.0, 0.98)],
        _ => vec![(0.0, 1.0), (0.22, 1.016), (0.42, 1.0), (0.62, 1.016), (0.80, 1.0), (1.0, 0.985)],
    };
    let glow = (mode != "fade").then(|| {
        let p = profile(s, status_color, if mode == "beacon" { 1.0 } else { 0.68 }, if mode == "beacon" { 44.0 } else { 24.0 });
        (
            p.color,
            p.blur,
            Track::Keys { duration, frames: vec![(0.0, p.minimum), (0.18, p.peak), (0.58, p.peak), (1.0, 0.0)], repeat: 1.0, hold_end: true },
            0.0,
        )
    });
    Anim {
        start: now,
        opacity: Some(Track::Keys { duration, frames: opacity, repeat: 1.0, hold_end: true }),
        scale: Some(Track::Keys { duration, frames: scale, repeat: 1.0, hold_end: true }),
        glow,
        flow: None,
        until: now + duration + 60_000.0,
    }
}

/// The subtle whole-shell fade when tasks or phases change (`StartUpdateAnimation`).
pub fn update_fade(now: f64) -> Anim {
    Anim {
        start: now,
        opacity: Some(Track::Linear { from: 0.96, to: 1.0, duration: 160.0, repeat: 1.0 }),
        scale: None,
        glow: None,
        flow: None,
        until: now + 160.0,
    }
}

impl Anim {
    /// Whether frames still change (the residual state after the timeline is static).
    pub fn moving(&self, now: f64) -> bool {
        let t = now - self.start;
        let tracks = [self.opacity.as_ref(), self.scale.as_ref(), self.glow.as_ref().map(|g| &g.2), self.flow.as_ref().map(|f| &f.1)];
        tracks.into_iter().flatten().any(|track| t < track.total())
    }
}
