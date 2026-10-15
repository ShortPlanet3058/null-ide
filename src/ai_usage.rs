//! How many tokens AI requests used: the last one, and this month's, as the providers count
//! them (what was read, what of that came from their cache, what was written). Kept in
//! Null's folder, so a monthly limit holds across launches. Nothing is sent anywhere.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// One request's tokens, or a sum of them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Read: the prompt, all of it (`cached` included).
    pub input: u64,
    /// Of what was read, how much came from the provider's cache (cheaper, faster).
    pub cached: u64,
    /// Written: the answer (thinking included, where it's counted).
    pub output: u64,
}

impl Usage {
    pub fn total(&self) -> u64 {
        self.input + self.output
    }

    fn add(&mut self, other: Usage) {
        self.input += other.input;
        self.cached += other.cached;
        self.output += other.output;
    }

    /// From an OpenAI-style `usage` object (`prompt_tokens`, `completion_tokens`, and the
    /// cached part where the provider says it).
    pub fn from_openai(usage: &serde_json::Value) -> Option<Usage> {
        let input = usage["prompt_tokens"].as_u64()?;
        let cached = usage["prompt_tokens_details"]["cached_tokens"]
            .as_u64()
            .or_else(|| usage["prompt_cache_hit_tokens"].as_u64())
            .unwrap_or(0);
        Some(Usage { input, cached, output: usage["completion_tokens"].as_u64().unwrap_or(0) })
    }

    /// From Anthropic's (`input_tokens` is only what wasn't cached there).
    pub fn from_anthropic(usage: &serde_json::Value) -> Usage {
        let fresh = usage["input_tokens"].as_u64().unwrap_or(0);
        let read = usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
        let written = usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);
        Usage { input: fresh + read + written, cached: read, output: usage["output_tokens"].as_u64().unwrap_or(0) }
    }
}

/// A month's tokens, as kept in `ai-usage.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Month {
    /// "2026-10".
    pub month: String,
    #[serde(flatten)]
    pub usage: Usage,
    pub requests: u64,
}

struct State {
    last: Option<Usage>,
    month: Option<Month>,
}

static STATE: Mutex<State> = Mutex::new(State { last: None, month: None });

fn this_month() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    // Days since 1970 to a civil date (Howard Hinnant's algorithm).
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}")
}

fn file() -> Option<std::path::PathBuf> {
    Some(crate::tools::data_dir()?.join("ai-usage.json"))
}

/// This month's, read once from the file (a new month starts from nothing).
fn month_in(state: &mut State) -> &mut Month {
    let now = this_month();
    let stale = state.month.as_ref().is_none_or(|m| m.month != now);
    if stale {
        let saved = file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|s| serde_json::from_str::<Month>(&s).ok())
            .filter(|m| m.month == now);
        state.month = Some(saved.unwrap_or(Month { month: now, ..Default::default() }));
    }
    state.month.as_mut().expect("just set")
}

/// A request's tokens, counted.
pub fn record(usage: Usage) {
    let Ok(mut state) = STATE.lock() else { return };
    state.last = Some(usage);
    let month = month_in(&mut state);
    month.usage.add(usage);
    month.requests += 1;
    let saved = month.clone();
    drop(state);
    if let (Some(path), Ok(json)) = (file(), serde_json::to_string_pretty(&saved)) {
        let _ = std::fs::write(path, json);
    }
}

pub fn last() -> Option<Usage> {
    STATE.lock().ok()?.last
}

pub fn this_months() -> Month {
    STATE.lock().map(|mut s| month_in(&mut s).clone()).unwrap_or_default()
}

/// "12.4k", "1.2M": a count at a glance.
pub fn short(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1_000.0).replace(".0k", "k"),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0).replace(".0M", "M"),
    }
}

/// What a request will cost, roughly, before it's sent (a token is about four characters).
pub fn estimate(text: &str) -> u64 {
    (text.chars().count() as u64).div_ceil(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_each_provider_s_counts() {
        let openai = serde_json::json!({ "prompt_tokens": 1200, "completion_tokens": 80, "prompt_tokens_details": { "cached_tokens": 1024 } });
        assert_eq!(Usage::from_openai(&openai), Some(Usage { input: 1200, cached: 1024, output: 80 }));
        let deepseek =
            serde_json::json!({ "prompt_tokens": 50, "completion_tokens": 5, "prompt_cache_hit_tokens": 40 });
        assert_eq!(Usage::from_openai(&deepseek).map(|u| u.cached), Some(40));
        assert_eq!(Usage::from_openai(&serde_json::json!({})), None);
        // Anthropic counts the cache apart: all of it was read.
        let anthropic = serde_json::json!({ "input_tokens": 10, "cache_read_input_tokens": 3000, "cache_creation_input_tokens": 5, "output_tokens": 70 });
        assert_eq!(Usage::from_anthropic(&anthropic), Usage { input: 3015, cached: 3000, output: 70 });
    }

    #[test]
    fn counts_add_up_over_the_month() {
        let before = this_months();
        record(Usage { input: 100, cached: 60, output: 10 });
        record(Usage { input: 50, cached: 0, output: 5 });
        let after = this_months();
        assert_eq!(after.month, this_month());
        assert_eq!(after.usage.input - before.usage.input, 150);
        assert_eq!(after.usage.cached - before.usage.cached, 60);
        assert_eq!(after.requests - before.requests, 2);
        assert_eq!(last(), Some(Usage { input: 50, cached: 0, output: 5 }));
    }

    #[test]
    fn says_counts_briefly() {
        assert_eq!(short(950), "950");
        assert_eq!(short(12_400), "12.4k");
        assert_eq!(short(3_000), "3k");
        assert_eq!(short(1_240_000), "1.2M");
        assert_eq!(this_month().len(), 7);
    }
}
