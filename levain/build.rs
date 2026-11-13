use std::env;
use std::path::PathBuf;

fn main() {
    // `desktop` and `mobile`, as tauri-build sets them for an app crate.
    println!("cargo::rustc-check-cfg=cfg(desktop)");
    println!("cargo::rustc-check-cfg=cfg(mobile)");
    let mobile = matches!(
        env::var("CARGO_CFG_TARGET_OS").as_deref(),
        Ok("android" | "ios")
    );
    println!(
        "cargo::rustc-cfg={}",
        if mobile { "mobile" } else { "desktop" }
    );
    record_versions();
    embed_app();
}

/// A levain variable, or the same with a `TAURI_` prefix: Tauri's iOS build
/// (Cargo run from Xcode) passes on only `TAURI_*`, `CARGO_*` and a few
/// others.
fn levain_env(name: &str) -> Option<std::ffi::OsString> {
    let tauri = format!("TAURI_{name}");
    println!("cargo::rerun-if-env-changed={name}");
    println!("cargo::rerun-if-env-changed={tauri}");
    env::var_os(name).or_else(|| env::var_os(tauri))
}

/// On mobile the app ships inside the library: `LEVAIN_APP_ARCHIVE` names
/// a `.tar.gz` of it, which `mobile::unpack` unpacks on first launch.
fn embed_app() {
    println!("cargo::rustc-check-cfg=cfg(levain_embedded_app)");
    let Some(archive) = levain_env("LEVAIN_APP_ARCHIVE").map(PathBuf::from) else {
        println!("cargo::rustc-env=LEVAIN_APP_HASH=none");
        return;
    };
    println!("cargo::rerun-if-changed={}", archive.display());
    let bytes = std::fs::read(&archive)
        .unwrap_or_else(|e| panic!("LEVAIN_APP_ARCHIVE {}: {e}", archive.display()));
    // FNV-1a: names the unpacked directory, so a new build unpacks afresh.
    let hash = bytes.iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    });
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("app.tar.gz");
    std::fs::write(&out, bytes).unwrap();
    println!("cargo::rustc-env=LEVAIN_APP_HASH={hash:016x}");
    println!("cargo::rustc-cfg=levain_embedded_app");
}

/// The GPUI and gpui-kit versions in levain's Cargo.lock, which each
/// component set's manifest records (`components.rs`). A host that depends
/// on levain by git revision gets the checkout's own Cargo.lock.
fn record_versions() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let lock = manifest.join("../Cargo.lock");
    println!("cargo::rerun-if-changed={}", lock.display());
    let lock = std::fs::read_to_string(&lock).unwrap_or_default();
    for (crate_name, var) in [
        ("gpui-pre", "LEVAIN_GPUI_VERSION"),
        ("gpui-kit", "LEVAIN_GPUI_KIT_VERSION"),
    ] {
        let version = locked_version(&lock, crate_name).unwrap_or("unknown");
        println!("cargo::rustc-env={var}={version}");
    }
}

fn locked_version<'a>(lock: &'a str, crate_name: &str) -> Option<&'a str> {
    let name = format!("name = \"{crate_name}\"\n");
    let at = lock.find(&name)? + name.len();
    lock[at..]
        .lines()
        .next()?
        .strip_prefix("version = \"")?
        .strip_suffix('"')
}
