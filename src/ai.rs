//! AI on demand: where answers come from, and one blocking call that streams text back.
//!
//! API keys live in the system keychain (or an environment variable), never in settings.
//! Subscriptions are used through the official CLIs the person has installed and signed
//! into; Null never sees those logins.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    #[default]
    Off,
    Nvidia,
    Ollama,
    OpenaiCompatible,
    Claude,
    ClaudeCode,
    Codex,
}

impl ProviderId {
    pub fn label(self) -> &'static str {
        match self {
            ProviderId::Off => "Off",
            ProviderId::Nvidia => "NVIDIA",
            ProviderId::Ollama => "Ollama",
            ProviderId::OpenaiCompatible => "OpenAI-compatible",
            ProviderId::Claude => "Claude API",
            ProviderId::ClaudeCode => "Claude Code",
            ProviderId::Codex => "Codex",
        }
    }

    /// The key used in settings and the keychain.
    pub fn key(self) -> &'static str {
        match self {
            ProviderId::Off => "off",
            ProviderId::Nvidia => "nvidia",
            ProviderId::Ollama => "ollama",
            ProviderId::OpenaiCompatible => "openai_compatible",
            ProviderId::Claude => "claude",
            ProviderId::ClaudeCode => "claude_code",
            ProviderId::Codex => "codex",
        }
    }

    /// Whether this provider takes an API key (Ollama and the CLIs don't).
    pub fn uses_api_key(self) -> bool {
        matches!(self, ProviderId::Nvidia | ProviderId::OpenaiCompatible | ProviderId::Claude)
    }

    fn key_env(self) -> Option<&'static str> {
        match self {
            ProviderId::Nvidia => Some("NVIDIA_API_KEY"),
            ProviderId::OpenaiCompatible => Some("OPENAI_API_KEY"),
            ProviderId::Claude => Some("ANTHROPIC_API_KEY"),
            _ => None,
        }
    }

    pub fn default_base_url(self) -> Option<&'static str> {
        match self {
            ProviderId::Nvidia => Some("https://integrate.api.nvidia.com/v1"),
            ProviderId::Ollama => Some("http://localhost:11434/v1"),
            ProviderId::OpenaiCompatible => Some("https://api.openai.com/v1"),
            _ => None,
        }
    }

    pub fn default_model(self) -> Option<&'static str> {
        match self {
            ProviderId::Nvidia => Some("nvidia/nemotron-3-super-120b-a12b"),
            ProviderId::Ollama => Some("qwen2.5-coder:7b"),
            ProviderId::Claude => Some("claude-opus-5-5"),
            // The CLIs use whatever model the person picked in them.
            _ => None,
        }
    }
}

/// Per-provider overrides, e.g. `"nvidia": { "model": "qwen/qwen3-coder-480b-a35b-instruct" }`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Let reasoning models think before answering. Slower; off by default on NVIDIA,
    /// where it can turn a one-second edit into a thirty-second one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    pub provider: ProviderId,
    #[serde(flatten)]
    pub providers: HashMap<String, ProviderSettings>,
}

impl AiSettings {
    pub fn model(&self, id: ProviderId) -> Option<String> {
        self.providers
            .get(id.key())
            .and_then(|p| p.model.clone())
            .filter(|m| !m.is_empty())
            .or(id.default_model().map(str::to_owned))
    }

    fn reasoning(&self, id: ProviderId) -> bool {
        self.providers.get(id.key()).and_then(|p| p.reasoning).unwrap_or(false)
    }

    fn base_url(&self, id: ProviderId) -> Option<String> {
        self.providers
            .get(id.key())
            .and_then(|p| p.base_url.clone())
            .filter(|u| !u.is_empty())
            .or(id.default_base_url().map(str::to_owned))
    }
}

const KEYCHAIN_SERVICE: &str = "Null IDE";

/// The provider's key: the keychain first, then its usual environment variable.
pub fn api_key(id: ProviderId) -> Option<String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, id.key())
        .ok()
        .and_then(|e| e.get_password().ok())
        .or_else(|| id.key_env().and_then(|v| std::env::var(v).ok()))
        .filter(|k| !k.trim().is_empty())
}

pub fn store_api_key(id: ProviderId, key: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, id.key()).map_err(|e| e.to_string())?;
    if key.trim().is_empty() { entry.delete_credential().or(Ok(())) } else { entry.set_password(key.trim()) }
        .map_err(|e: keyring::Error| e.to_string())
}

pub struct Prompt {
    pub system: String,
    pub user: String,
}

pub enum AiEvent {
    Text(String),
    Done,
    Failed(String),
}

/// Runs [`ask`] on its own thread and streams what happens. Dropping the receiver
/// stops listening; the request itself finishes in the background.
pub fn stream(settings: AiSettings, prompt: Prompt) -> futures::channel::mpsc::UnboundedReceiver<AiEvent> {
    let (tx, rx) = futures::channel::mpsc::unbounded();
    std::thread::Builder::new()
        .name("ai-request".into())
        .spawn(move || {
            let text_tx = tx.clone();
            let result = ask(&settings, &prompt, &mut |text| {
                text_tx.unbounded_send(AiEvent::Text(text.to_string())).ok();
            });
            tx.unbounded_send(match result {
                Ok(()) => AiEvent::Done,
                Err(message) => AiEvent::Failed(message),
            })
            .ok();
        })
        .ok();
    rx
}

/// Models sometimes wrap code in a markdown fence despite being asked not to.
pub fn strip_code_fence(text: &str) -> String {
    let trimmed = text.trim_matches('\n');
    let mut lines: Vec<&str> = trimmed.lines().collect();
    if lines.first().is_some_and(|l| l.trim_start().starts_with("```")) {
        lines.remove(0);
        if lines.last().is_some_and(|l| l.trim() == "```") {
            lines.pop();
        }
    }
    lines.join("\n")
}

/// Asks the configured provider and calls `on_text` as the answer arrives.
/// Blocking: run it off the UI thread. Errors are written for people, not logs.
pub fn ask(settings: &AiSettings, prompt: &Prompt, on_text: &mut dyn FnMut(&str)) -> Result<(), String> {
    let id = settings.provider;
    match id {
        ProviderId::Off => Err("AI is off. Choose a provider with “AI: Use …” in the command palette.".into()),
        ProviderId::Nvidia | ProviderId::Ollama | ProviderId::OpenaiCompatible => {
            let base = settings.base_url(id).unwrap_or_default();
            let model = settings.model(id).ok_or_else(|| {
                format!("Set a model for {} in settings (\"ai\" → \"{}\" → \"model\").", id.label(), id.key())
            })?;
            let key = api_key(id);
            if key.is_none() && id != ProviderId::Ollama && id != ProviderId::OpenaiCompatible {
                return Err(missing_key(id));
            }
            openai_compatible(id, &base, &model, key.as_deref(), settings.reasoning(id), prompt, on_text)
        }
        ProviderId::Claude => {
            let key = api_key(id).ok_or_else(|| missing_key(id))?;
            claude_api(&settings.model(id).unwrap_or_default(), &key, prompt, on_text)
        }
        ProviderId::ClaudeCode => claude_code(settings.model(id), prompt, on_text),
        ProviderId::Codex => codex(settings.model(id), prompt, on_text),
    }
}

fn missing_key(id: ProviderId) -> String {
    format!("No API key for {}. Add one with “AI: Set API Key” in the command palette.", id.label())
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).build().into()
}

/// The message inside a provider's error response, or the raw body.
fn error_message(status: u16, body: &str) -> String {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let message = parsed
        .as_ref()
        .and_then(|v| {
            v["error"]["message"].as_str().or(v["error"].as_str()).or(v["detail"].as_str()).or(v["message"].as_str())
        })
        .map(str::to_owned)
        .unwrap_or_else(|| body.chars().take(300).collect());
    match status {
        401 | 403 => format!("The API key was refused ({status}): {message}"),
        404 => format!("Not found ({status}). Check the model name and address: {message}"),
        429 => format!("Rate limited ({status}). Try again in a moment: {message}"),
        _ => format!("The provider answered {status}: {message}"),
    }
}

/// Reads a server-sent event stream, calling `on_data` with each `data:` payload.
fn read_sse(reader: impl Read, mut on_data: impl FnMut(&str) -> bool) -> Result<(), String> {
    for line in BufReader::new(reader).lines() {
        let line = line.map_err(|e| format!("The connection dropped: {e}"))?;
        if let Some(data) = line.strip_prefix("data:")
            && !on_data(data.trim())
        {
            break;
        }
    }
    Ok(())
}

fn openai_compatible(
    id: ProviderId,
    base: &str,
    model: &str,
    key: Option<&str>,
    reasoning: bool,
    prompt: &Prompt,
    on_text: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let mut request = agent().post(&url).header("Content-Type", "application/json");
    if let Some(key) = key {
        request = request.header("Authorization", &format!("Bearer {key}"));
    }
    let mut body = json!({
        "model": model,
        "stream": true,
        "temperature": 0.2,
        "messages": [
            { "role": "system", "content": prompt.system },
            { "role": "user", "content": prompt.user },
        ],
    });
    // NVIDIA's hosted reasoning models think first unless told not to. Only NVIDIA
    // gets this field: other OpenAI-compatible servers may reject unknown ones.
    if id == ProviderId::Nvidia && !reasoning {
        body["chat_template_kwargs"] = json!({ "enable_thinking": false });
    }
    let response = request.send_json(&body).map_err(|e| {
        if id == ProviderId::Ollama {
            format!("Couldn't reach Ollama at {base}. Is it running? ({e})")
        } else {
            format!("Couldn't reach {}: {e}", id.label())
        }
    })?;
    let status = response.status().as_u16();
    let mut body = response.into_body();
    if status >= 400 {
        return Err(error_message(status, &body.read_to_string().unwrap_or_default()));
    }
    read_sse(body.into_reader(), |data| {
        if data == "[DONE]" {
            return false;
        }
        if let Ok(event) = serde_json::from_str::<Value>(data)
            && let Some(text) = event["choices"][0]["delta"]["content"].as_str()
        {
            on_text(text);
        }
        true
    })
}

/// Claude through the Messages API. Streams text; declined requests fall back to the
/// model Anthropic recommends for that case instead of failing.
fn claude_api(model: &str, key: &str, prompt: &Prompt, on_text: &mut dyn FnMut(&str)) -> Result<(), String> {
    let body = json!({
        "model": model,
        "max_tokens": 16000,
        "stream": true,
        "fallbacks": "default",
        "output_config": { "effort": "medium" },
        "system": prompt.system,
        "messages": [{ "role": "user", "content": prompt.user }],
    });
    let response = agent()
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .header("anthropic-beta", "server-side-fallback-2026-07-01")
        .header("content-type", "application/json")
        .send_json(&body)
        .map_err(|e| format!("Couldn't reach the Claude API: {e}"))?;
    let status = response.status().as_u16();
    let mut body = response.into_body();
    if status >= 400 {
        return Err(error_message(status, &body.read_to_string().unwrap_or_default()));
    }
    let mut refused = false;
    let mut failure = None;
    read_sse(body.into_reader(), |data| {
        let Ok(event) = serde_json::from_str::<Value>(data) else { return true };
        match event["type"].as_str() {
            Some("content_block_delta") if event["delta"]["type"] == "text_delta" => {
                on_text(event["delta"]["text"].as_str().unwrap_or_default());
            }
            Some("message_delta") if event["delta"]["stop_reason"] == "refusal" => refused = true,
            Some("error") => failure = event["error"]["message"].as_str().map(str::to_owned),
            Some("message_stop") => return false,
            _ => {}
        }
        true
    })?;
    if let Some(message) = failure {
        return Err(format!("The Claude API reported an error: {message}"));
    }
    if refused {
        return Err("Claude declined this request.".into());
    }
    Ok(())
}

fn spawn_cli(program: &str, args: &[String], input: &str) -> Result<std::process::Child, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!("`{program}` isn't installed, or isn't on the PATH Null was started with.")
            } else {
                format!("Couldn't start `{program}`: {e}")
            }
        })?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let input = input.to_owned();
    // Write on another thread so a large prompt can't deadlock against a full stdout pipe.
    std::thread::spawn(move || {
        let _ = stdin.write_all(input.as_bytes());
    });
    Ok(child)
}

fn finish_cli(program: &str, mut child: std::process::Child) -> Result<(), String> {
    let mut stderr = String::new();
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut stderr);
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        return Ok(());
    }
    let detail: String =
        stderr.lines().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
    Err(format!("`{program}` stopped with an error. Is it signed in? {detail}").trim().to_string())
}

/// The person's own Claude Code, signed in with their Claude plan. All tools are
/// switched off: it only answers, and Null applies any change itself.
fn claude_code(model: Option<String>, prompt: &Prompt, on_text: &mut dyn FnMut(&str)) -> Result<(), String> {
    let mut args: Vec<String> = [
        "-p",
        "--tools",
        "",
        "--disallowedTools",
        "mcp__*",
        "--output-format",
        "stream-json",
        "--include-partial-messages",
        "--verbose",
        "--append-system-prompt",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.push(prompt.system.clone());
    if let Some(model) = model {
        args.extend(["--model".into(), model]);
    }
    let mut child = spawn_cli("claude", &args, &prompt.user)?;
    let stdout = child.stdout.take().expect("stdout is piped");
    let mut streamed = false;
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        let Ok(event) = serde_json::from_str::<Value>(&line) else { continue };
        match event["type"].as_str() {
            Some("stream_event") if event["event"]["delta"]["type"] == "text_delta" => {
                streamed = true;
                on_text(event["event"]["delta"]["text"].as_str().unwrap_or_default());
            }
            // Without partial messages, the full answer arrives once at the end.
            Some("result") if !streamed => on_text(event["result"].as_str().unwrap_or_default()),
            _ => {}
        }
    }
    finish_cli("claude", child)
}

/// The person's own Codex CLI, signed in with their ChatGPT plan, in a read-only
/// sandbox so it can't change files.
fn codex(model: Option<String>, prompt: &Prompt, on_text: &mut dyn FnMut(&str)) -> Result<(), String> {
    let mut args: Vec<String> = ["exec", "--sandbox", "read-only", "--skip-git-repo-check", "--ephemeral"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if let Some(model) = model {
        args.extend(["--model".into(), model]);
    }
    args.push("-".into());
    let input = format!("{}\n\n{}", prompt.system, prompt.user);
    let mut child = spawn_cli("codex", &args, &input)?;
    // Codex prints progress on stderr and only the final message on stdout.
    let mut answer = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut answer);
    }
    on_text(&answer);
    finish_cli("codex", child)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_fill_in_defaults_and_read_overrides() {
        let settings: AiSettings =
            serde_json::from_str(r#"{ "provider": "nvidia", "nvidia": { "model": "x/y" } }"#).unwrap();
        assert_eq!(settings.provider, ProviderId::Nvidia);
        assert_eq!(settings.model(ProviderId::Nvidia).as_deref(), Some("x/y"));
        assert_eq!(settings.base_url(ProviderId::Nvidia).as_deref(), Some("https://integrate.api.nvidia.com/v1"));
        assert_eq!(settings.model(ProviderId::Claude).as_deref(), Some("claude-opus-5-5"));
        assert_eq!(settings.model(ProviderId::ClaudeCode), None);
    }

    #[test]
    fn reads_sse_payloads() {
        let stream = "event: x\ndata: {\"a\":1}\n\ndata: [DONE]\ndata: after\n";
        let mut seen = Vec::new();
        read_sse(stream.as_bytes(), |d| {
            seen.push(d.to_string());
            d != "[DONE]"
        })
        .unwrap();
        assert_eq!(seen, vec!["{\"a\":1}", "[DONE]"]);
    }

    #[test]
    fn strips_markdown_fences() {
        assert_eq!(strip_code_fence("```rust\nfn a() {}\n```\n"), "fn a() {}");
        assert_eq!(strip_code_fence("fn a() {}"), "fn a() {}");
    }

    #[test]
    fn explains_provider_errors() {
        let message = error_message(401, r#"{"error":{"message":"bad key"}}"#);
        assert!(message.contains("refused") && message.contains("bad key"));
    }
}

/// Runs only on request, with a real key: `NVIDIA_API_KEY=... cargo test nvidia -- --ignored`.
#[cfg(test)]
mod live_tests {
    use super::*;

    #[test]
    #[ignore]
    fn nvidia_streams_an_answer() {
        let settings = AiSettings { provider: ProviderId::Nvidia, ..Default::default() };
        let prompt =
            Prompt { system: "Answer with one word.".into(), user: "What color is the sky on a clear day?".into() };
        let mut text = String::new();
        let mut chunks = 0;
        ask(&settings, &prompt, &mut |t| {
            chunks += 1;
            text.push_str(t)
        })
        .unwrap();
        println!("{chunks} chunks: {text:?}");
        assert!(text.to_lowercase().contains("blue"), "{text}");
    }
}
