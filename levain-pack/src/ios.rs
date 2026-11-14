//! The iOS wrapper crate: a `staticlib` that re-exports a host's `run`, which
//! the Tauri iOS project links. The host's own `cdylib` (for Android) can't
//! link for iOS, and a library's crate types can't vary by target, so every
//! host needs this crate beside it. It is the same crate for every language,
//! rendered here from templates.

use std::path::{Path, PathBuf};

const CARGO_TOML: &str = include_str!("../templates/ios/Cargo.toml.tmpl");
const LIB_RS: &str = include_str!("../templates/ios/lib.rs.tmpl");
const TAURI_CONF: &str = include_str!("../templates/ios/tauri.conf.json.tmpl");
const PACKAGE_JSON: &str = include_str!("../templates/ios/package.json.tmpl");
const PACKAGE_LOCK: &str = include_str!("../templates/ios/package-lock.json.tmpl");

/// What differs between hosts.
#[derive(Debug, Clone, PartialEq)]
pub struct IosCrate {
    /// The host's package name, as in its `Cargo.toml` (`baguette_host`).
    pub host_crate: String,
    /// The host's `[lib] name` (`baguette_host_lib`).
    pub host_lib: String,
    /// The host crate's directory, relative to the wrapper (`../baguette_host`).
    pub host_path: String,
    /// Host features the wrapper forwards; `--feature`.
    pub features: Vec<String>,
    /// The forwarded features on by default; `--default-feature`.
    pub default_features: Vec<String>,
    /// The wrapper's package name; `<product>_ios` unless given.
    pub name: String,
    /// The app's product name; the host crate without `_host`.
    pub product_name: String,
    pub version: String,
    pub identifier: String,
}

impl IosCrate {
    pub fn new(host_crate: &str, host_lib: &str, host_path: &str) -> Self {
        let product = host_crate.strip_suffix("_host").unwrap_or(host_crate);
        IosCrate {
            host_crate: host_crate.into(),
            host_lib: host_lib.into(),
            host_path: host_path.into(),
            features: vec![],
            default_features: vec![],
            name: format!("{product}_ios"),
            product_name: product.into(),
            version: "0.1.0".into(),
            identifier: format!("dev.{product}.host"),
        }
    }

    /// The files to write, as (relative path, contents).
    pub fn render(&self) -> Result<Vec<(&'static str, String)>, String> {
        for (what, value) in [
            ("host-crate", &self.host_crate),
            ("host-lib", &self.host_lib),
            ("host-path", &self.host_path),
            ("name", &self.name),
            ("product-name", &self.product_name),
            ("version", &self.version),
            ("identifier", &self.identifier),
        ]
        .into_iter()
        .chain(self.features.iter().map(|f| ("feature", f)))
        {
            if value.is_empty()
                || value
                    .chars()
                    .any(|c| c.is_control() || "\"\\{}".contains(c))
            {
                return Err(format!("invalid --{what} {value:?}"));
            }
        }
        if let Some(f) = self
            .default_features
            .iter()
            .find(|f| !self.features.contains(f))
        {
            return Err(format!("--default-feature {f} is not a --feature"));
        }
        let features = self.feature_lines();
        let vars = [
            ("host_crate", self.host_crate.as_str()),
            ("host_lib", &self.host_lib),
            ("host_path", &self.host_path),
            ("crate_name", &self.name),
            ("lib_name", &format!("{}_lib", self.name)),
            ("npm_name", &self.name.replace('_', "-")),
            ("product_name", &self.product_name),
            ("version", &self.version),
            ("identifier", &self.identifier),
            ("features", &features),
        ];
        let fill = |template: &str| {
            vars.iter().fold(template.to_owned(), |text, (key, value)| {
                text.replace(&format!("{{{{{key}}}}}"), value)
            })
        };
        Ok(vec![
            ("Cargo.toml", fill(CARGO_TOML)),
            ("src/lib.rs", fill(LIB_RS)),
            ("tauri.conf.json", fill(TAURI_CONF)),
            ("package.json", fill(PACKAGE_JSON)),
            ("package-lock.json", fill(PACKAGE_LOCK)),
        ])
    }

    fn feature_lines(&self) -> String {
        let quoted = |list: &[String]| {
            list.iter()
                .map(|f| format!("\"{f}\""))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut lines = vec![format!("default = [{}]", quoted(&self.default_features))];
        for f in &self.features {
            lines.push(format!("{f} = [\"{}/{f}\"]", self.host_crate));
        }
        lines.join("\n")
    }
}

/// Writes the wrapper crate into `out`.
pub fn write(spec: &IosCrate, out: &Path) -> Result<PathBuf, String> {
    for (path, text) in spec.render()? {
        let file = out.join(path);
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(&file, text).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    eprintln!("* wrote {}", out.display());
    Ok(out.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// baguette's hand-written `native/baguette_ios`.
    fn baguette() -> IosCrate {
        let mut c = IosCrate::new("baguette_host", "baguette_host_lib", "../baguette_host");
        c.identifier = "dev.baguette.host".into();
        c.features = vec!["components-gpui-kit".into(), "playwright".into()];
        c.default_features = vec!["components-gpui-kit".into()];
        c
    }

    fn file(files: &[(&str, String)], name: &str) -> String {
        files.iter().find(|(p, _)| *p == name).unwrap().1.clone()
    }

    #[test]
    fn renders_what_baguette_ios_has_today() {
        let files = baguette().render().unwrap();
        assert_eq!(
            file(&files, "src/lib.rs"),
            "//! The iOS app's static library: the host, whose `run` is the app's entry\n\
             //! point (`tauri::mobile_entry_point` exports it as `start_app`, which the\n\
             //! Xcode project's `main` calls).\n\n\
             pub use baguette_host_lib::run;\n"
        );
        assert_eq!(
            file(&files, "package.json"),
            "{\n  \"name\": \"baguette-ios\",\n  \"private\": true,\n  \"scripts\": {\n    \
             \"tauri\": \"tauri\"\n  },\n  \"devDependencies\": {\n    \
             \"@tauri-apps/cli\": \"^2.12.1\"\n  }\n}\n"
        );
        assert_eq!(
            file(&files, "tauri.conf.json"),
            "{\n  \"$schema\": \"https://schema.tauri.app/config/2\",\n  \
             \"productName\": \"baguette\",\n  \"version\": \"0.1.0\",\n  \
             \"identifier\": \"dev.baguette.host\",\n  \"build\": {\n    \
             \"frontendDist\": \"../baguette_host/dist\"\n  },\n  \"app\": {\n    \
             \"windows\": [],\n    \"security\": { \"csp\": null }\n  },\n  \"bundle\": {\n    \
             \"active\": true,\n    \"icon\": [\"../baguette_host/icons/icon.png\"],\n    \
             \"iOS\": { \"minimumSystemVersion\": \"15.0\" }\n  }\n}\n"
        );
        let cargo = file(&files, "Cargo.toml");
        for line in [
            "name = \"baguette_ios\"",
            "publish = false",
            "name = \"baguette_ios_lib\"",
            "crate-type = [\"staticlib\"]",
            "baguette_host = { path = \"../baguette_host\", default-features = false }",
            "tauri = { version = \"2.12\", default-features = false }",
            "default = [\"components-gpui-kit\"]",
            "components-gpui-kit = [\"baguette_host/components-gpui-kit\"]",
            "playwright = [\"baguette_host/playwright\"]",
        ] {
            assert!(cargo.lines().any(|l| l == line), "{line} in\n{cargo}");
        }
        let lock = file(&files, "package-lock.json");
        assert!(lock.contains("\"name\": \"baguette-ios\""));
        assert!(lock.contains("\"lockfileVersion\": 3"));
        assert!(!lock.contains("{{"));
    }

    #[test]
    fn other_hosts_get_their_own_names() {
        let mut c = IosCrate::new("chouquette_host", "chouquette_host", "../host");
        c.identifier = "dev.example.app".into();
        let files = c.render().unwrap();
        let cargo = file(&files, "Cargo.toml");
        assert!(cargo.contains("name = \"chouquette_ios_lib\""));
        assert!(cargo.contains("chouquette_host = { path = \"../host\""));
        assert!(cargo.contains("default = []"));
        assert!(file(&files, "src/lib.rs").contains("pub use chouquette_host::run;"));
        assert!(file(&files, "tauri.conf.json").contains("\"productName\": \"chouquette\""));
        assert!(file(&files, "tauri.conf.json").contains("\"identifier\": \"dev.example.app\""));
        assert!(file(&files, "package.json").contains("\"name\": \"chouquette-ios\""));
        for (_, text) in &files {
            assert!(!text.contains("{{"), "{text}");
        }
    }

    #[test]
    fn rejects_values_that_would_break_the_files() {
        let mut c = baguette();
        c.identifier = "a\"b".into();
        assert!(c.render().is_err());
        let mut c = baguette();
        c.default_features = vec!["nope".into()];
        assert!(c.render().unwrap_err().contains("nope"));
    }

    #[test]
    fn writes_the_files() {
        let tmp = tempfile::tempdir().unwrap();
        write(&baguette(), &tmp.path().join("ios")).unwrap();
        assert!(tmp.path().join("ios/src/lib.rs").is_file());
        assert!(tmp.path().join("ios/package-lock.json").is_file());
    }
}
