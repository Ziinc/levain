//! levain-pack: packages an app around a levain host. Language packages
//! (baguette's `mix release`, chouquette, briochette) call it; it knows
//! nothing about the language.
//!
//! ```text
//! levain-pack target
//! levain-pack fetch <base-url> <asset> --sha256 <sums> --cache <dir>
//! levain-pack install <host> <dest>
//! levain-pack bundle --project <dir> --host <exe> --host-name <name> --name <app>
//!     --version <v> --identifier <id> --bundles <a,b> --target-dir <dir> --out <dir>
//!     [--resource <from>=<to>]... [--config <extra>]
//! levain-pack ios-crate --host-crate <name> --host-lib <lib name> --host-path <dir>
//!     --out <dir> [--feature <f>]... [--default-feature <f>]... [--name <crate>]
//!     [--product-name <n>] [--version <v>] [--identifier <id>]
//! levain-pack init <android|ios> --project <dir> --config <json>
//!     [--signing-env <PREFIX>] [--ldflags <flags>] [--env K=V]...
//! levain-pack build <android|ios> --project <dir> --config <json>
//!     [--product <ext>=<path>]... [--env K=V]... [-- <tauri args>]
//! ```

mod bundle;
mod fetch;
mod ios;
mod mobile;

use std::path::{Path, PathBuf};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(error) = run(&args) {
        eprintln!("levain-pack: {error}");
        std::process::exit(1);
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let (command, rest) = args.split_first().ok_or_else(usage)?;
    let mut opts = Opts::parse(rest)?;
    match command.as_str() {
        "target" => {
            let target = fetch::desktop_target(std::env::consts::ARCH, std::env::consts::OS)?;
            println!("{target}");
        }
        "fetch" => {
            let [base, asset] = opts.positional::<2>()?;
            let dir = fetch::fetch(&base, &asset, &opts.path("sha256")?, &opts.path("cache")?)?;
            println!("{}", dir.display());
        }
        "install" => {
            let [host, dest] = opts.positional::<2>()?;
            install(Path::new(&host), Path::new(&dest))?;
        }
        "bundle" => {
            let b = bundle::Bundle {
                project: opts.path("project")?,
                host: opts.path("host")?,
                host_name: opts.one("host-name")?,
                name: opts.one("name")?,
                version: opts.one("version")?,
                identifier: opts.one("identifier")?,
                resources: opts
                    .all("resource")
                    .into_iter()
                    .map(|r| split_pair(&r))
                    .collect::<Result<_, _>>()?,
                bundles: opts
                    .one("bundles")?
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect(),
                target_dir: opts.path("target-dir")?,
                out: opts.path("out")?,
                extra_config: opts.maybe("config"),
            };
            bundle::run(&b)?;
        }
        "init" | "build" => {
            let [platform] = opts.positional::<1>()?;
            let platform = mobile::Platform::parse(&platform)?;
            let project = opts.path("project")?;
            let config = opts.path("config")?;
            let env: Vec<(String, String)> = opts
                .all("env")
                .iter()
                .map(|kv| split_pair(kv))
                .collect::<Result<_, _>>()?;
            let env: Vec<(&str, String)> =
                env.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
            if command == "init" {
                let options = mobile::Init {
                    signing_env: opts.maybe("signing-env"),
                    ldflags: opts.maybe("ldflags"),
                };
                mobile::init(platform, &project, &config, &options, &env)?;
            } else {
                let products: Vec<(String, PathBuf)> = opts
                    .all("product")
                    .iter()
                    .map(|p| split_pair(p).map(|(ext, to)| (ext, PathBuf::from(to))))
                    .collect::<Result<_, _>>()?;
                mobile::build(platform, &project, &config, &opts.rest, &products, &env)?;
            }
        }
        "ios-crate" => {
            let mut spec = ios::IosCrate::new(
                &opts.one("host-crate")?,
                &opts.one("host-lib")?,
                &opts.one("host-path")?,
            );
            spec.features = opts.all("feature");
            spec.default_features = opts.all("default-feature");
            if let Some(name) = opts.maybe("name") {
                spec.name = name;
            }
            if let Some(name) = opts.maybe("product-name") {
                spec.product_name = name;
            }
            if let Some(version) = opts.maybe("version") {
                spec.version = version;
            }
            if let Some(id) = opts.maybe("identifier") {
                spec.identifier = id;
            }
            let out = opts.path("out")?;
            ios::write(&spec, &out)?;
        }
        _ => return Err(format!("unknown command {command}\n{}", usage())),
    }
    opts.finish()
}

/// Every subcommand `run` dispatches.
const COMMANDS: [&str; 7] = [
    "target",
    "fetch",
    "install",
    "bundle",
    "ios-crate",
    "init",
    "build",
];

fn usage() -> String {
    format!("usage: levain-pack <{}> ...", COMMANDS.join("|"))
}

/// Copies the host to `dest`, executable. Replaces rather than overwrites,
/// so a running copy doesn't block the build.
fn install(host: &Path, dest: &Path) -> Result<(), String> {
    let _ = std::fs::remove_file(dest);
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::copy(host, dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    eprintln!("* installed {}", dest.display());
    Ok(())
}

fn split_pair(pair: &str) -> Result<(String, String), String> {
    pair.split_once('=')
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
        .ok_or_else(|| format!("expected <a>=<b>, got {pair}"))
}

/// Runs `program` in `dir`, failing on a non-zero exit.
pub(crate) fn cmd(
    dir: &Path,
    program: &str,
    args: &[String],
    env: &[(&str, String)],
) -> Result<(), String> {
    eprintln!("* running {program} {}", args.join(" "));
    // npm and npx are scripts on Windows, which Command only finds by their
    // full name.
    let program = if cfg!(windows) && matches!(program, "npm" | "npx") {
        format!("{program}.cmd")
    } else {
        program.to_owned()
    };
    let status = std::process::Command::new(&program)
        .args(args)
        .current_dir(dir)
        .envs(env.iter().map(|(k, v)| (*k, v)))
        .status()
        .map_err(|e| format!("{program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} exited with {status}"))
    }
}

/// `--name value` options, positional arguments, and what follows `--`.
#[derive(Debug, Default)]
struct Opts {
    named: Vec<(String, String)>,
    positional: Vec<String>,
    rest: Vec<String>,
}

impl Opts {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut opts = Opts::default();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            if arg == "--" {
                opts.rest = iter.cloned().collect();
                break;
            }
            match arg.strip_prefix("--") {
                Some(name) => {
                    let value = iter
                        .next()
                        .ok_or_else(|| format!("--{name} needs a value"))?;
                    opts.named.push((name.to_owned(), value.clone()));
                }
                None => opts.positional.push(arg.clone()),
            }
        }
        Ok(opts)
    }

    fn all(&mut self, name: &str) -> Vec<String> {
        let (taken, kept) = std::mem::take(&mut self.named)
            .into_iter()
            .partition(|(n, _)| n == name);
        self.named = kept;
        taken.into_iter().map(|(_, v)| v).collect()
    }

    fn maybe(&mut self, name: &str) -> Option<String> {
        self.all(name).pop()
    }

    fn one(&mut self, name: &str) -> Result<String, String> {
        self.maybe(name)
            .ok_or_else(|| format!("--{name} is required"))
    }

    fn path(&mut self, name: &str) -> Result<PathBuf, String> {
        self.one(name).map(PathBuf::from)
    }

    fn positional<const N: usize>(&mut self) -> Result<[String; N], String> {
        let args = std::mem::take(&mut self.positional);
        args.try_into()
            .map_err(|args: Vec<String>| format!("expected {N} arguments, got {}", args.len()))
    }

    /// Fails on options nobody read.
    fn finish(self) -> Result<(), String> {
        match self.named.first() {
            Some((name, _)) => Err(format!("unknown option --{name}")),
            None if !self.positional.is_empty() => {
                Err(format!("unexpected argument {}", self.positional[0]))
            }
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn options_positionals_and_rest() {
        let mut opts = Opts::parse(&strings(&[
            "a", "--env", "X=1", "--env", "Y=2", "b", "--", "--x",
        ]))
        .unwrap();
        assert_eq!(opts.all("env"), ["X=1", "Y=2"]);
        assert_eq!(opts.positional::<2>().unwrap(), ["a", "b"]);
        assert_eq!(opts.rest, ["--x"]);
        assert!(opts.finish().is_ok());
    }

    #[test]
    fn unknown_and_missing_options_fail() {
        assert!(Opts::parse(&strings(&["--name"])).is_err());
        let mut opts = Opts::parse(&strings(&["--nope", "1"])).unwrap();
        assert_eq!(opts.one("name"), Err("--name is required".into()));
        assert_eq!(opts.finish(), Err("unknown option --nope".into()));
        assert!(run(&strings(&["target", "extra"])).is_err());
        assert!(run(&strings(&["frobnicate"])).is_err());
    }

    #[test]
    fn usage_docs_and_dispatch_agree() {
        let source = include_str!("main.rs");
        let doc: String = source
            .lines()
            .take_while(|l| l.starts_with("//!"))
            .collect::<Vec<_>>()
            .join("\n");
        for command in COMMANDS {
            assert!(usage().contains(command));
            // Dispatched: no arguments fail, but not as an unknown command.
            if command != "target" {
                let error = run(&[command.to_string()]).unwrap_err();
                assert!(!error.starts_with("unknown command"), "{command}: {error}");
            }
        }
        // And nothing the doc comment lists is missing from COMMANDS.
        for line in doc
            .lines()
            .filter_map(|l| l.strip_prefix("//! levain-pack "))
        {
            let command = line.split_whitespace().next().unwrap();
            assert!(COMMANDS.contains(&command), "{command} is not dispatched");
        }
        assert!(
            run(&strings(&["nope"]))
                .unwrap_err()
                .starts_with("unknown command")
        );
    }

    #[test]
    fn ios_crate_writes_the_wrapper() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("w");
        let args = strings(&[
            "ios-crate",
            "--host-crate",
            "h_host",
            "--host-lib",
            "h_lib",
            "--host-path",
            "../h_host",
            "--feature",
            "a",
            "--out",
            out.to_str().unwrap(),
        ]);
        run(&args).unwrap();
        let cargo = std::fs::read_to_string(out.join("Cargo.toml")).unwrap();
        assert!(cargo.contains("a = [\"h_host/a\"]"));
        assert!(run(&strings(&["ios-crate", "--out", "x"])).is_err());
    }

    #[test]
    fn install_replaces_the_host() {
        let tmp = tempfile::tempdir().unwrap();
        let host = tmp.path().join("host");
        let dest = tmp.path().join("bin/counter_app");
        std::fs::write(&host, "new").unwrap();
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, "old").unwrap();
        install(&host, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dest).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
    }

    #[test]
    fn pairs() {
        assert_eq!(split_pair("a=b=c").unwrap(), ("a".into(), "b=c".into()));
        assert!(split_pair("ab").is_err());
    }
}
