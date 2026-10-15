//! Downloads what Null installs itself (Node.js for language servers that need it, servers
//! that come as programs of their own) into its own folder, never the system's. Every file
//! is checked against the SHA-256 Null was built with before anything is unpacked: a file
//! that isn't exactly the one expected is thrown away.

use std::io::Read;
use std::path::{Path, PathBuf};

/// The systems Null runs on, as downloads name them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    MacArm,
    MacX64,
    LinuxArm,
    LinuxX64,
    WindowsArm,
    WindowsX64,
}

impl Target {
    /// This computer's.
    pub fn here() -> Option<Target> {
        let arm = cfg!(target_arch = "aarch64");
        let x64 = cfg!(target_arch = "x86_64");
        Some(match () {
            _ if cfg!(target_os = "macos") && arm => Target::MacArm,
            _ if cfg!(target_os = "macos") && x64 => Target::MacX64,
            _ if cfg!(target_os = "linux") && arm => Target::LinuxArm,
            _ if cfg!(target_os = "linux") && x64 => Target::LinuxX64,
            _ if cfg!(windows) && arm => Target::WindowsArm,
            _ if cfg!(windows) && x64 => Target::WindowsX64,
            _ => return None,
        })
    }
}

/// How a download is packed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Packed {
    /// The program itself.
    Plain,
    /// The program, gzipped.
    Gz,
    TarGz,
    Zip,
}

/// One file to download, for one system.
#[derive(Debug, PartialEq, Eq)]
pub struct Asset {
    pub target: Target,
    pub url: &'static str,
    /// Its SHA-256, in hex, as published by its makers when Null was built.
    pub sha256: &'static str,
    pub packed: Packed,
}

/// The largest download Null takes (a server with its libraries is ~100 MB).
const MAX_BYTES: u64 = 400 * 1024 * 1024;

/// The asset for this computer, if there's one.
pub fn for_here(assets: &'static [Asset]) -> Option<&'static Asset> {
    let here = Target::here()?;
    assets.iter().find(|a| a.target == here)
}

/// Downloads `asset` and unpacks it into `dir` (a program alone is written as `name`),
/// replacing what's there. Only once its SHA-256 is the one expected. Slow: off the main thread.
pub fn install(asset: &Asset, dir: &Path, name: &str) -> Result<(), String> {
    let bytes = fetch(asset.url)?;
    verify(&bytes, asset.sha256).map_err(|e| format!("{e} ({})", asset.url))?;
    // Unpacked beside, then put in place: a failure halfway leaves nothing half-made.
    let parent = dir.parent().ok_or("There's no folder to install into.")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("Couldn't create {}: {e}", parent.display()))?;
    let staging = parent.join(format!(".{}-unpacking", dir.file_name().and_then(|n| n.to_str()).unwrap_or("download")));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| format!("Couldn't create {}: {e}", staging.display()))?;
    let unpacked = unpack(&bytes, asset.packed, &staging, name);
    if let Err(e) = unpacked {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    let _ = std::fs::remove_dir_all(dir);
    std::fs::rename(&staging, dir).map_err(|e| format!("Couldn't move into {}: {e}", dir.display()))
}

fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(std::time::Duration::from_secs(20)))
        .timeout_recv_body(Some(std::time::Duration::from_secs(600)))
        .build()
        .into();
    let response = agent.get(url).call().map_err(|e| format!("Couldn't download {url}: {e}"))?;
    let mut bytes = Vec::new();
    response
        .into_body()
        .into_reader()
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("The download of {url} stopped: {e}"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(format!("{url} is larger than expected."));
    }
    Ok(bytes)
}

/// Whether `bytes` are exactly the file expected.
pub fn verify(bytes: &[u8], sha256: &str) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let got: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
    if got.eq_ignore_ascii_case(sha256) {
        Ok(())
    } else {
        Err("The download isn't the expected file (its checksum differs), so it wasn't installed".into())
    }
}

/// Unpacks into `dir`; a program alone is written as `name`. Nothing is written outside `dir`.
pub fn unpack(bytes: &[u8], packed: Packed, dir: &Path, name: &str) -> Result<(), String> {
    let write_program = |data: &[u8]| -> Result<(), String> {
        let path = dir.join(name);
        std::fs::write(&path, data).map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
        make_executable(&path)
    };
    match packed {
        Packed::Plain => write_program(bytes),
        Packed::Gz => {
            let mut data = Vec::new();
            flate2::read::GzDecoder::new(bytes)
                .take(MAX_BYTES)
                .read_to_end(&mut data)
                .map_err(|e| format!("Couldn't unpack the download: {e}"))?;
            write_program(&data)
        }
        Packed::TarGz => {
            let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
            archive.set_preserve_permissions(true);
            for entry in archive.entries().map_err(|e| format!("Couldn't unpack the download: {e}"))? {
                let mut entry = entry.map_err(|e| format!("Couldn't unpack the download: {e}"))?;
                // (Refuses paths that would leave `dir`.)
                entry.unpack_in(dir).map_err(|e| format!("Couldn't unpack the download: {e}"))?;
            }
            Ok(())
        }
        Packed::Zip => {
            let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
                .map_err(|e| format!("Couldn't unpack the download: {e}"))?;
            for i in 0..archive.len() {
                let mut file = archive.by_index(i).map_err(|e| format!("Couldn't unpack the download: {e}"))?;
                // A path that would leave `dir` is no file of ours.
                let Some(relative) = file.enclosed_name() else { continue };
                let path = dir.join(relative);
                if file.is_dir() {
                    std::fs::create_dir_all(&path).map_err(|e| format!("Couldn't create {}: {e}", path.display()))?;
                    continue;
                }
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("Couldn't create {}: {e}", parent.display()))?;
                }
                let mut out =
                    std::fs::File::create(&path).map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
                std::io::copy(&mut file, &mut out).map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
                #[cfg(unix)]
                if let Some(mode) = file.unix_mode() {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode & 0o777));
                }
            }
            Ok(())
        }
    }
}

fn make_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("Couldn't make {} runnable: {e}", path.display()))?;
    }
    let _ = path;
    Ok(())
}

/// A program's file name here: `.exe` on Windows.
pub fn exe(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
}

/// The folder Null keeps `what` in (its own, never the system's).
pub fn home_of(what: &str) -> Option<PathBuf> {
    Some(crate::tools::data_dir()?.join(what))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn sha(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn only_the_expected_file_is_accepted() {
        let file = b"a language server";
        assert!(verify(file, &sha(file)).is_ok());
        assert!(verify(file, &sha(file).to_uppercase()).is_ok());
        assert!(verify(b"a language server, changed", &sha(file)).is_err());
    }

    #[test]
    fn unpacks_each_kind_inside_its_folder_only() {
        let dir = crate::tools::test_dir("download-unpack");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A program gzipped.
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(b"#!/bin/sh\necho hi\n").unwrap();
        unpack(&gz.finish().unwrap(), Packed::Gz, &dir, "server").unwrap();
        assert_eq!(std::fs::read(dir.join("server")).unwrap(), b"#!/bin/sh\necho hi\n");
        // A zip with a folder, and an entry trying to climb out.
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("pkg/bin/tool", options).unwrap();
        zip.write_all(b"tool").unwrap();
        zip.start_file("../outside", options).unwrap();
        zip.write_all(b"no").unwrap();
        let zipped = zip.finish().unwrap().into_inner();
        unpack(&zipped, Packed::Zip, &dir, "unused").unwrap();
        assert_eq!(std::fs::read(dir.join("pkg/bin/tool")).unwrap(), b"tool");
        assert!(!dir.parent().unwrap().join("outside").exists());
        // A tarball.
        let mut tarball = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
        let mut header = tar::Header::new_gnu();
        header.set_size(3);
        header.set_mode(0o755);
        header.set_cksum();
        tarball.append_data(&mut header, "node/bin/node", &b"bin"[..]).unwrap();
        let tgz = tarball.into_inner().unwrap().finish().unwrap();
        unpack(&tgz, Packed::TarGz, &dir, "unused").unwrap();
        assert_eq!(std::fs::read(dir.join("node/bin/node")).unwrap(), b"bin");
    }
}
