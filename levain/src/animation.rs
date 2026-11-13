//! Animations of any element's style, as GPUI's `Animation` and
//! `with_animation` run them: a style is interpolated from one set of
//! classes to another over time.
//!
//! ```heex
//! <div id="row-7-flash" class="absolute inset-0 bg-primary/25"
//!      animate-to="opacity-0" duration="1200" easing="ease-out" />
//! <div animate-from="w-0 bg-[#ff0000]" animate-to="w-64 bg-[#0000ff]"
//!      duration="600" delay="200" repeat="infinite" direction="alternate" />
//! ```
//!
//! - `animate-from` / `animate-to`: classes; a missing side is the element's
//!   own `class`. Every numeric or colour property interpolates: opacity,
//!   background, text and border colours, font size, width and height
//!   (and min/max), insets, margins, padding, gaps, corner radii and border
//!   widths. Lengths interpolate between like units (px with px, fractions
//!   with fractions); anything else switches halfway.
//! - `duration` and `delay` in ms (1000 and 0).
//! - `easing`: GPUI's `linear`, `quadratic` (also `ease-in`),
//!   `ease-in-out`, `ease-out` (its quint ease-out), `bounce` (out and
//!   back), `pulsate` (`pulsating_between(0, 1)`), or `cubic-bezier(x1, y1, x2, y2)`.
//! - `repeat`: how many times (1), or `infinite`.
//! - `direction`: `normal`, `reverse`, `alternate` or `alternate-reverse`.
//! - `fill`: what shows outside the active time, as in CSS: `forwards` (the
//!   default, which is what GPUI's `with_animation` does: the end holds),
//!   `backwards`, `both` or `none`.
//! - `animate`: a preset, `pulse`, `fade-in`, `fade-out` or `flash`, whose
//!   timing the attributes above can still change. (`spin` turns an `<icon>`.)
//!
//! An animation starts when its element first renders. It is keyed by the
//! element's `id` when it has one, so it survives the element moving (a row
//! prepended above it); a new `id` starts a new animation.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use tauri_plugin_gpui::gpui::{
    self, AbsoluteLength, DefiniteLength, Fill, Hsla, Length, Rgba, StyleRefinement, Styled,
};

use crate::style::{self, Theme, Variant};

/// A style to build up from classes, as an element's would be.
#[derive(Default)]
pub struct Refinement(pub StyleRefinement);

impl Styled for Refinement {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.0
    }
}

/// What `classes` set.
pub fn refinement(classes: &str, theme: &Theme) -> StyleRefinement {
    style::apply(
        Refinement::default(),
        style::for_variant(classes, Variant::Base),
        theme,
    )
    .0
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Easing {
    Linear,
    Quadratic,
    EaseInOut,
    EaseOut,
    Bounce,
    Pulsate,
    CubicBezier(f32, f32, f32, f32),
}

impl Easing {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name.trim() {
            "linear" => Self::Linear,
            "quadratic" | "ease-in" => Self::Quadratic,
            "ease-in-out" => Self::EaseInOut,
            "ease-out" => Self::EaseOut,
            "bounce" => Self::Bounce,
            "pulsate" => Self::Pulsate,
            other => {
                let args = other.strip_prefix("cubic-bezier(")?.strip_suffix(')')?;
                let n: Vec<f32> = args
                    .split(',')
                    .map(|n| n.trim().parse().ok())
                    .collect::<Option<_>>()?;
                let [x1, y1, x2, y2] = n[..] else {
                    return None;
                };
                Self::CubicBezier(x1, y1, x2, y2)
            }
        })
    }

    pub fn apply(self, t: f32) -> f32 {
        match self {
            Self::Linear => gpui::linear(t),
            Self::Quadratic => gpui::quadratic(t),
            Self::EaseInOut => gpui::ease_in_out(t),
            Self::EaseOut => gpui::ease_out_quint()(t),
            Self::Bounce => gpui::bounce(gpui::ease_in_out)(t),
            Self::Pulsate => gpui::pulsating_between(0., 1.)(t),
            Self::CubicBezier(x1, y1, x2, y2) => cubic_bezier(x1, y1, x2, y2, t),
        }
    }
}

/// CSS's `cubic-bezier`: y at the x that equals `t`, found by bisection.
fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    let at = |a: f32, b: f32, s: f32| {
        let u = 1. - s;
        3. * u * u * s * a + 3. * u * s * s * b + s * s * s
    };
    let (mut lo, mut hi) = (0f32, 1f32);
    for _ in 0..32 {
        let mid = (lo + hi) / 2.;
        if at(x1, x2, mid) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    at(y1, y2, (lo + hi) / 2.)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Direction {
    Normal,
    Reverse,
    Alternate,
    AlternateReverse,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FillMode {
    None,
    Forwards,
    Backwards,
    Both,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Timing {
    pub duration: Duration,
    pub delay: Duration,
    pub easing: Easing,
    /// How many times it runs; infinite is `f32::INFINITY`.
    pub iterations: f32,
    pub direction: Direction,
    pub fill: FillMode,
}

/// Where an animation is, `elapsed` after it started.
#[derive(Debug, PartialEq)]
pub struct Sample {
    /// Eased progress from `from` (0) to `to` (1), if the style applies now.
    pub progress: Option<f32>,
    /// Whether it still changes, so the next frame is wanted.
    pub running: bool,
}

impl Timing {
    pub fn sample(&self, elapsed: Duration) -> Sample {
        let duration = self.duration.as_secs_f32().max(0.001);
        let fills = |forwards: bool| match self.fill {
            FillMode::Both => true,
            FillMode::Forwards => forwards,
            FillMode::Backwards => !forwards,
            FillMode::None => false,
        };
        if elapsed < self.delay {
            let progress = fills(false).then(|| self.at(0, 0.));
            return Sample {
                progress,
                running: true,
            };
        }
        let iteration = (elapsed - self.delay).as_secs_f32() / duration;
        if iteration >= self.iterations {
            // The end of the last iteration: 1 into it, or 0 into the next
            // for a whole number of alternations.
            let last = (self.iterations.ceil() as u32).saturating_sub(1);
            let into = self.iterations - last as f32;
            let progress = fills(true).then(|| self.at(last, into));
            return Sample {
                progress,
                running: false,
            };
        }
        Sample {
            progress: Some(self.at(iteration.floor() as u32, iteration.fract())),
            running: true,
        }
    }

    /// Eased progress `into` (0 to 1) iteration number `iteration`.
    fn at(&self, iteration: u32, into: f32) -> f32 {
        let backwards = match self.direction {
            Direction::Normal => false,
            Direction::Reverse => true,
            Direction::Alternate => !iteration.is_multiple_of(2),
            Direction::AlternateReverse => iteration.is_multiple_of(2),
        };
        let t = self.easing.apply(into.clamp(0., 1.));
        if backwards { 1. - t } else { t }
    }
}

/// An element's animation: its two styles and its timing.
pub struct Spec {
    pub from: StyleRefinement,
    pub to: StyleRefinement,
    pub timing: Timing,
}

/// Defaults a preset sets: classes from and to, and its timing.
struct Preset {
    from: &'static str,
    to: &'static str,
    iterations: f32,
    direction: Direction,
    easing: Easing,
}

fn preset(name: &str) -> Option<Preset> {
    let once = |from, to| Preset {
        from,
        to,
        iterations: 1.,
        direction: Direction::Normal,
        easing: Easing::EaseOut,
    };
    Some(match name {
        "pulse" => Preset {
            from: "opacity-100",
            to: "opacity-40",
            iterations: f32::INFINITY,
            direction: Direction::Alternate,
            easing: Easing::EaseInOut,
        },
        "fade-in" => once("opacity-0", "opacity-100"),
        "fade-out" | "flash" => once("opacity-100", "opacity-0"),
        _ => return None,
    })
}

impl Spec {
    /// The animation `attrs` describe, if any.
    pub fn from_attrs(attrs: &BTreeMap<String, String>, theme: &Theme) -> Option<Self> {
        let preset = attrs.get("animate").and_then(|name| preset(name));
        let from = attrs.get("animate-from").map(String::as_str);
        let to = attrs.get("animate-to").map(String::as_str);
        if preset.is_none() && from.is_none() && to.is_none() {
            return None;
        }
        let own = attrs.get("class").map(String::as_str).unwrap_or("");
        let from = from.or(preset.as_ref().map(|p| p.from)).unwrap_or(own);
        let to = to.or(preset.as_ref().map(|p| p.to)).unwrap_or(own);
        let ms = |key: &str| attrs.get(key).and_then(|v| v.trim().parse::<u64>().ok());
        let timing = Timing {
            duration: Duration::from_millis(ms("duration").unwrap_or(1000)),
            delay: Duration::from_millis(ms("delay").unwrap_or(0)),
            easing: attrs
                .get("easing")
                .and_then(|e| Easing::parse(e))
                .or(preset.as_ref().map(|p| p.easing))
                .unwrap_or(Easing::Linear),
            iterations: match attrs.get("repeat").map(|r| r.trim()) {
                Some("infinite") => f32::INFINITY,
                Some(n) => n.parse().unwrap_or(1.),
                None => preset.as_ref().map_or(1., |p| p.iterations),
            },
            direction: match attrs.get("direction").map(String::as_str) {
                Some("reverse") => Direction::Reverse,
                Some("alternate") => Direction::Alternate,
                Some("alternate-reverse") => Direction::AlternateReverse,
                Some(_) => Direction::Normal,
                None => preset.as_ref().map_or(Direction::Normal, |p| p.direction),
            },
            fill: match attrs.get("fill").map(String::as_str) {
                Some("none") => FillMode::None,
                Some("backwards") => FillMode::Backwards,
                Some("both") => FillMode::Both,
                _ => FillMode::Forwards,
            },
        };
        Some(Self {
            from: refinement(from, theme),
            to: refinement(to, theme),
            timing,
        })
    }

    /// The style `elapsed` after it started, and whether it still runs.
    pub fn style_at(&self, elapsed: Duration) -> (Option<StyleRefinement>, bool) {
        let sample = self.timing.sample(elapsed);
        let style = sample
            .progress
            .map(|t| interpolate(&self.from, &self.to, t));
        (style, sample.running)
    }
}

/// When each animation started, by key, and when it last rendered: kept
/// while its element is around, or held in a list's rows.
#[derive(Default)]
pub struct Clock {
    started: std::collections::HashMap<String, (Instant, Instant)>,
}

/// How many animations' start times are kept: the least recently rendered
/// go first.
const KEPT: usize = 10_000;

impl Clock {
    /// How long the animation keyed `key` has run, starting it now if new.
    pub fn elapsed(&mut self, key: &str) -> Duration {
        let now = Instant::now();
        let (started, seen) = self.started.entry(key.to_owned()).or_insert((now, now));
        *seen = now;
        now - *started
    }

    pub fn prune(&mut self) {
        if self.started.len() > KEPT {
            let mut seen: Vec<Instant> = self.started.values().map(|(_, s)| *s).collect();
            seen.sort();
            let cutoff = seen[self.started.len() - KEPT];
            self.started.retain(|_, (_, s)| *s >= cutoff);
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.started.len()
    }
}

/// The key of an element's animation: its `id`, else its path.
pub fn key(attrs: &BTreeMap<String, String>, path: &str) -> String {
    match attrs.get("id") {
        Some(id) => format!("id:{id}"),
        None => format!("path:{path}"),
    }
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn color(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let (a, b) = (Rgba::from(a), Rgba::from(b));
    Hsla::from(Rgba {
        r: mix(a.r, b.r, t),
        g: mix(a.g, b.g, t),
        b: mix(a.b, b.b, t),
        a: mix(a.a, b.a, t),
    })
}

fn absolute(a: &AbsoluteLength, b: &AbsoluteLength, t: f32) -> AbsoluteLength {
    match (a, b) {
        (AbsoluteLength::Pixels(a), AbsoluteLength::Pixels(b)) => {
            AbsoluteLength::Pixels(gpui::px(mix(f32::from(*a), f32::from(*b), t)))
        }
        (AbsoluteLength::Rems(a), AbsoluteLength::Rems(b)) => {
            AbsoluteLength::Rems(gpui::rems(mix(a.0, b.0, t)))
        }
        _ => step(a, b, t),
    }
}

fn definite(a: &DefiniteLength, b: &DefiniteLength, t: f32) -> DefiniteLength {
    match (a, b) {
        (DefiniteLength::Absolute(a), DefiniteLength::Absolute(b)) => {
            DefiniteLength::Absolute(absolute(a, b, t))
        }
        (DefiniteLength::Fraction(a), DefiniteLength::Fraction(b)) => {
            DefiniteLength::Fraction(mix(*a, *b, t))
        }
        _ => step(a, b, t),
    }
}

fn length(a: &Length, b: &Length, t: f32) -> Length {
    match (a, b) {
        (Length::Definite(a), Length::Definite(b)) => Length::Definite(definite(a, b, t)),
        _ => step(a, b, t),
    }
}

/// What can't be interpolated switches halfway.
fn step<T: Clone>(a: &T, b: &T, t: f32) -> T {
    if t < 0.5 { a.clone() } else { b.clone() }
}

/// Each property set on both sides, `t` of the way from `from` to `to`;
/// one set on one side only takes that side's value.
fn both<T: Clone>(a: &Option<T>, b: &Option<T>, t: f32, f: impl Fn(&T, &T, f32) -> T) -> Option<T> {
    match (a, b) {
        (Some(a), Some(b)) => Some(f(a, b, t)),
        (a, b) => b.clone().or_else(|| a.clone()),
    }
}

pub fn interpolate(from: &StyleRefinement, to: &StyleRefinement, t: f32) -> StyleRefinement {
    let mut style = if t < 0.5 { from.clone() } else { to.clone() };
    style.opacity = both(&from.opacity, &to.opacity, t, |a, b, t| mix(*a, *b, t));
    style.border_color = both(&from.border_color, &to.border_color, t, |a, b, t| {
        color(*a, *b, t)
    });
    style.background = both(&from.background, &to.background, t, |a, b, t| {
        match (a, b) {
            (Fill::Color(a), Fill::Color(b)) => match (a.as_solid(), b.as_solid()) {
                (Some(a), Some(b)) => Fill::Color(gpui::solid_background(color(a, b, t))),
                _ => step(&Fill::Color(*a), &Fill::Color(*b), t),
            },
        }
    });
    style.text.color = both(&from.text.color, &to.text.color, t, |a, b, t| {
        color(*a, *b, t)
    });
    style.text.font_size = both(&from.text.font_size, &to.text.font_size, t, absolute);
    macro_rules! each {
        ($field:ident: $($side:ident),+ => $f:expr) => {
            $(style.$field.$side = both(&from.$field.$side, &to.$field.$side, t, $f);)+
        };
    }
    each!(size: width, height => length);
    each!(min_size: width, height => length);
    each!(max_size: width, height => length);
    each!(inset: top, right, bottom, left => length);
    each!(margin: top, right, bottom, left => length);
    each!(padding: top, right, bottom, left => definite);
    each!(gap: width, height => definite);
    each!(border_widths: top, right, bottom, left => absolute);
    each!(corner_radii: top_left, top_right, bottom_right, bottom_left => absolute);
    style
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn timing(iterations: f32, direction: Direction, fill: FillMode) -> Timing {
        Timing {
            duration: Duration::from_millis(1000),
            delay: Duration::from_millis(500),
            easing: Easing::Linear,
            iterations,
            direction,
            fill,
        }
    }

    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[test]
    fn samples_delay_iterations_and_the_end() {
        let t = timing(2., Direction::Normal, FillMode::Forwards);
        assert_eq!(
            t.sample(MS(0)),
            Sample {
                progress: None,
                running: true
            }
        );
        assert_eq!(t.sample(MS(750)).progress, Some(0.25));
        assert_eq!(t.sample(MS(1750)).progress, Some(0.25));
        assert_eq!(
            t.sample(MS(3000)),
            Sample {
                progress: Some(1.),
                running: false
            }
        );
        let none = timing(1., Direction::Normal, FillMode::None);
        assert_eq!(
            none.sample(MS(9000)),
            Sample {
                progress: None,
                running: false
            }
        );
        let both = timing(1., Direction::Normal, FillMode::Both);
        assert_eq!(both.sample(MS(0)).progress, Some(0.));
    }

    #[test]
    fn directions_run_backwards_and_alternate() {
        let reverse = timing(1., Direction::Reverse, FillMode::Forwards);
        assert_eq!(reverse.sample(MS(750)).progress, Some(0.75));
        let alternate = timing(2., Direction::Alternate, FillMode::Forwards);
        assert_eq!(alternate.sample(MS(1750)).progress, Some(0.75));
        // Two alternations end where they began.
        assert_eq!(alternate.sample(MS(5000)).progress, Some(0.));
        let forever = timing(f32::INFINITY, Direction::Alternate, FillMode::Forwards);
        assert!(forever.sample(MS(1_000_000)).running);
    }

    #[test]
    fn eases_as_gpui_and_css_do() {
        assert_eq!(Easing::parse("ease-in-out"), Some(Easing::EaseInOut));
        assert_eq!(Easing::parse("ease-in"), Some(Easing::Quadratic));
        assert_eq!(
            Easing::parse("cubic-bezier(0.4, 0, 0.2, 1)"),
            Some(Easing::CubicBezier(0.4, 0., 0.2, 1.))
        );
        assert_eq!(Easing::parse("cubic-bezier(1, 2)"), None);
        assert_eq!(Easing::parse("wobbly"), None);
        assert_eq!(Easing::Quadratic.apply(0.5), 0.25);
        // cubic-bezier(0, 0, 1, 1) is linear.
        let linear = Easing::CubicBezier(0., 0., 1., 1.);
        assert!((linear.apply(0.3) - 0.3).abs() < 1e-3);
        for easing in [
            Easing::Linear,
            Easing::EaseInOut,
            Easing::EaseOut,
            Easing::CubicBezier(0.4, 0., 0.2, 1.),
        ] {
            assert!(
                easing.apply(0.).abs() < 1e-3 && (easing.apply(1.) - 1.).abs() < 1e-3,
                "{easing:?}"
            );
        }
    }

    #[test]
    fn interpolates_numbers_colours_and_lengths() {
        let theme = Theme::new();
        let from = refinement(
            "opacity-100 w-[100px] bg-[#000000] p-[0px] rounded-sm",
            &theme,
        );
        let to = refinement(
            "opacity-0 w-[300px] bg-[#ffffff] p-[8px] rounded-lg",
            &theme,
        );
        let half = interpolate(&from, &to, 0.5);
        assert_eq!(half.opacity, Some(0.5));
        assert_eq!(half.size.width, Some(gpui::px(200.).into()));
        assert_eq!(half.padding.top, Some(gpui::px(4.).into()));
        assert_eq!(half.corner_radii.top_left, Some(gpui::px(5.).into()));
        let Some(Fill::Color(bg)) = half.background else {
            panic!("no background");
        };
        let grey = Rgba::from(bg.as_solid().unwrap());
        assert!((grey.r - 0.5).abs() < 0.01 && (grey.b - 0.5).abs() < 0.01);
    }

    #[test]
    fn unlike_lengths_switch_halfway() {
        let theme = Theme::new();
        let (from, to) = (
            refinement("w-[40%]", &theme),
            refinement("w-[100px]", &theme),
        );
        assert_eq!(interpolate(&from, &to, 0.4).size.width, from.size.width);
        assert_eq!(interpolate(&from, &to, 0.6).size.width, to.size.width);
    }

    #[test]
    fn reads_the_attributes_and_presets() {
        let theme = Theme::new();
        assert!(Spec::from_attrs(&attrs(&[("class", "p-2")]), &theme).is_none());

        let flash = Spec::from_attrs(
            &attrs(&[("animate", "flash"), ("duration", "1200")]),
            &theme,
        )
        .unwrap();
        assert_eq!(flash.timing.duration, MS(1200));
        assert_eq!(flash.timing.iterations, 1.);
        assert_eq!((flash.from.opacity, flash.to.opacity), (Some(1.), Some(0.)));
        // Once over, it holds the end and asks for no more frames.
        assert_eq!(flash.style_at(MS(5000)).0.unwrap().opacity, Some(0.));
        assert!(!flash.style_at(MS(5000)).1);

        // `animate-to` alone starts from the element's own classes.
        let spec = Spec::from_attrs(
            &attrs(&[
                ("class", "opacity-50"),
                ("animate-to", "opacity-0"),
                ("repeat", "infinite"),
                ("direction", "alternate"),
                ("easing", "ease-in"),
                ("delay", "100"),
                ("fill", "both"),
            ]),
            &theme,
        )
        .unwrap();
        assert_eq!(spec.from.opacity, Some(0.5));
        assert_eq!(spec.timing.iterations, f32::INFINITY);
        assert_eq!(spec.timing.direction, Direction::Alternate);
        assert_eq!(spec.timing.easing, Easing::Quadratic);
        assert_eq!(spec.timing.delay, MS(100));
        assert_eq!(spec.timing.fill, FillMode::Both);

        let pulse = Spec::from_attrs(&attrs(&[("animate", "pulse")]), &theme).unwrap();
        assert_eq!(pulse.timing.direction, Direction::Alternate);
        assert!(pulse.style_at(MS(1_000_000)).1);
    }

    #[test]
    fn keys_animations_by_id_and_keeps_their_start() {
        let path = |p| key(&attrs(&[]), p);
        assert_eq!(key(&attrs(&[("id", "e1")]), "root.0"), "id:e1");
        assert_eq!(path("root.0"), "path:root.0");

        let mut clock = Clock::default();
        let first = clock.elapsed("id:e1");
        std::thread::sleep(MS(5));
        // The same id, wherever it renders, keeps running.
        assert!(clock.elapsed("id:e1") > first);
        assert_eq!(clock.elapsed("id:e2"), Duration::ZERO);
        assert_eq!(clock.len(), 2);
    }
}
