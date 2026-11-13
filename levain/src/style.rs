//! Tailwind-style class strings mapped onto GPUI's `Styled` API.
//!
//! Only utilities GPUI can honour are supported; anything else is reported
//! once per class through [`unknown`] and otherwise ignored. Spacing follows
//! Tailwind's scale (1 = 4px), and colours are theme tokens (`bg-primary`),
//! `white`, `black`, `transparent` or arbitrary hex (`bg-[#2563eb]`).

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use tauri_plugin_gpui::gpui::{FontWeight, Hsla, Rgba, Styled, px, relative, rgb};

/// Theme tokens sent by Elixir, by name.
pub type Theme = HashMap<String, String>;

/// The state variants a class can be scoped to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Base,
    Hover,
    Active,
    Disabled,
}

/// Splits `classes` into the utilities for `variant`, with the prefix removed.
pub fn for_variant(classes: &str, variant: Variant) -> impl Iterator<Item = &str> {
    classes.split_whitespace().filter_map(move |class| {
        let (prefix, rest) = match class.split_once(':') {
            Some((prefix, rest)) => (Some(prefix), rest),
            None => (None, class),
        };
        matches!(
            (prefix, variant),
            (None, Variant::Base)
                | (Some("hover"), Variant::Hover)
                | (Some("active"), Variant::Active)
                | (Some("disabled"), Variant::Disabled)
        )
        .then_some(rest)
    })
}

pub fn apply<S: Styled>(
    mut style: S,
    utilities: impl Iterator<Item = impl AsRef<str>>,
    theme: &Theme,
) -> S {
    for utility in utilities {
        style = apply_one(style, utility.as_ref(), theme);
    }
    style
}

fn apply_one<S: Styled>(s: S, class: &str, theme: &Theme) -> S {
    match class {
        "flex" => s.flex(),
        "flex-row" => s.flex().flex_row(),
        "flex-col" => s.flex().flex_col(),
        "flex-wrap" => s.flex_wrap(),
        "flex-1" => s.flex_1(),
        "min-w-0" => s.min_w(px(0.)),
        "items-stretch" => s.items_stretch(),
        "text-right" => s.text_right(),
        "text-center" => s.text_center(),
        "font-mono" => s.font_family("monospace"),
        "border-2" => s.border_2(),
        "border-l-2" => s.border_l_2(),
        "border-l-4" => s.border_l_4(),
        "border-t" => s.border_t_1(),
        "border-b" => s.border_b_1(),
        "border-l" => s.border_l_1(),
        "border-r" => s.border_r_1(),
        "border-x" => s.border_x_1(),
        "border-y" => s.border_y_1(),
        "grow" => s.flex_grow_1(),
        "shrink-0" => s.flex_shrink_0(),
        "items-start" => s.items_start(),
        "items-center" => s.items_center(),
        "items-end" => s.items_end(),
        "justify-start" => s.justify_start(),
        "justify-center" => s.justify_center(),
        "justify-end" => s.justify_end(),
        "justify-between" => s.justify_between(),
        "size-full" => s.size_full(),
        "relative" => s.relative(),
        "absolute" => s.absolute(),
        "w-full" => s.w_full(),
        "h-full" => s.h_full(),
        "overflow-hidden" => s.overflow_hidden(),
        // Scrolling needs the element's id, so the view applies these.
        "overflow-scroll" | "overflow-auto" | "overflow-x-scroll" | "overflow-x-auto"
        | "overflow-y-scroll" | "overflow-y-auto" => s,
        "border" => s.border_1(),
        "rounded" => s.rounded(px(4.)),
        "rounded-sm" => s.rounded(px(2.)),
        "rounded-md" => s.rounded(px(6.)),
        "rounded-lg" => s.rounded(px(8.)),
        "rounded-xl" => s.rounded(px(12.)),
        "rounded-full" => s.rounded_full(),
        "shadow-sm" => s.shadow_sm(),
        "shadow" | "shadow-md" => s.shadow_md(),
        "shadow-lg" => s.shadow_lg(),
        "text-xs" => s.text_xs(),
        "text-sm" => s.text_sm(),
        "text-base" => s.text_base(),
        "text-lg" => s.text_lg(),
        "text-xl" => s.text_xl(),
        "text-2xl" => s.text_2xl(),
        "text-3xl" => s.text_3xl(),
        "font-normal" => s.font_weight(FontWeight::NORMAL),
        "font-medium" => s.font_weight(FontWeight::MEDIUM),
        "font-semibold" => s.font_weight(FontWeight::SEMIBOLD),
        "font-bold" => s.font_weight(FontWeight::BOLD),
        "cursor-pointer" => s.cursor_pointer(),
        "cursor-default" => s.cursor_default(),
        _ => apply_parametric(s, class, theme),
    }
}

fn apply_parametric<S: Styled>(s: S, class: &str, theme: &Theme) -> S {
    if let Some(n) = class
        .strip_prefix("opacity-")
        .and_then(|n| n.parse::<f32>().ok())
    {
        return s.opacity(n / 100.);
    }
    // Percentages of the parent: `w-[40%]`, `h-[75%]`.
    for (name, horizontal) in [("w-[", true), ("h-[", false)] {
        if let Some(fraction) = class
            .strip_prefix(name)
            .and_then(|rest| rest.strip_suffix("%]"))
            .and_then(|n| n.parse::<f32>().ok())
        {
            let len = relative(fraction / 100.);
            return if horizontal { s.w(len) } else { s.h(len) };
        }
    }
    const SPACING: &[&str] = &[
        "max-w", "max-h", "min-h", "min-w", "size", "gap", "px", "py", "pt", "pb", "pl", "pr",
        "mx", "my", "mt", "mb", "p", "m", "w", "h", "left", "top", "right", "bottom",
    ];
    for name in SPACING {
        if let Some(len) = class
            .strip_prefix(name)
            .and_then(|rest| rest.strip_prefix('-'))
            .and_then(spacing)
        {
            let len = px(len);
            return match *name {
                "p" => s.p(len),
                "px" => s.px(len),
                "py" => s.py(len),
                "pt" => s.pt(len),
                "pb" => s.pb(len),
                "pl" => s.pl(len),
                "pr" => s.pr(len),
                "m" => s.m(len),
                "mx" => s.mx(len),
                "my" => s.my(len),
                "mt" => s.mt(len),
                "mb" => s.mb(len),
                "gap" => s.gap(len),
                "w" => s.w(len),
                "h" => s.h(len),
                "size" => s.size(len),
                "max-w" => s.max_w(len),
                "max-h" => s.max_h(len),
                "min-h" => s.min_h(len),
                "left" => s.left(len),
                "top" => s.top(len),
                "right" => s.right(len),
                "bottom" => s.bottom(len),
                _ => s.min_w(len),
            };
        }
    }
    for name in ["bg", "text", "border"] {
        if let Some(color) = class
            .strip_prefix(name)
            .and_then(|rest| rest.strip_prefix('-'))
            .and_then(|value| color(value, theme))
        {
            return match name {
                "bg" => s.bg(color),
                "text" => s.text_color(color),
                _ => s.border_color(color),
            };
        }
    }
    generated(s, class)
}

/// A class named for one of `Styled`'s methods (`style_gen.rs`).
fn generated<S: Styled>(mut s: S, class: &str) -> S {
    match crate::style_gen::refinement(class) {
        Some(refinement) => {
            tauri_plugin_gpui::gpui::Refineable::refine(s.style(), &refinement);
            s
        }
        None => unknown(s, class),
    }
}

/// Tailwind's spacing scale in pixels: `4` is 16px, `px` is 1px, and
/// `[13px]` is exactly that.
fn spacing(value: &str) -> Option<f32> {
    if let Some(pixels) = value.strip_prefix('[').and_then(|v| v.strip_suffix("px]")) {
        return pixels.parse().ok();
    }
    match value {
        "px" => Some(1.),
        _ => value.parse::<f32>().ok().map(|n| n * 4.),
    }
}

pub fn color(value: &str, theme: &Theme) -> Option<Hsla> {
    let (value, alpha) = match value.rsplit_once('/') {
        Some((value, alpha)) => (value, alpha.parse::<f32>().ok()? / 100.),
        None => (value, 1.),
    };
    let rgba = match value {
        "white" => rgb(0xffffff),
        "black" => rgb(0x000000),
        "transparent" => Rgba {
            r: 0.,
            g: 0.,
            b: 0.,
            a: 0.,
        },
        _ => {
            let hex = match value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
                Some(hex) => hex,
                None => theme.get(value)?.as_str(),
            };
            hex_color(hex)?
        }
    };
    Some(Hsla::from(rgba).opacity(alpha))
}

fn hex_color(hex: &str) -> Option<Rgba> {
    let digits = hex.strip_prefix('#')?;
    let value = u32::from_str_radix(digits, 16).ok()?;
    match digits.len() {
        6 => Some(rgb(value)),
        8 => Some(Rgba {
            r: ((value >> 24) & 0xff) as f32 / 255.,
            g: ((value >> 16) & 0xff) as f32 / 255.,
            b: ((value >> 8) & 0xff) as f32 / 255.,
            a: (value & 0xff) as f32 / 255.,
        }),
        _ => None,
    }
}

/// The unsupported classes reported so far.
static REPORTED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

#[cfg(test)]
fn reported(class: &str) -> bool {
    REPORTED
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|r| r.contains(class))
}

fn unknown<S>(s: S, class: &str) -> S {
    let mut reported = REPORTED.lock().unwrap();
    if reported.get_or_insert_default().insert(class.to_owned()) {
        eprintln!("baguette: unsupported class `{class}`");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_split_by_prefix() {
        let classes = "px-4 hover:opacity-90 active:bg-primary disabled:opacity-50 focus:ring";
        assert_eq!(
            for_variant(classes, Variant::Base).collect::<Vec<_>>(),
            ["px-4"]
        );
        assert_eq!(
            for_variant(classes, Variant::Hover).collect::<Vec<_>>(),
            ["opacity-90"]
        );
        assert_eq!(
            for_variant(classes, Variant::Active).collect::<Vec<_>>(),
            ["bg-primary"]
        );
        assert_eq!(
            for_variant(classes, Variant::Disabled).collect::<Vec<_>>(),
            ["opacity-50"]
        );
    }

    #[test]
    fn colors_resolve_tokens_hex_and_alpha() {
        let theme = Theme::from([("primary".into(), "#2563eb".into())]);
        assert_eq!(color("primary", &theme), Some(Hsla::from(rgb(0x2563eb))));
        assert_eq!(color("[#ff0000]", &theme), Some(Hsla::from(rgb(0xff0000))));
        assert_eq!(color("white/50", &theme).map(|c| c.a), Some(0.5));
        assert_eq!(color("missing", &theme), None);
    }

    #[test]
    fn scroll_classes_are_supported() {
        // The view makes these elements scroll (it needs their id); styling
        // must not report them.
        for class in [
            "overflow-scroll",
            "overflow-auto",
            "overflow-x-scroll",
            "overflow-x-auto",
            "overflow-y-scroll",
            "overflow-y-auto",
        ] {
            apply(
                tauri_plugin_gpui::gpui::div(),
                [class].into_iter(),
                &Theme::new(),
            );
            assert!(!reported(class), "{class} reported as unsupported");
        }
        apply(
            tauri_plugin_gpui::gpui::div(),
            ["overflow-sideways"].into_iter(),
            &Theme::new(),
        );
        assert!(reported("overflow-sideways"));
    }

    // `style_gen.rs`, from `mix baguette.styles`.
    #[test]
    fn styled_methods_are_classes_too() {
        use crate::style_gen::refinement;
        for class in [
            "w-1/2",
            "-mt-2",
            "p-0.5",
            "rounded-t-lg",
            "cursor-grab",
            "gap-x-3",
        ] {
            assert!(refinement(class).is_some(), "{class}");
        }
        // Outside the stable categories, or not a method without arguments.
        for class in ["debug", "visible", "bg", "w"] {
            assert!(refinement(class).is_none(), "{class}");
        }
        for class in ["rounded-tl-xl", "inset-0", "whitespace-nowrap"] {
            apply(
                tauri_plugin_gpui::gpui::div(),
                [class].into_iter(),
                &Theme::new(),
            );
            assert!(!reported(class), "{class} reported as unsupported");
        }
    }

    // Tailwind's bare side borders are 1px, like `border`.
    #[test]
    fn side_borders_without_a_width_are_one_pixel() {
        for class in [
            "border-t", "border-b", "border-l", "border-r", "border-x", "border-y",
        ] {
            apply(
                tauri_plugin_gpui::gpui::div(),
                [class].into_iter(),
                &Theme::new(),
            );
            assert!(!reported(class), "{class} reported as unsupported");
        }
    }

    #[test]
    fn spacing_uses_the_tailwind_scale() {
        assert_eq!(spacing("4"), Some(16.));
        assert_eq!(spacing("0.5"), Some(2.));
        assert_eq!(spacing("[13px]"), Some(13.));
        assert_eq!(spacing("px"), Some(1.));
        assert_eq!(spacing("primary"), None);
    }
}
