//! Node.js for the language servers that are npm packages (Python's, TypeScript's, HTML's…)
//! on a computer without it: the official build, downloaded into Null's own folder and
//! checked against the checksum nodejs.org published, used only for those servers. Nothing
//! is installed on the system; a Node.js already there is used instead.

use crate::download::{Asset, Packed, Target};
use std::path::PathBuf;

/// The long-term support release Null uses (Node 24, "Krypton").
pub const VERSION: &str = "v24.22.0";

const ASSETS: &[Asset] = &[
    Asset {
        target: Target::MacArm,
        url: "https://nodejs.org/dist/v24.22.0/node-v24.22.0-darwin-arm64.tar.gz",
        sha256: "6a9724d9b50a5f22b4816cded441b5a8d9ddae546191a5e5b7efa5a5e2ee26d5",
        packed: Packed::TarGz,
    },
    Asset {
        target: Target::MacX64,
        url: "https://nodejs.org/dist/v24.22.0/node-v24.22.0-darwin-x64.tar.gz",
        sha256: "495e3f273c645f3c4677d438304cdfc701729ea5dec5728f5c8e76a2dece4b91",
        packed: Packed::TarGz,
    },
    Asset {
        target: Target::LinuxArm,
        url: "https://nodejs.org/dist/v24.22.0/node-v24.22.0-linux-arm64.tar.gz",
        sha256: "b7e7991d5407815adfa74a2a9cd4db683552c8ce32ff4a295c22e972849c3f5d",
        packed: Packed::TarGz,
    },
    Asset {
        target: Target::LinuxX64,
        url: "https://nodejs.org/dist/v24.22.0/node-v24.22.0-linux-x64.tar.gz",
        sha256: "28ea0a438de9a13bd7891fd758eee6c7819ee8af5442dfd8d45bd65e56d4228c",
        packed: Packed::TarGz,
    },
    Asset {
        target: Target::WindowsArm,
        url: "https://nodejs.org/dist/v24.22.0/node-v24.22.0-win-arm64.zip",
        sha256: "3a5c209a5bc59e65b3bc0fa13ed5e166a26bd3d55332847f498b76b037408d61",
        packed: Packed::Zip,
    },
    Asset {
        target: Target::WindowsX64,
        url: "https://nodejs.org/dist/v24.22.0/node-v24.22.0-win-x64.zip",
        sha256: "f9683a19877e12a9b167305c8db71040d812876359de21f0d2e97c3f321f80c3",
        packed: Packed::Zip,
    },
];

/// The name of the folder the download unpacks to (`node-v24.22.0-darwin-arm64`).
fn unpacked_name(target: Target) -> String {
    let platform = match target {
        Target::MacArm => "darwin-arm64",
        Target::MacX64 => "darwin-x64",
        Target::LinuxArm => "linux-arm64",
        Target::LinuxX64 => "linux-x64",
        Target::WindowsArm => "win-arm64",
        Target::WindowsX64 => "win-x64",
    };
    format!("node-{VERSION}-{platform}")
}

/// Where Null's own Node.js is (or would be).
fn folder() -> Option<PathBuf> {
    Some(crate::download::home_of("node")?.join(VERSION))
}

/// The folder with Null's own `node` and `npm` in it (whether or not it's there yet), for
/// the PATH of what Null starts.
pub fn bin_dir() -> Option<PathBuf> {
    let root = folder()?.join(unpacked_name(Target::here()?));
    Some(if cfg!(windows) { root } else { root.join("bin") })
}

/// The Linux build for a WSL distribution (Arm or x64, as Windows is), to unpack there.
pub fn linux_build() -> &'static Asset {
    let target = if cfg!(target_arch = "aarch64") { Target::LinuxArm } else { Target::LinuxX64 };
    ASSETS.iter().find(|a| a.target == target).expect("a Linux build")
}

/// Whether Null can bring its own Node.js to this computer.
pub fn available() -> bool {
    crate::download::for_here(ASSETS).is_some()
}

/// npm: the computer's, else Null's own, downloaded the first time it's needed. Slow the
/// first time: off the main thread.
pub fn npm() -> Result<PathBuf, String> {
    if let Some(npm) = crate::tools::find("npm") {
        return Ok(npm);
    }
    bring()?;
    crate::tools::find("npm").ok_or_else(|| "Node.js was downloaded, but its npm can't be found.".into())
}

/// Downloads Null's own Node.js.
fn bring() -> Result<(), String> {
    let asset = crate::download::for_here(ASSETS)
        .ok_or("Node.js isn't installed, and there's no build of it for this computer: get it from nodejs.org.")?;
    let folder = folder().ok_or("There's no folder to install into.")?;
    crate::download::install(asset, &folder, "node")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_system_has_its_build_and_folder() {
        for target in
            [Target::MacArm, Target::MacX64, Target::LinuxArm, Target::LinuxX64, Target::WindowsArm, Target::WindowsX64]
        {
            let asset = ASSETS.iter().find(|a| a.target == target).expect("a build for every system");
            assert!(asset.url.contains(&unpacked_name(target)), "{}", asset.url);
            assert!(asset.url.starts_with("https://nodejs.org/dist/v24.22.0/"));
            assert_eq!(asset.sha256.len(), 64);
        }
        assert!(available());
    }

    /// Downloads it for real (50 MB): `cargo test brings_node -- --ignored`.
    #[test]
    #[ignore]
    fn brings_node_and_its_npm() {
        bring().unwrap();
        let bin = bin_dir().unwrap();
        let node = bin.join(crate::download::exe("node"));
        let version = crate::tools::command(&node).arg("--version").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&version.stdout).trim(), VERSION);
        let npm = bin.join(if cfg!(windows) { "npm.cmd" } else { "npm" });
        let npm_version = crate::tools::command(&npm)
            .arg("--version")
            .env("PATH", std::env::join_paths([bin.clone()]).unwrap())
            .output()
            .unwrap();
        assert!(npm_version.status.success(), "{}", String::from_utf8_lossy(&npm_version.stderr));
    }
}
