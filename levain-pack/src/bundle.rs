//! Desktop bundles (deb, AppImage, dmg, msi, ...) with the Tauri bundler,
//! around a host that is already built: nothing is compiled again.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// What to bundle.
#[derive(Debug, Default)]
pub struct Bundle {
    /// The language host's Tauri project (its `tauri.conf.json`).
    pub project: PathBuf,
    /// The host executable to ship.
    pub host: PathBuf,
    /// The host binary's name in its project (`baguette_host`).
    pub host_name: String,
    pub name: String,
    pub version: String,
    pub identifier: String,
    /// `(source, destination)` pairs: files or directories (ending in `/`)
    /// the bundle carries in its resources.
    pub resources: Vec<(String, String)>,
    pub bundles: Vec<String>,
    /// Scratch space for the bundler; it finds the host in `release/`.
    pub target_dir: PathBuf,
    /// Where the bundles go, one directory per format.
    pub out: PathBuf,
    /// The app's own Tauri configuration, merged over this one.
    pub extra_config: Option<String>,
}

/// The configuration merged over the project's for this bundle.
pub fn config(b: &Bundle) -> Value {
    let resources: serde_json::Map<String, Value> = b
        .resources
        .iter()
        .map(|(from, to)| (from.clone(), Value::String(to.clone())))
        .collect();
    json!({
        "productName": b.name,
        "mainBinaryName": format!("{}_app", b.name),
        "version": b.version,
        "identifier": b.identifier,
        "bundle": {"active": true, "targets": b.bundles, "resources": resources},
    })
}

/// The Tauri CLI's arguments, after `npx`.
pub fn tauri_args(b: &Bundle, config_path: &Path) -> Vec<String> {
    let mut args: Vec<String> = ["--yes", "@tauri-apps/cli@2", "bundle", "--config"]
        .map(String::from)
        .into();
    args.push(config_path.display().to_string());
    if let Some(extra) = &b.extra_config {
        args.extend(["--config".into(), extra.clone()]);
    }
    args.push("--bundles".into());
    args.extend(b.bundles.iter().cloned());
    args
}

/// The host under the names the bundler looks for: its project's binary and
/// the bundle's main binary.
pub fn stage_host(b: &Bundle) -> Result<(), String> {
    let release = b.target_dir.join("release");
    std::fs::create_dir_all(&release).map_err(|e| e.to_string())?;
    for name in [b.host_name.clone(), format!("{}_app", b.name)] {
        let to = release.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        std::fs::copy(&b.host, &to).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    Ok(())
}

/// The bundles themselves, not the bundler's staging directories, copied to
/// `out/<format>/`.
pub fn collect(b: &Bundle) -> Result<Vec<PathBuf>, String> {
    let mut copied = Vec::new();
    for format in &b.bundles {
        let dir = b.target_dir.join("release/bundle").join(format);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let from = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if !from.is_file() || !name.starts_with(&format!("{}_", b.name)) {
                continue;
            }
            let to_dir = b.out.join(format);
            std::fs::create_dir_all(&to_dir).map_err(|e| e.to_string())?;
            let to = to_dir.join(&name);
            std::fs::copy(&from, &to).map_err(|e| e.to_string())?;
            copied.push(to);
        }
    }
    Ok(copied)
}

pub fn run(b: &Bundle) -> Result<(), String> {
    std::fs::create_dir_all(&b.target_dir).map_err(|e| e.to_string())?;
    let config_path = b.target_dir.join("tauri.bundle.json");
    std::fs::write(&config_path, config(b).to_string()).map_err(|e| e.to_string())?;
    stage_host(b)?;
    eprintln!("* bundling {}", b.bundles.join(", "));
    crate::cmd(
        &b.project,
        "npx",
        &tauri_args(b, &config_path),
        &[
            ("CARGO_TARGET_DIR", b.target_dir.display().to_string()),
            ("NO_STRIP", "true".into()),
        ],
    )?;
    for path in collect(b)? {
        eprintln!("* bundled {}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle(tmp: &Path) -> Bundle {
        Bundle {
            project: tmp.join("project"),
            host: tmp.join("host"),
            host_name: "baguette_host".into(),
            name: "counter".into(),
            version: "0.1.0".into(),
            identifier: "dev.baguette.counter".into(),
            resources: vec![("/rel/lib/".into(), "release/lib/".into())],
            bundles: vec!["deb".into(), "appimage".into()],
            target_dir: tmp.join("target"),
            out: tmp.join("out"),
            extra_config: None,
        }
    }

    #[test]
    fn config_names_the_app_and_its_resources() {
        let b = bundle(Path::new("/t"));
        assert_eq!(
            config(&b),
            json!({
                "productName": "counter",
                "mainBinaryName": "counter_app",
                "version": "0.1.0",
                "identifier": "dev.baguette.counter",
                "bundle": {
                    "active": true,
                    "targets": ["deb", "appimage"],
                    "resources": {"/rel/lib/": "release/lib/"},
                },
            })
        );
    }

    #[test]
    fn the_apps_own_config_comes_last() {
        let mut b = bundle(Path::new("/t"));
        b.extra_config = Some("sign.json".into());
        assert_eq!(
            tauri_args(&b, Path::new("c.json")).join(" "),
            "--yes @tauri-apps/cli@2 bundle --config c.json --config sign.json --bundles deb appimage"
        );
    }

    #[test]
    fn stages_the_host_and_collects_only_bundles() {
        let tmp = tempfile::tempdir().unwrap();
        let b = bundle(tmp.path());
        std::fs::write(&b.host, "exe").unwrap();
        stage_host(&b).unwrap();
        let exe = std::env::consts::EXE_SUFFIX;
        for name in ["baguette_host", "counter_app"] {
            let staged = b.target_dir.join(format!("release/{name}{exe}"));
            assert_eq!(std::fs::read(staged).unwrap(), b"exe");
        }

        let deb = b.target_dir.join("release/bundle/deb");
        std::fs::create_dir_all(deb.join("counter_0.1.0_amd64")).unwrap();
        std::fs::write(deb.join("counter_0.1.0_amd64.deb"), "deb").unwrap();
        std::fs::write(deb.join("other.txt"), "x").unwrap();
        let copied = collect(&b).unwrap();
        assert_eq!(copied, [b.out.join("deb/counter_0.1.0_amd64.deb")]);
    }
}
