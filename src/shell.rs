//! Command lines Null types into its terminal, written for the shell there: a POSIX shell
//! (zsh, bash…) on the Mac and Linux, PowerShell on Windows, where `(cd x && y)`, `X=1 y`
//! and `'\''` don't work (Windows PowerShell 5 has no `&&` at all).

/// The kind of shell the terminal runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Posix,
    PowerShell,
}

/// Whether the window being worked in has a folder inside WSL: its terminal's shell is
/// Linux's then (see `wsl`), and what's typed into it is written for it.
static WSL_PROJECT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Said by the window that comes to the front (and when its folder changes).
pub fn set_wsl_project(wsl: bool) {
    WSL_PROJECT.store(wsl, std::sync::atomic::Ordering::Relaxed);
}

/// The terminal's shell: PowerShell on Windows (Null starts it there), a POSIX one elsewhere
/// and for a folder inside WSL.
pub fn kind() -> Kind {
    if cfg!(windows) && !WSL_PROJECT.load(std::sync::atomic::Ordering::Relaxed) {
        Kind::PowerShell
    } else {
        Kind::Posix
    }
}

/// A path as the terminal's shell has it: a folder inside WSL in its Linux form.
pub fn path_text(path: &std::path::Path) -> String {
    match crate::wsl::of(path) {
        Some(wsl) => wsl.linux,
        None => path.display().to_string(),
    }
}

/// The program the terminal starts on Windows: PowerShell 7 when it's installed, else the
/// Windows PowerShell every Windows has.
pub fn windows_program() -> String {
    if crate::tools::find("pwsh").is_some() { "pwsh.exe".into() } else { "powershell.exe".into() }
}

/// Run by Windows' terminal as PowerShell starts: its prompt puts the folder it's in into the
/// window's title, whatever prompt the person has, so Null knows where the shell is.
pub const POWERSHELL_FOLDER_IN_TITLE: &str = "$global:__NullPrompt = $function:prompt; \
     function global:prompt { $Host.UI.RawUI.WindowTitle = $PWD.ProviderPath; & $global:__NullPrompt }";

/// `word` as the shell takes it: as it is when it's plain, quoted otherwise.
pub fn quote(word: &str, kind: Kind) -> String {
    // (In PowerShell `@` starts splatting and `,` makes a list: not plain there.)
    let plain_marks = match kind {
        Kind::Posix => "-_./:=@+,%",
        Kind::PowerShell => "-_./:=+%\\",
    };
    let plain = !word.is_empty() && word.chars().all(|c| c.is_alphanumeric() || plain_marks.contains(c));
    if plain { word.to_string() } else { quote_always(word, kind) }
}

/// A program to run, quoted as needed: in PowerShell a quoted one is a string, not a
/// command, unless `&` runs it.
pub fn program(word: &str, kind: Kind) -> String {
    let quoted = quote(word, kind);
    if kind == Kind::PowerShell && quoted.starts_with('\'') { format!("& {quoted}") } else { quoted }
}

/// Words joined as one line, each quoted as it needs.
pub fn line(words: &[String], kind: Kind) -> String {
    words.iter().map(|w| quote(w, kind)).collect::<Vec<_>>().join(" ")
}

/// `command` run in `folder` (quoted here), the terminal staying where it was.
pub fn in_folder(folder: &str, command: &str, kind: Kind) -> String {
    let folder = quote(&path_text(std::path::Path::new(folder)), kind);
    match kind {
        Kind::Posix => format!("(cd {folder} && {command})"),
        // (-LiteralPath: `[ ]` in a name aren't wildcards.)
        Kind::PowerShell => {
            format!("Push-Location -LiteralPath {folder}; try {{ {command} }} finally {{ Pop-Location }}")
        }
    }
}

/// `command` run with these variables of the environment set, for it alone.
pub fn with_env(vars: &[(String, String)], command: &str, kind: Kind) -> String {
    if vars.is_empty() {
        return command.to_string();
    }
    match kind {
        Kind::Posix => {
            let set: Vec<String> = vars.iter().map(|(k, v)| format!("{k}={}", quote(v, kind))).collect();
            format!("{} {command}", set.join(" "))
        }
        // (PowerShell sets them for the session: put back as they were after.)
        Kind::PowerShell => {
            let keep: Vec<String> = vars.iter().map(|(k, _)| format!("$__null_{k} = $env:{k}")).collect();
            let set: Vec<String> = vars.iter().map(|(k, v)| format!("$env:{k} = {}", quote_always(v, kind))).collect();
            let back: Vec<String> = vars.iter().map(|(k, _)| format!("$env:{k} = $__null_{k}")).collect();
            format!("{}; {}; try {{ {command} }} finally {{ {} }}", keep.join("; "), set.join("; "), back.join("; "))
        }
    }
}

/// Commands one after the other, each only if the one before worked.
pub fn and_then(commands: &[String], kind: Kind) -> String {
    match kind {
        Kind::Posix => commands.join(" && "),
        // (`&&` is PowerShell 7's only. What a program says is `$LASTEXITCODE`, cleared
        // first: `$?` alone would be Pop-Location's after a command run in a folder.)
        Kind::PowerShell => {
            let Some((last, before)) = commands.split_last() else { return String::new() };
            let chained = before
                .iter()
                .rev()
                .fold(last.clone(), |rest, command| format!("{command}; if ($? -and -not $LASTEXITCODE) {{ {rest} }}"));
            if before.is_empty() { chained } else { format!("$global:LASTEXITCODE = 0; {chained}") }
        }
    }
}

/// Quoted even when plain (PowerShell's `$env:X=` wants a string).
fn quote_always(word: &str, kind: Kind) -> String {
    match kind {
        Kind::Posix => format!("'{}'", word.replace('\'', r"'\''")),
        // (In PowerShell's single quotes only a quote is special, doubled; its typographic
        // ones count as quotes too.)
        Kind::PowerShell => {
            let mut quoted = String::from("'");
            for c in word.chars() {
                if matches!(c, '\'' | '‘' | '’' | '‚' | '‛') {
                    quoted.push(c);
                }
                quoted.push(c);
            }
            quoted.push('\'');
            quoted
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Kind::*;

    #[test]
    fn lines_are_written_for_each_shell() {
        assert_eq!(quote("src/a.rs", Posix), "src/a.rs");
        assert_eq!(quote("it's here", Posix), r"'it'\''s here'");
        assert_eq!(quote("it's here", PowerShell), "'it''s here'");
        assert_eq!(quote(r"C:\work\a.rs", PowerShell), r"C:\work\a.rs");
        assert_eq!(in_folder("web app", "npm test", Posix), "(cd 'web app' && npm test)");
        assert_eq!(
            in_folder("web", "npm test", PowerShell),
            "Push-Location -LiteralPath web; try { npm test } finally { Pop-Location }"
        );
        let vars = [("UPDATE_EXPECT".to_string(), "1".to_string())];
        assert_eq!(with_env(&vars, "cargo test", Posix), "UPDATE_EXPECT=1 cargo test");
        assert_eq!(
            with_env(&vars, "cargo test", PowerShell),
            "$__null_UPDATE_EXPECT = $env:UPDATE_EXPECT; $env:UPDATE_EXPECT = '1'; \
             try { cargo test } finally { $env:UPDATE_EXPECT = $__null_UPDATE_EXPECT }"
        );
        let steps = ["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(and_then(&steps, Posix), "a && b && c");
        assert_eq!(
            and_then(&steps, PowerShell),
            "$global:LASTEXITCODE = 0; a; if ($? -and -not $LASTEXITCODE) { b; if ($? -and -not $LASTEXITCODE) { c } }"
        );
        assert_eq!(quote("@smoke", PowerShell), "'@smoke'");
        assert_eq!(quote("it’s", PowerShell), "'it’’s'");
        assert_eq!(program(r"C:\My Env\python.exe", PowerShell), r"& 'C:\My Env\python.exe'");
        assert_eq!(program("python", PowerShell), "python");
    }
}
