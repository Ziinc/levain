//! Precompiled assets (hosts, runtimes): the desktop target they are built
//! for, and downloading one checked against a pinned SHA-256.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The desktop target precompiled assets are named for: `<cpu>-<os>`, as in
/// `x86_64-linux`, `aarch64-macos` or `x86_64-windows`.
pub fn desktop_target(arch: &str, os: &str) -> Result<String, String> {
    let cpu = match arch {
        "aarch64" | "arm64" => "aarch64",
        "x86_64" => "x86_64",
        other => return Err(format!("no precompiled assets for {other}")),
    };
    let os = match os {
        "linux" | "macos" | "windows" => os,
        other => return Err(format!("no precompiled assets for {other}")),
    };
    Ok(format!("{cpu}-{os}"))
}

/// A `sha256sum` file (`<sha>  <file>` or `<sha> *<file>` per line, `#`
/// comments) as file name to checksum.
pub fn parse_sha256s(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let (sha, file) = line.split_once(char::is_whitespace)?;
            let file = file.trim_start().trim_start_matches('*');
            Some((file.to_owned(), sha.to_lowercase()))
        })
        .collect()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Unpacks `body`, a `.tar.gz` that must match `sha256`, into `dir`, unless
/// `dir` is already there. Unpacks beside it first, so an interrupted run
/// leaves nothing behind.
pub fn unpack_checked(body: &[u8], sha256: &str, dir: &Path) -> Result<(), String> {
    if dir.is_dir() {
        return Ok(());
    }
    if sha256_hex(body) != sha256.to_lowercase() {
        return Err(format!(
            "{} does not match its pinned SHA-256",
            dir.display()
        ));
    }
    let partial = PathBuf::from(format!("{}.partial", dir.display()));
    let _ = std::fs::remove_dir_all(&partial);
    std::fs::create_dir_all(&partial).map_err(|e| e.to_string())?;
    tar::Archive::new(flate2::read::GzDecoder::new(body))
        .unpack(&partial)
        .map_err(|e| format!("unpacking into {}: {e}", partial.display()))?;
    std::fs::rename(&partial, dir).map_err(|e| e.to_string())
}

/// Downloads `<base>/<asset>` into `<cache>/<asset without .tar.gz>`, checked
/// against its line in `sums`, once, and returns that directory.
pub fn fetch(base: &str, asset: &str, sums: &Path, cache: &Path) -> Result<PathBuf, String> {
    let sums = std::fs::read_to_string(sums).map_err(|e| format!("{}: {e}", sums.display()))?;
    let sha256 = parse_sha256s(&sums)
        .remove(asset)
        .ok_or_else(|| format!("no pinned checksum for {asset}"))?;
    let dir = cache.join(asset.strip_suffix(".tar.gz").unwrap_or(asset));
    if dir.is_dir() {
        return Ok(dir);
    }
    std::fs::create_dir_all(cache).map_err(|e| e.to_string())?;
    let url = format!("{}/{asset}", base.trim_end_matches('/'));
    eprintln!("* downloading {url}");
    let mut body = Vec::new();
    ureq::get(&url)
        .call()
        .map_err(|e| format!("GET {url}: {e}"))?
        .into_reader()
        .read_to_end(&mut body)
        .map_err(|e| format!("GET {url}: {e}"))?;
    unpack_checked(&body, &sha256, &dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_are_cpu_and_os() {
        assert_eq!(desktop_target("x86_64", "linux").unwrap(), "x86_64-linux");
        assert_eq!(desktop_target("arm64", "macos").unwrap(), "aarch64-macos");
        assert_eq!(
            desktop_target("aarch64", "windows").unwrap(),
            "aarch64-windows"
        );
        assert!(desktop_target("riscv64", "linux").is_err());
        assert!(desktop_target("x86_64", "freebsd").is_err());
    }

    #[test]
    fn reads_sha256sum_output() {
        let sums = parse_sha256s(
            "# hosts\nABC  baguette_host-v1-x86_64-linux.tar.gz\ndef *liberlang.tar.gz\n\n",
        );
        assert_eq!(sums["baguette_host-v1-x86_64-linux.tar.gz"], "abc");
        assert_eq!(sums["liberlang.tar.gz"], "def");
        assert_eq!(sums.len(), 2);
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

    #[test]
    fn unpacks_only_a_matching_archive() {
        let cache = tempfile::tempdir().unwrap();
        let dir = cache.path().join("host");
        let body = tar_gz("host", b"exe");

        let error = unpack_checked(&body, "00", &dir).unwrap_err();
        assert!(error.contains("SHA-256"), "{error}");
        assert!(!dir.exists());

        unpack_checked(&body, &sha256_hex(&body).to_uppercase(), &dir).unwrap();
        assert_eq!(std::fs::read(dir.join("host")).unwrap(), b"exe");
        // Already there: not checked or unpacked again.
        unpack_checked(b"", "00", &dir).unwrap();
    }

    #[test]
    fn an_asset_without_a_checksum_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let sums = tmp.path().join("sums");
        std::fs::write(&sums, "abc  other.tar.gz\n").unwrap();
        let error = fetch("https://example.invalid", "x.tar.gz", &sums, tmp.path());
        assert_eq!(error, Err("no pinned checksum for x.tar.gz".into()));
    }
}
