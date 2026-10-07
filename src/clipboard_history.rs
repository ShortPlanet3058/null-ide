//! What was copied or cut in Null lately, to paste again (⌘K Paste from History…): the
//! last 20, newest first, each as it was copied (whole lines, its indentation), so it
//! pastes as it would have then.

use gpui::{App, ClipboardItem, Global};
use std::time::Instant;

const KEEP: usize = 20;
/// Copies bigger than this aren't kept.
const MAX_BYTES: usize = 1024 * 1024;

#[derive(Default)]
struct ClipboardHistory(Vec<(ClipboardItem, Instant)>);

impl Global for ClipboardHistory {}

/// `item` was just copied: first in the history (once, however often it's copied).
pub fn remember(item: &ClipboardItem, cx: &mut App) {
    let Some(text) = item.text() else { return };
    // A huge copy (a whole log) isn't kept: twenty of them would weigh on memory.
    if text.trim().is_empty() || text.len() > MAX_BYTES {
        return;
    }
    let history = &mut cx.default_global::<ClipboardHistory>().0;
    history.retain(|(kept, _)| kept.text().is_none_or(|t| t.len() != text.len() || t != text));
    history.insert(0, (item.clone(), Instant::now()));
    history.truncate(KEEP);
}

/// What was copied, newest first, and when.
pub fn entries(cx: &App) -> Vec<(ClipboardItem, Instant)> {
    cx.try_global::<ClipboardHistory>().map(|h| h.0.clone()).unwrap_or_default()
}

/// How a copy reads in the list: its first line with something on it, shortened, then how
/// many lines.
pub fn preview(text: &str) -> (String, usize) {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let short: String = first.chars().take(80).collect();
    let short = if first.chars().count() > 80 { format!("{short}…") } else { short };
    (short, text.trim_end_matches('\n').lines().count().max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn newest_first_each_once(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            for text in ["one", "two", "one", "   "] {
                remember(&ClipboardItem::new_string(text.into()), cx);
            }
            let texts: Vec<String> = entries(cx).into_iter().filter_map(|(item, _)| item.text()).collect();
            assert_eq!(texts, ["one", "two"]);
        });
        assert_eq!(preview("\n  fn main() {\n    go();\n}\n"), ("fn main() {".to_string(), 4));
    }
}
