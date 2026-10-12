//! Command lines Null types into its terminal, written for the shell there: a POSIX shell
//! (zsh, bash…) on the Mac and Linux, PowerShell on Windows, where `(cd x && y)`, `X=1 y`
//! and `'\''` don't work (Windows PowerShell 5 has no `&&` at all).

/// The kind of shell the terminal runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Posix,
    PowerShell,
}

/// The terminal's shell: PowerShell on Windows (Null starts it there), else a POSIX one.
pub fn kind() -> Kind {
    if cfg!(windows) { Kind::PowerShell } else { Kind::Posix }
}

/// The program the terminal starts on Windows: PowerShell 7 when it's installed, else the
/// Windows PowerShell every Windows has.
pub fn windows_program() -> String {
    if crate::tools::find("pwsh").is_some() { "pwsh.exe".into() } else { "powershell.exe".into() }
}

/// `word` as the shell takes it: as it is when it's plain, quoted otherwise.
pub fn quote(word: &str, kind: Kind) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_alphanumeric() || "-_./:=@+,%".contains(c) || (c == '\\' && kind == Kind::PowerShell));
    if plain {
        return word.to_string();
    }
    match kind {
        Kind::Posix => format!("'{}'", word.replace('\'', r"'\''")),
        // (In PowerShell's single quotes only a quote is special: doubled.)
        Kind::PowerShell => format!("'{}'", word.replace('\'', "''")),
    }
}

/// Words joined as one line, each quoted as it needs.
pub fn line(words: &[String], kind: Kind) -> String {
    words.iter().map(|w| quote(w, kind)).collect::<Vec<_>>().join(" ")
}

/// `command` run in `folder` (quoted here), the terminal staying where it was.
pub fn in_folder(folder: &str, command: &str, kind: Kind) -> String {
    let folder = quote(folder, kind);
    match kind {
        Kind::Posix => format!("(cd {folder} && {command})"),
        Kind::PowerShell => format!("Push-Location {folder}; try {{ {command} }} finally {{ Pop-Location }}"),
    }
}

/// `command` run with these variables of the environment set.
pub fn with_env(vars: &[(String, String)], command: &str, kind: Kind) -> String {
    if vars.is_empty() {
        return command.to_string();
    }
    match kind {
        Kind::Posix => {
            let set: Vec<String> = vars.iter().map(|(k, v)| format!("{k}={}", quote(v, kind))).collect();
            format!("{} {command}", set.join(" "))
        }
        Kind::PowerShell => {
            let set: Vec<String> = vars.iter().map(|(k, v)| format!("$env:{k}={}", quote_always(v, kind))).collect();
            format!("{}; {command}", set.join("; "))
        }
    }
}

/// Commands one after the other, each only if the one before worked.
pub fn and_then(commands: &[String], kind: Kind) -> String {
    match kind {
        Kind::Posix => commands.join(" && "),
        // (`&&` is PowerShell 7's only: `$?` says whether the last one worked, in both;
        // nested, as a skipped `if` sets it again.)
        Kind::PowerShell => {
            let Some((last, before)) = commands.split_last() else { return String::new() };
            before.iter().rev().fold(last.clone(), |rest, command| format!("{command}; if ($?) {{ {rest} }}"))
        }
    }
}

/// Quoted even when plain (PowerShell's `$env:X=` wants a string).
fn quote_always(word: &str, kind: Kind) -> String {
    match kind {
        Kind::Posix => format!("'{}'", word.replace('\'', r"'\''")),
        Kind::PowerShell => format!("'{}'", word.replace('\'', "''")),
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
            "Push-Location web; try { npm test } finally { Pop-Location }"
        );
        let vars = [("UPDATE_EXPECT".to_string(), "1".to_string())];
        assert_eq!(with_env(&vars, "cargo test", Posix), "UPDATE_EXPECT=1 cargo test");
        assert_eq!(with_env(&vars, "cargo test", PowerShell), "$env:UPDATE_EXPECT='1'; cargo test");
        let steps = ["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(and_then(&steps, Posix), "a && b && c");
        assert_eq!(and_then(&steps, PowerShell), "a; if ($?) { b; if ($?) { c } }");
    }
}
