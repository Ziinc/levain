//! Build-script helpers for hosts built on levain. A host's `build.rs` is
//! `levain_build::build();` plus whatever is specific to its language.

use std::ffi::OsString;

/// Runs `tauri-build` for a host.
///
/// GPUI embeds its own Windows manifest (DPI awareness, common controls v6);
/// a second one from Tauri fails the link (CVT1100), so Tauri's is left out.
///
/// # Panics
/// When `tauri-build` fails, as a build script's failure should.
pub fn build() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest()),
    )
    .expect("tauri-build");
}

/// The `TAURI_`-prefixed twin of an environment variable's name.
///
/// Xcode runs Cargo through the Tauri CLI, which passes on only `TAURI_*`,
/// `CARGO_*` and a few other variables, so build inputs set from outside
/// arrive under this name on iOS.
pub fn tauri_twin(name: &str) -> String {
    format!("TAURI_{name}")
}

/// The `cargo::rerun-if-env-changed` lines for a variable and its twin.
pub fn rerun_directives(name: &str) -> [String; 2] {
    [
        format!("cargo::rerun-if-env-changed={name}"),
        format!("cargo::rerun-if-env-changed={}", tauri_twin(name)),
    ]
}

/// `name`'s value, else its `TAURI_`-prefixed twin's, from `lookup`.
pub fn pick(name: &str, lookup: impl Fn(&str) -> Option<OsString>) -> Option<OsString> {
    lookup(name).or_else(|| lookup(&tauri_twin(name)))
}

/// Reads an environment variable, or its `TAURI_`-prefixed twin, and tells
/// Cargo to rerun the build script when either changes.
pub fn env_var(name: &str) -> Option<OsString> {
    for line in rerun_directives(name) {
        println!("{line}");
    }
    pick(name, |n| std::env::var_os(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(set: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |name| {
            set.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[test]
    fn the_plain_name_wins_over_its_twin() {
        let both = lookup(&[("X", "plain"), ("TAURI_X", "twin")]);
        assert_eq!(pick("X", both), Some("plain".into()));
    }

    #[test]
    fn the_twin_is_used_when_the_plain_name_is_unset() {
        assert_eq!(
            pick("X", lookup(&[("TAURI_X", "twin")])),
            Some("twin".into())
        );
        assert_eq!(pick("X", lookup(&[("Y", "other")])), None);
    }

    #[test]
    fn both_names_are_watched() {
        assert_eq!(
            rerun_directives("LEVAIN_APP_ARCHIVE"),
            [
                "cargo::rerun-if-env-changed=LEVAIN_APP_ARCHIVE",
                "cargo::rerun-if-env-changed=TAURI_LEVAIN_APP_ARCHIVE",
            ]
        );
    }
}
