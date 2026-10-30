//! Whether Null can draw its windows. On Linux GPUI draws with the graphics card through
//! Vulkan, and panics when no card will ("Unable to init GPU context"): an NVIDIA card
//! whose driver is installed without its Vulkan part, or too old for what GPUI asks, or a
//! machine without any Vulkan. Then Null starts again drawn by Mesa's software Vulkan
//! (lavapipe), slower, and says why once its window is open; with no software one either,
//! it says what's wrong and how to put it right (in the terminal, and in a dialog when it
//! was started from the desktop), not a panic.

use gpui::Application;

/// Set on a Null started again without the card: why the card didn't draw.
#[cfg(all(unix, not(target_os = "macos")))]
const WITHOUT_CARD: &str = "NULL_DRAWN_WITHOUT_CARD";

/// The app, its windows drawn by the graphics card, or else as said above.
pub fn application() -> Application {
    #[cfg(all(unix, not(target_os = "macos")))]
    return linux::application();
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    Application::new()
}

/// Why this Null draws without the graphics card, to say in its window: once a run.
pub fn without_card() -> Option<String> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        static SAID: AtomicBool = AtomicBool::new(false);
        let why = linux::WHY.get()?.clone();
        return (!SAID.swap(true, Ordering::Relaxed))
            .then(|| format!("Drawn without the graphics card, so slower: {why}. {}", linux::REMEDY));
    }
    #[allow(unreachable_code)]
    None
}

#[cfg(all(unix, not(target_os = "macos")))]
mod linux {
    use super::WITHOUT_CARD;
    use gpui::Application;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};

    pub const REMEDY: &str = "For an NVIDIA card: Software & Updates → Additional Drivers, the NVIDIA driver \
        (proprietary), then restart the computer (on Ubuntu, `sudo ubuntu-drivers install` does the same)";

    /// Why this run draws without the card (set when started again so).
    pub static WHY: OnceLock<String> = OnceLock::new();

    /// What Vulkan said while the card was tried (GPUI's Blade says why it turns a card
    /// down in its log, and Null keeps no log otherwise).
    static SAID: Mutex<Vec<String>> = Mutex::new(Vec::new());

    struct Listening;

    impl log::Log for Listening {
        fn enabled(&self, metadata: &log::Metadata) -> bool {
            metadata.level() <= log::Level::Warn
        }
        fn log(&self, record: &log::Record) {
            if self.enabled(record.metadata())
                && let Ok(mut said) = SAID.lock()
                && said.len() < 20
            {
                said.push(record.args().to_string().trim().to_string());
            }
        }
        fn flush(&self) {}
    }

    pub fn application() -> Application {
        let started_again = std::env::var(WITHOUT_CARD).ok().filter(|w| !w.is_empty());
        static LISTENING: Listening = Listening;
        let listening = log::set_logger(&LISTENING).is_ok();
        if listening {
            log::set_max_level(log::LevelFilter::Warn);
        }
        // (Quiet meanwhile: GPUI's panic is answered here, not printed or kept as a crash.)
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let made = std::panic::catch_unwind(Application::new);
        std::panic::set_hook(hook);
        if listening {
            log::set_max_level(log::LevelFilter::Off);
        }
        let panic = match made {
            Ok(app) => {
                // A card turned down, yet drawn: by another, Mesa's software one most likely.
                let said = SAID.lock().map(|s| s.clone()).unwrap_or_default();
                if started_again.is_none() && said.iter().any(|s| s.trim_start().starts_with("Rejected for")) {
                    WHY.set(why("", &said)).ok();
                }
                if let Some(why) = started_again {
                    WHY.set(why).ok();
                    // (Vulkan was read as GPUI started: what this Null starts, a game in its
                    // terminal, gets the card again.)
                    // SAFETY: nothing else reads the environment as this is changed: the
                    // only other thread then reads no variables.
                    unsafe {
                        std::env::remove_var("VK_DRIVER_FILES");
                        std::env::remove_var("VK_ICD_FILENAMES");
                        std::env::remove_var(WITHOUT_CARD);
                    }
                }
                return app;
            }
            Err(panic) => panic,
        };
        let message = panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default();
        let said = SAID.lock().map(|s| s.clone()).unwrap_or_default();
        if !message.contains("GPU") {
            // Not the card (no display to reach): as GPUI says it.
            end(&format!("Null couldn't open a window: {message}"));
        }
        let why = why(&message, &said);
        if started_again.is_none()
            && let Some(software) = software_vulkan()
        {
            use std::os::unix::process::CommandExt;
            let error = std::process::Command::new(std::env::current_exe().unwrap_or_else(|_| "null".into()))
                .args(std::env::args_os().skip(1))
                .env("VK_DRIVER_FILES", &software)
                .env("VK_ICD_FILENAMES", &software)
                .env(WITHOUT_CARD, &why)
                .exec();
            eprintln!("Null couldn't start again without the graphics card: {error}");
        }
        let software = if started_again.is_some() {
            "Mesa's software Vulkan didn't draw either."
        } else {
            "To draw without the card meanwhile (slower): sudo apt install mesa-vulkan-drivers."
        };
        end(&format!("Null can't draw its window: {why}.\n\n{REMEDY}.\n{software}"))
    }

    /// Why no card drew, plainly, from GPUI's panic and what Vulkan said.
    pub fn why(message: &str, said: &[String]) -> String {
        let too_old = said.iter().find(|s| s.contains("Please update the driver") || s.starts_with("Rejected for"));
        if message.contains("Loading") || said.iter().any(|s| s.contains("Missing Vulkan entry points")) {
            "Vulkan isn't installed (libvulkan1)".to_string()
        } else if let Some(rejected) = too_old {
            format!("the graphics driver is too old for Vulkan as Null draws ({})", rejected.trim_start_matches('\t'))
        } else if message.contains("NoSupportedDeviceFound") {
            "no graphics card answers through Vulkan (its driver may be installed without it)".to_string()
        } else {
            format!("the graphics card didn't start ({})", message.trim_start_matches("Unable to init GPU context: "))
        }
    }

    /// Mesa's software Vulkan (lavapipe), where its driver file is installed.
    fn software_vulkan() -> Option<PathBuf> {
        let mut folders: Vec<PathBuf> = std::env::var("XDG_DATA_DIRS")
            .ok()
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into())
            .split(':')
            .map(|d| PathBuf::from(d).join("vulkan/icd.d"))
            .collect();
        folders.push("/etc/vulkan/icd.d".into());
        folders.into_iter().find_map(|folder| {
            let mut found: Vec<PathBuf> = std::fs::read_dir(folder)
                .ok()?
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("lvp_icd")))
                .collect();
            found.sort();
            // (This machine's own, before a 32-bit one.)
            found.iter().find(|p| p.to_string_lossy().contains("x86_64")).or(found.first()).cloned()
        })
    }

    /// Says `message` (in a dialog too when there's no terminal to read it in) and ends.
    fn end(message: &str) -> ! {
        use std::io::IsTerminal;
        eprintln!("{message}");
        if !std::io::stderr().is_terminal() {
            let shown = ["zenity", "kdialog"].iter().any(|program| {
                let args: Vec<&str> = if *program == "zenity" {
                    vec!["--error", "--title=Null", "--width=480", "--no-markup", "--text", message]
                } else {
                    vec!["--title", "Null", "--error", message]
                };
                std::process::Command::new(program).args(args).status().is_ok()
            });
            if !shown {
                std::process::Command::new("notify-send").args(["Null", message]).status().ok();
            }
        }
        std::process::exit(1)
    }

    #[cfg(test)]
    mod tests {
        use super::why;

        #[test]
        fn why_the_card_did_not_draw_is_said_plainly() {
            assert!(why("Loading(..)", &[]).contains("Vulkan isn't installed"));
            let old = ["Rejected for dynamic rendering. Features = ..".to_string()];
            assert!(why("Unable to init GPU context: NoSupportedDeviceFound", &old).contains("too old"));
            assert!(why("Unable to init GPU context: NoSupportedDeviceFound", &[]).contains("no graphics card"));
        }
    }
}
