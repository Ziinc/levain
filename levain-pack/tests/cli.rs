//! The `levain-pack` binary, run as a user runs it: the tools it calls (`npm`,
//! `npx`, `xcodegen`) are scripts on a private `PATH` that log what they were
//! asked and make what the real ones would, so no network or toolchain is
//! needed.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pack(dir: &Path, path: Option<&Path>, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_levain-pack"));
    command.args(args).current_dir(dir);
    for proxy in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        command.env_remove(proxy);
    }
    if let Some(bin) = path {
        let old = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![bin.to_path_buf()];
        paths.extend(std::env::split_paths(&old));
        command.env("PATH", std::env::join_paths(paths).unwrap());
    }
    command.output().expect("levain-pack runs")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn it_prints_the_desktop_target() {
    let tmp = tempfile::tempdir().unwrap();
    let output = pack(tmp.path(), None, &["target"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let target = stdout(&output);
    let (cpu, os) = target.trim().split_once('-').expect("<cpu>-<os>");
    assert!(["x86_64", "aarch64"].contains(&cpu), "{target}");
    assert!(["linux", "macos", "windows"].contains(&os), "{target}");
}

#[test]
fn mistakes_end_in_a_message_and_a_failing_exit() {
    let tmp = tempfile::tempdir().unwrap();
    for (args, message) in [
        (&[][..], "usage: levain-pack"),
        (&["frobnicate"][..], "unknown command frobnicate"),
        (&["target", "--nope", "1"][..], "unknown option --nope"),
        (&["install", "only-one"][..], "expected 2 arguments, got 1"),
        (&["bundle"][..], "--project is required"),
    ] {
        let output = pack(tmp.path(), None, args);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        let error = stderr(&output);
        assert!(error.starts_with("levain-pack: "), "{error}");
        assert!(error.contains(message), "{args:?}: {error}");
    }
}

#[test]
fn installing_copies_the_host_over_the_old_one() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("host"), "new host").unwrap();
    let dest = tmp.path().join("bin/app");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    std::fs::write(&dest, "old host").unwrap();
    let output = pack(
        tmp.path(),
        None,
        &["install", "host", dest.to_str().unwrap()],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(std::fs::read(&dest).unwrap(), b"new host");
    assert!(stderr(&output).contains("installed"));
}

#[test]
fn the_ios_crate_takes_its_optional_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("wrapper");
    let output = pack(
        tmp.path(),
        None,
        &[
            "ios-crate",
            "--host-crate",
            "h_host",
            "--host-lib",
            "h_lib",
            "--host-path",
            "../h",
            "--feature",
            "d",
            "--default-feature",
            "d",
            "--feature",
            "f",
            "--name",
            "wrap",
            "--product-name",
            "Wrap",
            "--version",
            "2.3.4",
            "--identifier",
            "dev.example.wrap",
            "--out",
            out.to_str().unwrap(),
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let cargo = std::fs::read_to_string(out.join("Cargo.toml")).unwrap();
    assert!(cargo.contains("name = \"wrap\""), "{cargo}");
    assert!(cargo.contains("f = [\"h_host/f\"]"), "{cargo}");
    let tauri = std::fs::read_to_string(out.join("tauri.conf.json")).unwrap();
    assert!(tauri.contains("\"productName\": \"Wrap\""), "{tauri}");
    assert!(tauri.contains("\"version\": \"2.3.4\""), "{tauri}");
    assert!(
        tauri.contains("\"identifier\": \"dev.example.wrap\""),
        "{tauri}"
    );
}

/// A one-request-per-connection HTTP server on localhost serving `files`,
/// and the base URL to reach it.
fn serve(files: Vec<(&'static str, Vec<u8>)>) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut request = [0u8; 2048];
            let n = stream.read(&mut request).unwrap_or(0);
            let head = String::from_utf8_lossy(&request[..n]).into_owned();
            let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let response = match files.iter().find(|(name, _)| path == format!("/{name}")) {
                Some((_, body)) => {
                    let mut response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    response.extend(body);
                    response
                }
                None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_vec(),
            };
            let _ = stream.write_all(&response);
        }
    });
    base
}

fn tar_gz(name: &str, body: &[u8]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut tar = tar::Builder::new(gz);
    let mut header = tar::Header::new_gnu();
    header.set_size(body.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    tar.append_data(&mut header, name, body).unwrap();
    tar.into_inner().unwrap().finish().unwrap()
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn fetch_downloads_checks_and_caches_an_asset() {
    let tmp = tempfile::tempdir().unwrap();
    let archive = tar_gz("host", b"the host");
    let base = serve(vec![("host-x86_64-linux.tar.gz", archive.clone())]);
    let sums = tmp.path().join("sums.txt");
    std::fs::write(
        &sums,
        format!(
            "{}  host-x86_64-linux.tar.gz\n{}  missing.tar.gz\n{}  host-wrong.tar.gz\n",
            sha256(&archive),
            sha256(b"x"),
            sha256(b"y")
        ),
    )
    .unwrap();
    let cache = tmp.path().join("cache");
    let fetch = |asset: &str| {
        pack(
            tmp.path(),
            None,
            &[
                "fetch",
                &format!("{base}/"),
                asset,
                "--sha256",
                sums.to_str().unwrap(),
                "--cache",
                cache.to_str().unwrap(),
            ],
        )
    };

    let output = fetch("host-x86_64-linux.tar.gz");
    assert!(output.status.success(), "{}", stderr(&output));
    let dir = PathBuf::from(stdout(&output).trim());
    assert_eq!(dir, cache.join("host-x86_64-linux"));
    assert_eq!(std::fs::read(dir.join("host")).unwrap(), b"the host");
    assert!(stderr(&output).contains("downloading"));

    // The second time it is already there, and is not downloaded again.
    let again = fetch("host-x86_64-linux.tar.gz");
    assert!(again.status.success());
    assert!(!stderr(&again).contains("downloading"));

    // A 404 is an error naming the URL.
    let output = fetch("missing.tar.gz");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("GET http://"),
        "{}",
        stderr(&output)
    );
    // A body that does not match its pinned checksum is refused, and left nowhere.
    let wrong = serve(vec![("host-wrong.tar.gz", archive)]);
    let output = pack(
        tmp.path(),
        None,
        &[
            "fetch",
            &wrong,
            "host-wrong.tar.gz",
            "--sha256",
            sums.to_str().unwrap(),
            "--cache",
            cache.to_str().unwrap(),
        ],
    );
    assert!(
        stderr(&output).contains("pinned SHA-256"),
        "{}",
        stderr(&output)
    );
    assert!(!cache.join("host-wrong").exists());
}

#[cfg(unix)]
mod tools {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn script(bin: &Path, name: &str, body: &str) {
        let path = bin.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A project directory, a `PATH` directory with the fake tools, and the log.
    struct Setup {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        project: PathBuf,
        bin: PathBuf,
        log: PathBuf,
    }

    impl Setup {
        fn new() -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path().to_path_buf();
            let (project, bin, log) = (root.join("project"), root.join("bin"), root.join("log"));
            std::fs::create_dir_all(&project).unwrap();
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::write(&log, "").unwrap();
            let logged = format!(
                r#"echo "$(basename "$0") in $(basename "$PWD"): $*" >> "{}""#,
                log.display()
            );
            let npm = r#"
echo "env A=$A" >> "$LOG"
[ -n "$FAKE_FAIL" ] && exit 3
case "$*" in
  *"android init"*)
    mkdir -p gen/android/app
    printf 'plugins {\n}\n\nandroid {\n    compileSdk = 34\n    buildTypes {\n        getByName("release") {\n            isMinifyEnabled = true\n        }\n    }\n}\n' > gen/android/app/build.gradle.kts ;;
  *"ios init"*)
    mkdir -p gen/apple
    printf 'settings:\n  base:\n        ENABLE_BITCODE: false\n' > gen/apple/project.yml ;;
  *"android build"*)
    mkdir -p gen/android/app/build/outputs/apk/universal/release
    echo apk > gen/android/app/build/outputs/apk/universal/release/app.apk ;;
esac"#;
            script(
                &bin,
                "npm",
                &format!("LOG=\"{}\"\n{logged}\n{npm}", log.display()),
            );
            script(&bin, "xcodegen", &logged);
            let npx = r#"
[ -n "$FAKE_FAIL" ] && exit 4
dir="$CARGO_TARGET_DIR/release/bundle/deb"
mkdir -p "$dir/counter_0.1.0_amd64"
echo deb > "$dir/counter_0.1.0_amd64.deb"
echo noise > "$dir/notes.txt""#;
            script(&bin, "npx", &format!("{logged}\n{npx}"));
            Self {
                _tmp: tmp,
                root,
                project,
                bin,
                log,
            }
        }

        fn run(&self, args: &[&str]) -> Output {
            pack(&self.root, Some(&self.bin), args)
        }

        fn log(&self) -> String {
            std::fs::read_to_string(&self.log).unwrap()
        }

        fn config(&self) -> String {
            let path = self.root.join("tauri.json");
            std::fs::write(&path, "{}").unwrap();
            path.display().to_string()
        }
    }

    #[test]
    fn init_android_generates_the_project_and_signs_release_builds() {
        let s = Setup::new();
        let (project, config) = (s.project.display().to_string(), s.config());
        let output = s.run(&[
            "init",
            "android",
            "--project",
            &project,
            "--config",
            &config,
            "--signing-env",
            "EX",
            "--env",
            "A=1",
        ]);
        assert!(output.status.success(), "{}", stderr(&output));
        let gradle =
            std::fs::read_to_string(s.project.join("gen/android/app/build.gradle.kts")).unwrap();
        assert!(gradle.contains("useLegacyPackaging = true"), "{gradle}");
        assert!(
            gradle.contains(r#"System.getenv("EX_KEYSTORE")"#),
            "{gradle}"
        );
        let log = s.log();
        assert!(
            log.contains("npm in project: install --no-audit --no-fund"),
            "{log}"
        );
        assert!(
            log.contains("run tauri -- android init --ci --config"),
            "{log}"
        );
        assert!(log.contains("env A=1"), "the --env reaches the tool: {log}");
    }

    #[test]
    fn init_ios_patches_the_project_and_regenerates_it_only_with_linker_flags() {
        let s = Setup::new();
        let (project, config) = (s.project.display().to_string(), s.config());
        let plain = s.run(&["init", "ios", "--project", &project, "--config", &config]);
        assert!(plain.status.success(), "{}", stderr(&plain));
        assert!(!s.log().contains("xcodegen"));

        let flagged = s.run(&[
            "init",
            "ios",
            "--project",
            &project,
            "--config",
            &config,
            "--ldflags",
            "-lc++",
        ]);
        assert!(flagged.status.success(), "{}", stderr(&flagged));
        let yml = std::fs::read_to_string(s.project.join("gen/apple/project.yml")).unwrap();
        assert!(yml.contains("OTHER_LDFLAGS: $(inherited) -lc++"), "{yml}");
        assert!(
            s.log().contains("xcodegen in apple: generate"),
            "{}",
            s.log()
        );
    }

    #[test]
    fn build_android_copies_its_product_and_passes_the_rest_to_the_tauri_cli() {
        let s = Setup::new();
        let (project, config) = (s.project.display().to_string(), s.config());
        let to = s.root.join("dist/counter.apk");
        let product = format!("apk={}", to.display());
        let output = s.run(&[
            "build",
            "android",
            "--project",
            &project,
            "--config",
            &config,
            "--product",
            &product,
            "--",
            "--target",
            "aarch64",
        ]);
        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(std::fs::read(&to).unwrap(), b"apk\n");
        assert!(
            s.log().contains("android build --ci --config"),
            "{}",
            s.log()
        );
        assert!(s.log().contains("--target aarch64"), "{}", s.log());

        // A product the build did not make is an error saying which.
        let product = format!("aab={}", s.root.join("dist/x.aab").display());
        let output = s.run(&[
            "build",
            "android",
            "--project",
            &project,
            "--config",
            &config,
            "--product",
            &product,
        ]);
        assert_eq!(output.status.code(), Some(1));
        assert!(
            stderr(&output).contains("no .aab under"),
            "{}",
            stderr(&output)
        );
    }

    #[test]
    fn a_tool_that_fails_or_is_missing_fails_the_command() {
        let s = Setup::new();
        let (project, config) = (s.project.display().to_string(), s.config());
        let args = [
            "init",
            "android",
            "--project",
            &project,
            "--config",
            &config,
        ];
        let mut command = Command::new(env!("CARGO_BIN_EXE_levain-pack"));
        let old = std::env::var_os("PATH").unwrap_or_default();
        let path =
            std::env::join_paths(std::iter::once(s.bin.clone()).chain(std::env::split_paths(&old)))
                .unwrap();
        let failed = command
            .args(args)
            .env("PATH", path)
            .env("FAKE_FAIL", "1")
            .output()
            .unwrap();
        assert_eq!(failed.status.code(), Some(1));
        assert!(
            stderr(&failed).contains("npm exited with"),
            "{}",
            stderr(&failed)
        );

        // With no npm anywhere on PATH.
        let empty = s.root.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let missing = Command::new(env!("CARGO_BIN_EXE_levain-pack"))
            .args(args)
            .env("PATH", &empty)
            .output()
            .unwrap();
        assert_eq!(missing.status.code(), Some(1));
        assert!(stderr(&missing).contains("npm: "), "{}", stderr(&missing));
    }

    #[test]
    fn bundle_stages_the_host_runs_the_bundler_and_collects_the_bundles() {
        let s = Setup::new();
        let host = s.root.join("host-exe");
        std::fs::write(&host, "exe").unwrap();
        let (project, host_arg) = (s.project.display().to_string(), host.display().to_string());
        let (target, out) = (s.root.join("target"), s.root.join("out"));
        let (target_arg, out_arg) = (target.display().to_string(), out.display().to_string());
        let args = [
            "bundle",
            "--project",
            &project,
            "--host",
            &host_arg,
            "--host-name",
            "example_host",
            "--name",
            "counter",
            "--version",
            "0.1.0",
            "--identifier",
            "dev.example.counter",
            "--bundles",
            "deb",
            "--target-dir",
            &target_arg,
            "--out",
            &out_arg,
            "--resource",
            "/rel/lib/=release/lib/",
            "--config",
            "sign.json",
        ];
        let output = s.run(&args);
        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(
            std::fs::read(out.join("deb/counter_0.1.0_amd64.deb")).unwrap(),
            b"deb\n"
        );
        assert!(!out.join("deb/notes.txt").exists());
        let config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(target.join("tauri.bundle.json")).unwrap())
                .unwrap();
        assert_eq!(config["productName"], "counter");
        assert_eq!(config["bundle"]["resources"]["/rel/lib/"], "release/lib/");
        assert!(
            s.log()
                .contains("npx in project: --yes @tauri-apps/cli@2 bundle"),
            "{}",
            s.log()
        );
        assert!(
            s.log().contains("--config sign.json --bundles deb"),
            "{}",
            s.log()
        );
        assert!(stderr(&output).contains("bundled"), "{}", stderr(&output));
    }
}
