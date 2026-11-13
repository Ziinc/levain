//! Android and iOS: the app ships inside the host library
//! (`LEVAIN_APP_ARCHIVE`, a `.tar.gz` embedded at build time) and is
//! unpacked into the app's data on first launch.

use std::path::{Path, PathBuf};

#[cfg(levain_embedded_app)]
static ARCHIVE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/app.tar.gz"));
#[cfg(not(levain_embedded_app))]
static ARCHIVE: &[u8] = &[];

/// Unpacks the embedded app under `data_dir`, once per app build, and
/// returns its root. `prepare` runs on the unpacked files before they are
/// moved into place (Android: linking native libraries).
pub fn unpack(
    data_dir: &Path,
    prepare: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<PathBuf, String> {
    if ARCHIVE.is_empty() {
        return Err("this app was built without an embedded app (LEVAIN_APP_ARCHIVE)".into());
    }
    let root = unpack_archive(ARCHIVE, env!("LEVAIN_APP_HASH"), data_dir, prepare)?;
    crate::log_line(&format!("levain: app at {}", root.display()));
    Ok(root)
}

/// The marker a completed unpack leaves in the app's root.
const UNPACKED: &str = ".levain-unpacked";

pub(crate) fn unpack_archive(
    archive: &[u8],
    hash: &str,
    data_dir: &Path,
    prepare: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<PathBuf, String> {
    let root = data_dir.join(format!("release-{hash}"));
    if root.join(UNPACKED).is_file() {
        return Ok(root);
    }
    let staging = data_dir.join("release-unpacking");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    tar::Archive::new(flate2::read::GzDecoder::new(archive))
        .unpack(&staging)
        .map_err(|e| format!("unpacking the app: {e}"))?;
    prepare(&staging)?;
    std::fs::write(staging.join(UNPACKED), hash).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&root);
    std::fs::rename(&staging, &root).map_err(|e| e.to_string())?;
    // Earlier builds of this app.
    for entry in std::fs::read_dir(data_dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("release-") && path != root {
            let _ = std::fs::remove_dir_all(path);
        }
    }
    Ok(root)
}

/// The directory Android installed the host library in.
#[cfg(target_os = "android")]
pub fn native_lib_dir() -> Result<PathBuf, String> {
    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    // SAFETY: dladdr only reads the address and fills `info`.
    let found = unsafe { libc::dladdr(native_lib_dir as *const libc::c_void, &mut info) };
    if found == 0 || info.dli_fname.is_null() {
        return Err("cannot find the app's native library directory".into());
    }
    // SAFETY: dladdr returned a NUL-terminated path.
    let path = unsafe { std::ffi::CStr::from_ptr(info.dli_fname) };
    Path::new(&*path.to_string_lossy())
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "the app's library has no directory".into())
}

/// Sends the process's stdout and stderr to logcat under `tag`; Android
/// discards them otherwise.
#[cfg(target_os = "android")]
pub fn log_stdio(tag: &'static str) {
    let mut fds = [0; 2];
    // SAFETY: pipe fills two descriptors; dup2 points stdout and stderr at
    // its write end for the life of the process.
    unsafe {
        if libc::pipe(fds.as_mut_ptr()) != 0 {
            return;
        }
        libc::dup2(fds[1], 1);
        libc::dup2(fds[1], 2);
    }
    let read = fds[0];
    let _ = std::thread::Builder::new()
        .name("stdio-log".into())
        .spawn(move || {
            use std::io::BufRead;
            use std::os::fd::FromRawFd;
            // SAFETY: the read end belongs to this thread alone.
            let file = unsafe { std::fs::File::from_raw_fd(read) };
            for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
                log::info!(target: tag, "{line}");
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive(files: &[(&str, &str)]) -> Vec<u8> {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        for (path, body) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, path, body.as_bytes()).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn unpacks_once_per_build_and_drops_earlier_ones() {
        let data = tempfile::tempdir().unwrap();
        let old = data.path().join("release-old");
        std::fs::create_dir_all(&old).unwrap();

        let bytes = archive(&[("app.json", "{}")]);
        let mut prepared = 0;
        let root = unpack_archive(&bytes, "abc", data.path(), |dir| {
            prepared += 1;
            assert!(dir.join("app.json").is_file());
            Ok(())
        })
        .unwrap();
        assert_eq!(root, data.path().join("release-abc"));
        assert!(root.join("app.json").is_file());
        assert!(!old.exists());

        // The same build again: nothing to do.
        unpack_archive(&bytes, "abc", data.path(), |_| {
            prepared += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(prepared, 1);
    }

    #[test]
    fn a_failed_prepare_leaves_no_app() {
        let data = tempfile::tempdir().unwrap();
        let bytes = archive(&[("app.json", "{}")]);
        let error = unpack_archive(&bytes, "abc", data.path(), |_| Err("no".into()));
        assert_eq!(error, Err("no".into()));
        assert!(!data.path().join("release-abc").exists());
    }
}
