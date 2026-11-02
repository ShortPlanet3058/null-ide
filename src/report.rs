//! What to send when something's wrong: Null's version and how it's installed, the
//! system, and on Linux what decides whether it can draw (the session, the graphics cards,
//! their Vulkan drivers), with the last crash. `null --report` prints it (it works when no
//! window opens at all); Copy Problem Report copies it. Nothing of yours is in it: no
//! files, settings or keys, and your home folder is `~`.

/// The report, as text to paste into an issue.
pub fn text() -> String {
    let mut lines = vec![format!("Null {} · {}", env!("CARGO_PKG_VERSION"), installed_as())];
    lines.push(format!("System: {} · {}", system(), std::env::consts::ARCH));
    #[cfg(all(unix, not(target_os = "macos")))]
    lines.extend(linux::lines());
    if let Some(why) = crate::drawing::drawn_without_card() {
        lines.push(format!("Drawn without the graphics card: {why}"));
    }
    if let Some(crash) = last_crash() {
        lines.push(String::new());
        lines.push(format!("Last crash:\n{crash}"));
    }
    private(&lines.join("\n"))
}

fn installed_as() -> &'static str {
    use crate::updates::Install;
    match crate::updates::how() {
        Some(Install::Setup(_)) => "Windows installer",
        Some(Install::App(_)) => "Mac app",
        Some(Install::Binary(_)) => ".tar.gz",
        Some(Install::Deb(_)) => ".deb",
        Some(Install::AppImage(_)) => "AppImage",
        None => "doesn't update itself (the .zip, a build, or a folder it can't write to)",
    }
}

/// The system's name and version.
fn system() -> String {
    let said = |program: &str, args: &[&str]| -> Option<String> {
        let output = crate::tools::command(program).args(args).output().ok()?;
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        (output.status.success() && !text.is_empty()).then_some(text)
    };
    if cfg!(target_os = "macos") {
        return format!("macOS {}", said("sw_vers", &["-productVersion"]).unwrap_or_default());
    }
    if cfg!(windows) {
        // ("Microsoft Windows [version 10.0.22631.4317]", in the system's language.)
        return said("cmd", &["/c", "ver"]).unwrap_or_else(|| "Windows".into());
    }
    let name = std::fs::read_to_string("/etc/os-release").ok().and_then(|text| {
        text.lines()
            .find_map(|l| l.strip_prefix("PRETTY_NAME="))
            .map(|v| v.trim_matches('"').to_string())
    });
    let kernel = said("uname", &["-r"]).map(|k| format!(" · Linux {k}")).unwrap_or_default();
    format!("{}{kernel}", name.unwrap_or_else(|| "Linux".into()))
}

/// The last crash written down (see `crash`), its first lines.
fn last_crash() -> Option<String> {
    let log = std::fs::read_to_string(crate::tools::data_dir()?.join("crash.log")).ok()?;
    let last = log.rsplit("\n== ").next()?.trim();
    (!last.is_empty()).then(|| last.lines().take(30).collect::<Vec<_>>().join("\n"))
}

/// `text` with your home folder as `~`.
fn private(text: &str) -> String {
    match crate::tools::home_dir().map(|h| h.display().to_string()).filter(|h| h.len() > 1) {
        Some(home) => text.replace(&home, "~"),
        None => text.to_string(),
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod linux {
    use std::path::Path;

    pub fn lines() -> Vec<String> {
        let mut lines = Vec::new();
        #[cfg(target_env = "gnu")]
        {
            // SAFETY: glibc's own version string, static.
            let version = unsafe { std::ffi::CStr::from_ptr(libc::gnu_get_libc_version()) };
            lines.push(format!("C library: glibc {}", version.to_string_lossy()));
        }
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        lines.push(format!(
            "Session: {} ({}) · WAYLAND_DISPLAY {} · DISPLAY {}",
            var("XDG_SESSION_TYPE").unwrap_or_else(|| "?".into()),
            var("XDG_CURRENT_DESKTOP").unwrap_or_else(|| "?".into()),
            var("WAYLAND_DISPLAY").unwrap_or_else(|| "not set".into()),
            var("DISPLAY").unwrap_or_else(|| "not set".into()),
        ));
        lines.push(format!("Graphics cards: {}", cards()));
        if let Some(nvidia) = std::fs::read_to_string("/proc/driver/nvidia/version").ok()
            && let Some(first) = nvidia.lines().next()
        {
            lines.push(format!("NVIDIA driver: {}", first.trim()));
        }
        lines.push(format!("Vulkan drivers: {}", vulkan_drivers()));
        for name in ["VK_DRIVER_FILES", "VK_ICD_FILENAMES"] {
            if let Some(value) = var(name) {
                lines.push(format!("{name}: {value}"));
            }
        }
        lines
    }

    /// The graphics cards the kernel sees, by maker.
    fn cards() -> String {
        let mut found: Vec<String> = Vec::new();
        for entry in std::fs::read_dir("/sys/class/drm").into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            // (card0, card1: not their outputs, card0-HDMI-A-1.)
            if !name.starts_with("card") || name.contains('-') {
                continue;
            }
            let Ok(vendor) = std::fs::read_to_string(entry.path().join("device/vendor")) else { continue };
            let maker = match vendor.trim() {
                "0x10de" => "NVIDIA".to_string(),
                "0x1002" => "AMD".to_string(),
                "0x8086" => "Intel".to_string(),
                "0x15ad" => "VMware (virtual)".to_string(),
                "0x1af4" => "virtio (virtual)".to_string(),
                "0x1234" => "QEMU (virtual)".to_string(),
                other => other.to_string(),
            };
            let driver = std::fs::read_link(entry.path().join("device/driver"))
                .ok()
                .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()));
            found.push(match driver {
                Some(driver) => format!("{maker} ({driver})"),
                None => maker,
            });
        }
        if found.is_empty() { "none seen".into() } else { found.join(", ") }
    }

    /// The Vulkan drivers installed (their manifests' names), where the Vulkan loader looks.
    fn vulkan_drivers() -> String {
        let data = std::env::var("XDG_DATA_DIRS").ok().filter(|d| !d.is_empty());
        let data = data.as_deref().unwrap_or("/usr/local/share:/usr/share");
        let mut names: Vec<String> = data
            .split(':')
            .map(|d| Path::new(d).join("vulkan/icd.d"))
            .chain(["/etc/vulkan/icd.d".into()])
            .flat_map(|dir| std::fs::read_dir(dir).into_iter().flatten().flatten())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".json"))
            .collect();
        names.sort();
        names.dedup();
        if names.is_empty() { "none found".into() } else { names.join(", ") }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The report says the version and the system, and has your home folder as `~`.
    #[test]
    fn the_report_says_what_runs_where_and_nothing_of_yours() {
        let report = text();
        assert!(report.starts_with(&format!("Null {}", env!("CARGO_PKG_VERSION"))), "{report}");
        assert!(report.contains("System: "), "{report}");
        if let Some(home) = crate::tools::home_dir() {
            assert!(!report.contains(&home.display().to_string()), "{report}");
        }
        assert_eq!(private(&format!("{}/x", crate::tools::home_dir().unwrap().display())), "~/x");
    }
}
