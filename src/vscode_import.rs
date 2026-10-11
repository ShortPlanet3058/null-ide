//! Your settings from VS Code (or Insiders, VSCodium, Cursor), brought over once when
//! asked: the ones Null has too, in its words.

use crate::settings::Settings;
use serde_json::{Map, Value};
use std::path::PathBuf;

/// The editors whose settings can be brought over, and where each keeps them.
fn settings_files() -> Vec<(&'static str, PathBuf)> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return Vec::new() };
    let support = if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData/Roaming"))
    } else {
        home.join(".config")
    };
    [("VS Code", "Code"), ("VS Code Insiders", "Code - Insiders"), ("VSCodium", "VSCodium"), ("Cursor", "Cursor")]
        .into_iter()
        .map(|(name, folder)| (name, support.join(folder).join("User/settings.json")))
        .collect()
}

/// The editor used last (its settings changed most lately), and its settings file.
pub fn latest() -> Option<(&'static str, PathBuf)> {
    settings_files()
        .into_iter()
        .filter_map(|(name, path)| Some((std::fs::metadata(&path).ok()?.modified().ok()?, name, path)))
        .max_by_key(|(when, _, _)| *when)
        .map(|(_, name, path)| (name, path))
}

/// What's on this computer that bringing settings over goes by: the fonts there are, and
/// whether VS Code has Vim's keys (VSCodeVim) installed.
pub struct Here {
    pub fonts: Vec<String>,
    pub vim: bool,
}

impl Here {
    pub fn of(cx: &gpui::App) -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let vim = !cfg!(test)
            && home.is_some_and(|home| {
                [".vscode", ".vscode-insiders", ".vscode-oss", ".cursor"].iter().any(|dir| {
                    std::fs::read_dir(home.join(dir).join("extensions")).is_ok_and(|entries| {
                        entries.flatten().any(|e| e.file_name().to_string_lossy().starts_with("vscodevim.vim-"))
                    })
                })
            });
        Self { fonts: cx.text_system().all_font_names(), vim }
    }
}

/// VS Code's settings (`written`) in Null's words: the editing ones a project can have
/// too, and those only you have (the font, the caret, saving…).
pub fn in_null_words(written: &Map<String, Value>, here: &Here) -> Map<String, Value> {
    let mut out = crate::settings::in_null_words(written);
    // Hidden from Files: yours, as `hide` (`files_exclude` is a project's own list).
    if let Some(hidden) = out.remove("files_exclude") {
        out.insert("hide".into(), hidden);
    }
    // Languages Null has (VS Code's `git-commit`, `makefile`… aren't settings here).
    let names = crate::editor::language_names();
    if let Some(Value::Object(languages)) = out.get_mut("languages") {
        languages.retain(|name, _| names.iter().any(|n| n.eq_ignore_ascii_case(name)));
        if languages.is_empty() {
            out.remove("languages");
        }
    }
    let mut set = |key: &str, value: Value| {
        out.insert(key.to_string(), value);
    };
    let text = |key: &str| written.get(key).and_then(Value::as_str);
    let on = |key: &str| written.get(key).and_then(Value::as_bool);
    if let Some(size) = written.get("editor.fontSize").and_then(Value::as_f64).filter(|s| (6.0..=72.0).contains(s)) {
        set("font_size", Value::from(size));
    }
    // Its first family there is: "Consolas, 'JetBrains Mono', monospace" is JetBrains
    // Mono on a Mac without Consolas.
    let there = |family: &str| crate::fonts::BUNDLED.contains(&family) || here.fonts.iter().any(|f| f == family);
    if let Some(family) = text("editor.fontFamily").and_then(|families| {
        families.split(',').map(|f| f.trim().trim_matches(['\'', '"']).to_string()).find(|f| there(f))
    }) {
        set("code_font", Value::from(family));
    }
    for (theirs, ours) in [
        ("editor.fontLigatures", "ligatures"),
        ("editor.guides.indentation", "indent_guides"),
        ("editor.stickyScroll.enabled", "sticky_scroll"),
        ("editor.codeLens", "code_lens"),
        ("workbench.editor.enablePreview", "preview_tabs"),
    ] {
        if let Some(on) = on(theirs) {
            set(ours, Value::from(on));
        }
    }
    let shown = match text("editor.renderWhitespace") {
        Some("all") => Some("all"),
        Some("trailing") => Some("trailing"),
        Some("selection") => Some("selection"),
        _ => None,
    };
    if let Some(shown) = shown {
        set("whitespace", Value::from(shown));
    }
    match text("editor.lineNumbers") {
        Some("off") => set("line_numbers", Value::from("hidden")),
        Some("on") => set("line_numbers", Value::from("shown")),
        _ => {}
    }
    let caret = match text("editor.cursorStyle") {
        Some("line" | "line-thin") => Some("bar"),
        Some("block" | "block-outline") => Some("block"),
        Some("underline" | "underline-thin") => Some("underline"),
        _ => None,
    };
    if let Some(caret) = caret {
        set("caret", Value::from(caret));
    }
    if let Some(blinking) = text("editor.cursorBlinking") {
        set("caret_blink", Value::from(blinking != "solid"));
    }
    if let Some(rulers) = written.get("editor.rulers").and_then(Value::as_array) {
        set("line_guide", Value::from(!rulers.is_empty()));
    }
    if let Some(hints) = text("editor.inlayHints.enabled") {
        // ("offUnlessPressed": off until a key is held, as Null's are.)
        set("inlay_hints", Value::from(matches!(hints, "on" | "onUnlessPressed")));
    }
    if let Some(closing) = text("editor.autoClosingBrackets") {
        set("auto_close", Value::from(closing != "never"));
    }
    let saving = match text("files.autoSave") {
        Some("off") => Some("off"),
        Some("afterDelay") => Some("after_pause"),
        Some("onFocusChange" | "onWindowChange") => Some("when_leaving"),
        _ => None,
    };
    if let Some(saving) = saving {
        set("auto_save", Value::from(saving));
    }
    // Vim's keys, for someone who has them there (VSCodeVim installed and set up).
    if here.vim && written.keys().any(|k| k.starts_with("vim.")) {
        set("keymap", Value::from("vim"));
    }
    out
}

/// What bringing `written` over changes: Null's settings after, and each one changed, in
/// words ("Font size: 14").
pub fn changes(settings: &Settings, written: &Map<String, Value>, here: &Here) -> (Settings, Vec<String>) {
    let (after, _) = crate::settings::merged(settings, &in_null_words(written, here));
    let (before_json, after_json) =
        (serde_json::to_value(settings).unwrap_or_default(), serde_json::to_value(&after).unwrap_or_default());
    let said = |value: &Value| match value {
        Value::Bool(true) => "on".to_string(),
        Value::Bool(false) => "off".to_string(),
        Value::String(s) => s.replace('_', " "),
        Value::Array(list) => list.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
        Value::Object(map) => map.keys().cloned().collect::<Vec<_>>().join(", "),
        // 17, not 17.0.
        Value::Number(n) if n.as_f64().is_some_and(|f| f.fract() == 0.) => {
            format!("{}", n.as_f64().unwrap_or_default() as i64)
        }
        other => other.to_string(),
    };
    let mut changed = Vec::new();
    for (key, value) in after_json.as_object().into_iter().flatten() {
        if before_json.get(key) == Some(value) {
            continue;
        }
        let name = match key.as_str() {
            "font_size" => "Font size",
            "code_font" => "Code font",
            "ligatures" => "Ligatures",
            "indent_size" => "Indent size",
            "indent_with_tabs" => "Indent with tabs",
            "word_wrap" => "Word wrap",
            "format_on_save" => "Format on save",
            "format_on_paste" => "Format on paste",
            "whitespace" => "Show whitespace",
            "line_numbers" => "Line numbers",
            "caret" => "Caret",
            "caret_blink" => "Caret blink",
            "bracket_colours" => "Bracket colours",
            "indent_guides" => "Indent guides",
            "line_guide" => "Line guide",
            "sticky_scroll" => "Sticky scroll",
            "inlay_hints" => "Inlay hints",
            "code_lens" => "Code lens",
            "auto_close" => "Close brackets",
            "auto_save" => "Auto save",
            "preview_tabs" => "Preview tabs",
            "hide" => "Hidden from Files",
            "languages" => "Settings by language",
            "keymap" => "Keys",
            other => other,
        };
        changed.push(format!("{name}: {}", said(value)));
    }
    (after, changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn your_vs_code_settings_in_null_s_words() {
        let written: Map<String, Value> = serde_json::from_str(
            r#"{
                "editor.fontSize": 17, "editor.fontFamily": "Consolas, 'JetBrains Mono', Menlo, monospace",
                "editor.fontLigatures": true, "editor.tabSize": 2, "editor.cursorStyle": "block",
                "editor.cursorBlinking": "solid", "editor.renderWhitespace": "boundary",
                "files.autoSave": "onFocusChange", "editor.rulers": [100],
                "[python]": { "editor.tabSize": 4 }, "workbench.colorTheme": "One Dark Pro",
                "vim.useSystemClipboard": true, "editor.inlayHints.enabled": "offUnlessPressed",
                "files.exclude": { "**/*.pyc": true }, "[git-commit]": { "editor.wordWrap": "on" }
            }"#,
        )
        .unwrap();
        // A Mac without Consolas, with VSCodeVim.
        let here = Here { fonts: vec!["Menlo".into()], vim: true };
        let null = in_null_words(&written, &here);
        assert_eq!(null["inlay_hints"], false);
        assert_eq!(null["hide"], serde_json::json!(["**/*.pyc"]), "yours, not a project's list");
        assert!(!null.contains_key("files_exclude"));
        assert!(null["languages"].as_object().unwrap().keys().all(|k| k.eq_ignore_ascii_case("python")), "{null:?}");
        // VSCodeVim's settings left behind (it's gone): the keys stay.
        assert!(!in_null_words(&written, &Here { fonts: Vec::new(), vim: false }).contains_key("keymap"));
        assert_eq!(null["font_size"], 17.0);
        assert_eq!(null["code_font"], "JetBrains Mono");
        assert_eq!(
            (&null["caret"], &null["caret_blink"], &null["auto_save"]),
            (&"block".into(), &false.into(), &"when_leaving".into())
        );
        assert!(!null.contains_key("whitespace"), "boundary: none of Null's");
        assert_eq!((&null["line_guide"], &null["keymap"]), (&true.into(), &"vim".into()));
        let (after, changed) = changes(&Settings::default(), &written, &here);
        assert_eq!(after.font_size, 17.);
        assert_eq!(after.indent_size, 2);
        assert_eq!(after.caret, crate::settings::CaretShape::Block);
        assert!(after.languages.keys().any(|k| k.eq_ignore_ascii_case("python")));
        assert!(changed.contains(&"Font size: 17".to_string()), "{changed:?}");
        assert!(changed.contains(&"Code font: JetBrains Mono".to_string()), "{changed:?}");
        assert!(changed.contains(&"Auto save: when leaving".to_string()), "{changed:?}");
        // Brought over again: nothing more changes.
        let (_, again) = changes(&after, &written, &here);
        assert!(again.is_empty(), "{again:?}");
        // A value Null can't read is left out, not the rest.
        let odd: Map<String, Value> =
            serde_json::from_str(r#"{ "editor.fontSize": "big", "editor.tabSize": 8 }"#).unwrap();
        assert_eq!(changes(&Settings::default(), &odd, &here).0.indent_size, 8);
    }
}
