//! AI in the terminal, only when asked: a line typed as `# what you want` becomes the
//! command for it on Tab (put on the line, never run), and a command that went wrong can be
//! explained (with the command that puts it right, to put on the line).

use crate::ai::{Effort, Prompt};
use std::path::Path;

/// The signs a prompt ends with, before what's typed (as `terminal` reads them).
const PROMPT_SIGNS: [&str; 6] = ["% ", "$ ", "# ", "> ", "❯ ", "➜ "];

/// What's asked on a prompt's line, typed up to the cursor: `% # list big files` →
/// (the characters from `#` on, to take back off the line; `list big files`). The `#`
/// must come right after the prompt (a root prompt's own `# ` isn't one).
pub fn asked_on_line(line: &str) -> Option<(usize, String)> {
    let at = line
        .char_indices()
        .find(|&(i, c)| c == '#' && i > 0 && PROMPT_SIGNS.iter().any(|sign| line[..i].ends_with(sign)))
        .map(|(i, _)| i)?;
    let asked = line[at + 1..].trim();
    // (A `#!` or `##` is something else; a word or two at least.)
    if asked.is_empty() || asked.starts_with(['!', '#']) || !asked.chars().any(char::is_alphabetic) {
        return None;
    }
    Some((line[at..].chars().count(), asked.to_string()))
}

/// What's in `folder`, to help name files right: the first few names (read from disk:
/// off the window's thread).
pub fn names_in(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .map(|entries| entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).take(200).collect())
        .unwrap_or_default();
    names.sort();
    names.truncate(40);
    names
}

/// Asks for the one command that does what was asked, in this terminal (in `folder`, as
/// its shell writes it, holding `names`).
pub fn command_prompt(asked: &str, terminal: &str, folder: &str, names: &[String]) -> Prompt {
    Prompt {
        system: "You turn a request written in plain words into one command for the terminal described. Reply \
                 with the command only, on one line: no explanation, no markdown, no backticks, no prompt sign. \
                 Use what that shell and system have. Don't add sudo unless the request needs it, and don't delete \
                 or overwrite anything the request doesn't ask to."
            .into(),
        user: format!(
            "Terminal: {terminal}\nFolder: {folder}\nIn it: {}\n\nRequest: {asked}",
            if names.is_empty() { "nothing".to_string() } else { names.join(", ") }
        ),
        max_tokens: Some(300),
        effort: Some(Effort::Low),
        ..Default::default()
    }
}

/// The command in an answer: its first line that has one, without a fence, backticks or a
/// prompt sign, and nothing a shell would take as a key (control characters).
pub fn command_in(answer: &str) -> Option<String> {
    let text = crate::ai::strip_code_fence(&without_thinking(answer));
    let line = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("```"))?;
    let line = line.trim_matches('`').trim();
    let line = line.strip_prefix("$ ").or_else(|| line.strip_prefix("> ")).unwrap_or(line).trim();
    let line: String = line.chars().filter(|c| !c.is_control()).collect();
    (!line.is_empty()).then_some(line)
}

/// An answer without a reasoning model's thinking (`<think>…</think>`); still thinking,
/// nothing yet.
fn without_thinking(answer: &str) -> String {
    let mut text = answer.to_string();
    while let Some(start) = text.find("<think>") {
        match text[start..].find("</think>") {
            Some(end) => text.replace_range(start..start + end + "</think>".len(), ""),
            None => text.truncate(start),
        }
    }
    text
}

/// Programs that show text, so an `error` in what they print is the text's, not theirs.
const SHOWS_TEXT: [&str; 18] = [
    "grep", "egrep", "rg", "ag", "ack", "cat", "bat", "less", "more", "head", "tail", "sed", "awk", "echo", "printf",
    "man", "history", "jq",
];

/// What output says when a command went wrong (the shell gives no exit code to read).
const WENT_WRONG: [&str; 20] = [
    "command not found",
    "is not recognized as",
    "No such file or directory",
    "Permission denied",
    "Traceback (most recent call last)",
    "panicked at",
    "error:",
    "error[",
    "Error:",
    "ERROR",
    "FAILED",
    "fatal:",
    "npm ERR!",
    "npm error",
    "Exception:",
    "Exception in thread",
    "Segmentation fault",
    "cannot find",
    "Could not",
    "Unable to",
];

/// Whether a command looks to have gone wrong, by what it printed (the first line, the
/// command as typed, aside).
pub fn went_wrong(command: &str, output: &str) -> bool {
    let program = command.split_whitespace().find(|w| !w.contains('=')).unwrap_or("");
    let program = program.rsplit(['/', '\\']).next().unwrap_or(program);
    if program.is_empty() || SHOWS_TEXT.contains(&program) || command.starts_with("git log") {
        return false;
    }
    output.lines().skip(1).any(|line| WENT_WRONG.iter().any(|sign| line.contains(sign)))
}

/// How much of the end of what a command printed is sent to be explained.
const EXPLAIN_CHARS: usize = 8_000;

/// Asks what went wrong with `command`, from the end of what it printed.
pub fn explain_prompt(command: &str, output: &str, terminal: &str, folder: &str) -> Prompt {
    let start = output.len().saturating_sub(EXPLAIN_CHARS);
    let start = (start..=output.len()).find(|&i| output.is_char_boundary(i)).unwrap_or(output.len());
    Prompt {
        system: "You explain why a command run in a terminal went wrong, to the person who ran it. In two to four \
                 short plain sentences: what went wrong, and what to do about it. No markdown, no emoji, no \
                 headings. If one command would put it right, add it as the very last line, alone, starting with \
                 `FIX: ` (the command only, for the terminal described). If nothing went wrong, say so in one \
                 sentence."
            .into(),
        user: format!(
            "Terminal: {terminal}\nFolder: {folder}\nCommand: {command}\n\nWhat it printed (its end):\n{}",
            &output[start..]
        ),
        max_tokens: Some(600),
        ..Default::default()
    }
}

/// An explanation as it reads, and the command it ends with to put things right (its
/// `FIX: ` line), if any; while it's still coming, a `FIX` line half written is left out.
pub fn explanation(answer: &str) -> (String, Option<String>) {
    let mut text = Vec::new();
    let mut fix = None;
    let answer = without_thinking(answer);
    for line in answer.lines() {
        let trimmed = line.trim();
        if let Some(command) = trimmed.strip_prefix("FIX:") {
            fix = command_in(command);
        } else if !("FIX:".starts_with(trimmed) && !trimmed.is_empty()) {
            text.push(line);
        }
    }
    (text.join("\n").trim().replace('`', ""), fix)
}

/// Where a terminal runs, for the model to write for it: the shell and the system.
pub fn terminal_words(root: &Path) -> String {
    if let Some(server) = crate::remote::of(root) {
        return format!("a shell on {} over ssh (Linux or another Unix, likely bash)", server.host);
    }
    if crate::wsl::of(root).is_some() {
        return "the person's shell in WSL (Linux on Windows)".into();
    }
    // (Windows PowerShell 5.1 has no `&&` or `||`: PowerShell 7's are wrong there.)
    if cfg!(windows) {
        return if crate::shell::windows_program() == "pwsh.exe" {
            "PowerShell 7 on Windows".into()
        } else {
            "Windows PowerShell 5.1 on Windows (no && or ||)".into()
        };
    }
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).or_else(crate::terminal_watch::account_shell);
    let shell = shell.as_deref().and_then(|s| s.rsplit('/').next()).unwrap_or("sh").to_string();
    let system = if cfg!(target_os = "macos") { "macOS" } else { "Linux" };
    format!("{shell} on {system}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wish_after_the_prompt() {
        assert_eq!(asked_on_line("ada@mac app % # list big files"), Some((16, "list big files".into())));
        // A root prompt's own `#` isn't one.
        assert_eq!(asked_on_line("root@box:~# # show disk use"), Some((15, "show disk use".into())));
        assert_eq!(asked_on_line("PS C:\\app> # files changed today"), Some((21, "files changed today".into())));
        assert_eq!(asked_on_line("➜  app ❯ # é"), Some((3, "é".into())));
        assert_eq!(asked_on_line("ada@mac app % ls"), None);
        assert_eq!(asked_on_line("root@box:~# ls"), None);
        assert_eq!(asked_on_line("ada@mac app % #"), None);
        assert_eq!(asked_on_line("ada@mac app % #!/bin/sh"), None);
        assert_eq!(asked_on_line("ada@mac app % echo 1 # a note"), None);
    }

    #[test]
    fn the_command_in_an_answer() {
        assert_eq!(command_in("find . -size +10M\n").as_deref(), Some("find . -size +10M"));
        assert_eq!(command_in("```bash\n$ du -sh *\n```").as_deref(), Some("du -sh *"));
        assert_eq!(command_in("`ls -la`").as_deref(), Some("ls -la"));
        assert_eq!(command_in("rm x\x1b[A\r").as_deref(), Some("rm x[A"));
        assert_eq!(command_in("  \n"), None);
        // A reasoning model's thinking isn't the command; still thinking, there's none yet.
        assert_eq!(command_in("<think>\nls? no.\n</think>\ndu -sh *").as_deref(), Some("du -sh *"));
        assert_eq!(command_in("<think>\nmaybe ls"), None);
    }

    #[test]
    fn what_went_wrong_by_what_it_printed() {
        assert!(went_wrong("cargo build", "% cargo build\nerror[E0425]: cannot find value `x`\n"));
        assert!(went_wrong("pyhton x.py", "% pyhton x.py\nzsh: command not found: pyhton\n"));
        assert!(!went_wrong("cargo build", "% cargo build\n   Finished dev\n"));
        // The command's own line, and what shows text, don't count.
        assert!(!went_wrong("echo error:", "% echo error:\nerror:\n"));
        assert!(!went_wrong("grep -r ERROR logs", "% grep -r ERROR logs\nlogs/a: ERROR x\n"));
        assert!(!went_wrong("tail -f /var/log/x", "% tail -f /var/log/x\nFAILED\n"));
    }

    #[test]
    fn an_explanation_and_its_fix() {
        let (text, fix) = explanation("The file isn't there.\nCreate it first.\nFIX: `touch notes.txt`\n");
        assert_eq!(text, "The file isn't there.\nCreate it first.");
        assert_eq!(fix.as_deref(), Some("touch notes.txt"));
        // Still coming: a fix half written isn't shown as text.
        assert_eq!(explanation("It's missing.\nFI"), ("It's missing.".to_string(), None));
        assert_eq!(explanation("Nothing went wrong."), ("Nothing went wrong.".to_string(), None));
    }

    #[test]
    fn the_end_of_long_output_is_sent() {
        let output = "é".repeat(EXPLAIN_CHARS);
        let prompt = explain_prompt("make", &output, "zsh on macOS", "/tmp");
        assert!(prompt.user.len() < output.len() + 200);
        assert!(prompt.user.ends_with('é'));
    }
}
