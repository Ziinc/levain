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

/// The `cargo::metadata` lines that pass on the Android libraries of the
/// Tauri plugins a crate depends on (its `DEP_*_ANDROID_LIBRARY_PATH`
/// variables), sorted.
///
/// tauri-build adds a plugin's Kotlin code to the Android project from the
/// app crate's own `DEP_*_ANDROID_LIBRARY_PATH` variables, and Cargo gives a
/// build script only its direct dependencies' metadata. levain depends on
/// the plugins in a host's place, so its build script (with `links =
/// "levain"`) prints these, and the host's sees each as
/// `DEP_LEVAIN_*_ANDROID_LIBRARY_PATH`. Tauri's own library is left out:
/// the host depends on tauri and gets it directly.
pub fn android_library_metadata(vars: impl IntoIterator<Item = (String, String)>) -> Vec<String> {
    let mut lines: Vec<String> = vars
        .into_iter()
        .filter(|(key, _)| key != "DEP_TAURI_ANDROID_LIBRARY_PATH")
        .filter_map(|(key, path)| {
            let name = key.strip_prefix("DEP_")?;
            name.ends_with("_ANDROID_LIBRARY_PATH")
                .then(|| format!("cargo::metadata={}={path}", name.to_lowercase()))
        })
        .collect();
    lines.sort();
    lines
}

/// Passes on the Android libraries of the Tauri plugins this crate depends
/// on to the crates that depend on it (see [`android_library_metadata`]).
/// For a build script of a crate with a `links` key.
pub fn reexport_android_libraries() {
    let vars = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)));
    for line in android_library_metadata(vars) {
        println!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugins_android_libraries_are_passed_on_but_not_tauris_own() {
        let vars = [
            (
                "DEP_TAURI_PLUGIN_DIALOG_ANDROID_LIBRARY_PATH",
                "/p/dialog/android",
            ),
            (
                "DEP_TAURI_PLUGIN_GPUI_ANDROID_LIBRARY_PATH",
                "/p/gpui/android",
            ),
            // The host depends on tauri itself and gets this one directly.
            ("DEP_TAURI_ANDROID_LIBRARY_PATH", "/p/tauri/android"),
            ("DEP_TAURI_DEV", "true"),
            ("PATH", "/bin"),
        ]
        .map(|(k, v)| (k.to_owned(), v.to_owned()));
        assert_eq!(
            android_library_metadata(vars),
            [
                "cargo::metadata=tauri_plugin_dialog_android_library_path=/p/dialog/android",
                "cargo::metadata=tauri_plugin_gpui_android_library_path=/p/gpui/android",
            ]
        );
    }

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
    fn an_unset_variable_is_none_under_both_names() {
        assert_eq!(env_var("LEVAIN_BUILD_TEST_SURELY_UNSET"), None);
    }

    #[test]
    fn a_variable_that_is_set_is_read() {
        // PATH is set wherever Cargo runs.
        assert_eq!(env_var("PATH"), std::env::var_os("PATH"));
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
