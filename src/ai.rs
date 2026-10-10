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
    Mistral,
    Groq,
    Gemini,
    #[serde(rename = "openrouter")]
    OpenRouter,
}

/// A model worth picking, with what it's good for.
pub struct Recommended {
    pub model: &'static str,
    pub note: &'static str,
}

impl ProviderId {
    pub fn label(self) -> &'static str {
        match self {
            ProviderId::Off => "Off",
            ProviderId::Nvidia => "NVIDIA",
            ProviderId::Ollama => "Ollama",
            ProviderId::OpenaiCompatible => "OpenAI",
            ProviderId::Claude => "Claude API",
            ProviderId::ClaudeCode => "Claude Code",
            ProviderId::Codex => "Codex",
            ProviderId::Mistral => "Mistral",
            ProviderId::Groq => "Groq",
            ProviderId::Gemini => "Gemini",
            ProviderId::OpenRouter => "OpenRouter",
        }
    }

    /// One line on what it is and what it costs.
    pub fn description(self) -> &'static str {
        match self {
            ProviderId::Off => "No AI.",
            ProviderId::ClaudeCode => "Your Claude plan (Pro or Max), through the claude command line tool",
            ProviderId::Codex => "Your ChatGPT plan, through the codex command line tool",
            ProviderId::Claude => "The Anthropic API, paid per use, with an API key",
            ProviderId::OpenaiCompatible => "The OpenAI API, or any server that speaks it, with an API key",
            ProviderId::Mistral => "Free tier · Codestral makes the best suggestions while typing",
            ProviderId::Groq => "Free tier · very fast open models",
            ProviderId::Gemini => "Free tier on the Flash models",
            ProviderId::OpenRouter => "Free models (marked :free) and many paid ones, one key",
            ProviderId::Nvidia => "Free credits · NVIDIA's hosted open models",
            ProviderId::Ollama => "Models running on this computer. Free, private, no key",
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
            ProviderId::Mistral => "mistral",
            ProviderId::Groq => "groq",
            ProviderId::Gemini => "gemini",
            ProviderId::OpenRouter => "openrouter",
        }
    }

    /// The thinking levels this provider can be asked for (none: it can't be told).
    pub fn efforts(self) -> &'static [Effort] {
        use Effort::*;
        match self {
            ProviderId::Off | ProviderId::Ollama | ProviderId::Mistral => &[],
            // Nemotron's thinking is on or off.
            ProviderId::Nvidia => &[Off, Medium],
            // Opus 5.5 always thinks a little; "low" is the least.
            ProviderId::Claude => &[Low, Medium, High],
            ProviderId::Groq => &[Low, Medium, High],
            ProviderId::Gemini => &[Off, Low, Medium, High],
            ProviderId::OpenRouter => &[Auto, Off, Low, Medium, High],
            ProviderId::OpenaiCompatible | ProviderId::ClaudeCode | ProviderId::Codex => &[Auto, Low, Medium, High],
        }
    }

    /// Quick enough for small edits and short answers; raise it in Settings for harder work.
    pub fn default_effort(self) -> Effort {
        match self {
            // NVIDIA's thinking can turn a one-second edit into a thirty-second one.
            ProviderId::Nvidia | ProviderId::Gemini => Effort::Off,
            ProviderId::OpenaiCompatible => Effort::Auto,
            ProviderId::Off | ProviderId::Ollama | ProviderId::Mistral => Effort::Auto,
            _ => Effort::Low,
        }
    }

    /// Whether this provider takes an API key (Ollama and the CLIs don't).
    pub fn uses_api_key(self) -> bool {
        !matches!(self, ProviderId::Off | ProviderId::Ollama | ProviderId::ClaudeCode | ProviderId::Codex)
    }

    fn key_env(self) -> Option<&'static str> {
        match self {
            ProviderId::Nvidia => Some("NVIDIA_API_KEY"),
            ProviderId::OpenaiCompatible => Some("OPENAI_API_KEY"),
            ProviderId::Claude => Some("ANTHROPIC_API_KEY"),
            ProviderId::Mistral => Some("MISTRAL_API_KEY"),
            ProviderId::Groq => Some("GROQ_API_KEY"),
            ProviderId::Gemini => Some("GEMINI_API_KEY"),
            ProviderId::OpenRouter => Some("OPENROUTER_API_KEY"),
            _ => None,
        }
    }

    /// Where to get a key, for the key prompt.
    pub fn key_url(self) -> Option<&'static str> {
        match self {
            ProviderId::Nvidia => Some("build.nvidia.com"),
            ProviderId::OpenaiCompatible => Some("platform.openai.com/api-keys"),
            ProviderId::Claude => Some("console.anthropic.com"),
            ProviderId::Mistral => Some("console.mistral.ai"),
            ProviderId::Groq => Some("console.groq.com/keys"),
            ProviderId::Gemini => Some("aistudio.google.com/apikey"),
            ProviderId::OpenRouter => Some("openrouter.ai/keys"),
            _ => None,
        }
    }

    pub fn default_base_url(self) -> Option<&'static str> {
        match self {
            ProviderId::Nvidia => Some("https://integrate.api.nvidia.com/v1"),
            ProviderId::Ollama => Some("http://localhost:11434/v1"),
            ProviderId::OpenaiCompatible => Some("https://api.openai.com/v1"),
            ProviderId::Mistral => Some("https://api.mistral.ai/v1"),
            ProviderId::Groq => Some("https://api.groq.com/openai/v1"),
            ProviderId::Gemini => Some("https://generativelanguage.googleapis.com/v1beta/openai"),
            ProviderId::OpenRouter => Some("https://openrouter.ai/api/v1"),
            _ => None,
        }
    }

    pub fn default_model(self) -> Option<&'static str> {
        match self {
            // The command line tools use the model chosen in them, unless one is set here.
            ProviderId::ClaudeCode | ProviderId::Codex => None,
            _ => self.recommended().first().map(|r| r.model),
        }
    }

    /// Models worth picking for answers and edits, the default first.
    pub fn recommended(self) -> &'static [Recommended] {
        match self {
            ProviderId::Off => &[],
            ProviderId::Claude => &[
                Recommended { model: "claude-opus-5-5", note: "strongest" },
                Recommended { model: "claude-sonnet-5-5", note: "balanced" },
                Recommended { model: "claude-haiku-4-5", note: "fastest" },
            ],
            ProviderId::ClaudeCode => &[
                Recommended { model: "sonnet", note: "balanced" },
                Recommended { model: "opus", note: "strongest" },
                Recommended { model: "haiku", note: "fastest" },
            ],
            ProviderId::Codex | ProviderId::OpenaiCompatible => &[
                Recommended { model: "gpt-6.1-sol", note: "balanced" },
                Recommended { model: "gpt-6-astra", note: "strongest" },
                Recommended { model: "gpt-6-luna", note: "fastest" },
            ],
            ProviderId::Mistral => &[
                Recommended { model: "mistral-medium-latest", note: "strongest" },
                Recommended { model: "mistral-small-latest", note: "fast" },
            ],
            ProviderId::Groq => &[
                Recommended { model: "openai/gpt-oss-120b", note: "strongest" },
                Recommended { model: "llama-3.3-70b-versatile", note: "balanced" },
                Recommended { model: "openai/gpt-oss-20b", note: "fastest" },
            ],
            ProviderId::Gemini => &[
                Recommended { model: "gemini-3.8-flash", note: "balanced, free" },
                Recommended { model: "gemini-3.1-pro-preview", note: "strongest, paid" },
                Recommended { model: "gemini-3.5-flash-lite", note: "fastest, free" },
            ],
            ProviderId::OpenRouter => &[
                Recommended { model: "qwen/qwen3.8-27b:free", note: "free" },
                Recommended { model: "nvidia/nemotron-3-super-120b-a12b:free", note: "free" },
                Recommended { model: "poolside/laguna-s-2.1:free", note: "free, for code" },
            ],
            ProviderId::Nvidia => &[
                Recommended { model: "nvidia/nemotron-3-super-120b-a12b", note: "balanced" },
                Recommended { model: "deepseek-ai/deepseek-v4.1-flash", note: "fast" },
                Recommended { model: "mistralai/codestral-22b-instruct-v0.1", note: "for code" },
            ],
            ProviderId::Ollama => &[
                Recommended { model: "qwen2.5-coder:7b", note: "for code" },
                Recommended { model: "qwen2.5-coder:14b", note: "stronger, slower" },
            ],
        }
    }

    /// Models worth picking for suggestions while typing, the default first. Fill-in-the-middle
    /// code models where the provider has them: fast, and they continue the code.
    pub fn recommended_for_suggestions(self) -> &'static [Recommended] {
        match self {
            ProviderId::Nvidia => &[
                Recommended { model: "bigcode/starcoder2-15b", note: "fills in code" },
                Recommended { model: "google/codegemma-7b", note: "fills in code, faster" },
            ],
            ProviderId::Ollama => &[
                Recommended { model: "qwen2.5-coder:1.5b", note: "fills in code" },
                Recommended { model: "qwen2.5-coder:7b", note: "better, slower" },
            ],
            ProviderId::Mistral => &[Recommended { model: "codestral-latest", note: "fills in code" }],
            ProviderId::Claude => &[Recommended { model: "claude-haiku-4-5", note: "fastest" }],
            ProviderId::Groq => &[
                Recommended { model: "llama-3.1-8b-instant", note: "fastest" },
                Recommended { model: "openai/gpt-oss-20b", note: "smarter" },
            ],
            ProviderId::Gemini => &[Recommended { model: "gemini-3.5-flash-lite", note: "fastest" }],
            ProviderId::OpenRouter => &[Recommended { model: "cohere/north-mini-code:free", note: "free, for code" }],
            ProviderId::OpenaiCompatible => &[Recommended { model: "gpt-6-luna", note: "fastest" }],
            ProviderId::Off | ProviderId::ClaudeCode | ProviderId::Codex => &[],
        }
    }
}

/// How much a model thinks before answering: more is slower and, for hard
/// questions, better. Not every provider has every level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    /// Send nothing: the model decides.
    Auto,
    Off,
    Low,
    Medium,
    High,
}

impl Effort {
    pub fn label(self) -> &'static str {
        match self {
            Effort::Auto => "Model's default",
            Effort::Off => "Off",
            Effort::Low => "Low",
            Effort::Medium => "Medium",
            Effort::High => "High",
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Effort::Auto => "auto",
            Effort::Off => "off",
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
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
    /// A faster model for suggestions while typing; the usual one when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_model: Option<String>,
    /// How much the model thinks first; the provider's default when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,
    /// The older on/off switch for thinking, still read from existing settings files.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    /// The master switch: when off, no AI appears anywhere, whatever the provider.
    pub enabled: bool,
    pub provider: ProviderId,
    /// Ghost completions: the AI's guess at what comes next, shown faintly at the caret.
    pub completions: bool,
    #[serde(flatten)]
    pub providers: HashMap<String, ProviderSettings>,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self { enabled: true, provider: ProviderId::Off, completions: false, providers: HashMap::new() }
    }
}

impl AiSettings {
    /// The provider to use, or Off when AI is switched off.
    pub fn active(&self) -> ProviderId {
        if self.enabled { self.provider } else { ProviderId::Off }
    }

    pub fn model(&self, id: ProviderId) -> Option<String> {
        self.providers
            .get(id.key())
            .and_then(|p| p.model.clone())
            .filter(|m| !m.is_empty())
            .or(id.default_model().map(str::to_owned))
    }

    /// The model for suggestions while typing: the one set for them, else a fast default.
    pub fn completion_model(&self, id: ProviderId) -> Option<String> {
        self.providers
            .get(id.key())
            .and_then(|p| p.completion_model.clone())
            .filter(|m| !m.is_empty())
            .or_else(|| id.recommended_for_suggestions().first().map(|r| r.model.to_string()))
    }

    /// How much the model thinks: the level set for this provider (if it offers it),
    /// else its default.
    pub fn effort(&self, id: ProviderId) -> Effort {
        let saved = self.providers.get(id.key());
        saved
            .and_then(|p| p.effort.or(p.reasoning.map(|on| if on { Effort::Medium } else { Effort::Off })))
            .filter(|e| id.efforts().contains(e))
            .unwrap_or(id.default_effort())
    }

    pub(crate) fn base_url(&self, id: ProviderId) -> Option<String> {
        self.providers
            .get(id.key())
            .and_then(|p| p.base_url.clone())
            .filter(|u| !u.is_empty())
            .or(id.default_base_url().map(str::to_owned))
    }
}

const KEYCHAIN_SERVICE: &str = "Null IDE";

/// Keys read from the keychain this session, by provider. Reading the keychain can make
/// macOS ask for permission (always, for a freshly built unsigned binary), so it's read
/// at most once per launch, and only when a request needs the key.
static KEYS: std::sync::Mutex<Option<HashMap<&'static str, Option<String>>>> = std::sync::Mutex::new(None);

fn env_key(id: ProviderId) -> Option<String> {
    id.key_env().and_then(|v| std::env::var(v).ok()).filter(|k| !k.trim().is_empty())
}

/// The provider's key: its usual environment variable first, then the keychain.
pub fn api_key(id: ProviderId) -> Option<String> {
    if let Some(key) = env_key(id) {
        return Some(key);
    }
    let mut keys = KEYS.lock().unwrap_or_else(|e| e.into_inner());
    keys.get_or_insert_with(HashMap::new)
        .entry(id.key())
        .or_insert_with(|| {
            keyring::Entry::new(KEYCHAIN_SERVICE, id.key())
                .ok()
                .and_then(|e| e.get_password().ok())
                .filter(|k| !k.trim().is_empty())
        })
        .clone()
}

/// Whether a key is known to be set, without touching the keychain: None when it
/// hasn't been read yet this session.
pub fn known_key(id: ProviderId) -> Option<bool> {
    if env_key(id).is_some() {
        return Some(true);
    }
    let keys = KEYS.lock().unwrap_or_else(|e| e.into_inner());
    keys.as_ref().and_then(|k| k.get(id.key())).map(Option::is_some)
}

pub fn store_api_key(id: ProviderId, key: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, id.key()).map_err(|e| e.to_string())?;
    let key = key.trim();
    if key.is_empty() { entry.delete_credential().or(Ok(())) } else { entry.set_password(key) }
        .map_err(|e: keyring::Error| e.to_string())?;
    let mut keys = KEYS.lock().unwrap_or_else(|e| e.into_inner());
    keys.get_or_insert_with(HashMap::new).insert(id.key(), (!key.is_empty()).then(|| key.to_string()));
    Ok(())
}

#[derive(Clone, Default)]
pub struct Prompt {
    pub system: String,
    pub user: String,
    /// A different model than the provider's usual one (a fast one for suggestions).
    pub model: Option<String>,
    /// Caps the answer's length, which also makes it come back sooner.
    pub max_tokens: Option<u32>,
    /// A different thinking level than the provider's (the least, for suggestions).
    pub effort: Option<Effort>,
    /// More varied answers (for another option); the usual when unset.
    pub temperature: Option<f32>,
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
    // Text hidden in characters that show nothing isn't the person's to give the model.
    let prompt = &Prompt {
        system: crate::editor::invisible::without_hidden(&prompt.system).0,
        user: crate::editor::invisible::without_hidden(&prompt.user).0,
        ..prompt.clone()
    };
    let started = std::time::Instant::now();
    let mut first = None;
    let result = ask_inner(settings, prompt, &mut |text| {
        first.get_or_insert_with(|| started.elapsed());
        on_text(text)
    });
    log_request(settings, "ask", started, first, &result);
    result
}

fn ask_inner(settings: &AiSettings, prompt: &Prompt, on_text: &mut dyn FnMut(&str)) -> Result<(), String> {
    let id = settings.active();
    let effort = prompt.effort.filter(|e| id.efforts().contains(e)).unwrap_or(settings.effort(id));
    let model = prompt.model.clone().or_else(|| settings.model(id));
    match id {
        ProviderId::Off => Err("AI is off. Turn it on in Settings → AI (⌘,).".into()),
        ProviderId::Claude => {
            let key = api_key(id).ok_or_else(|| missing_key(id))?;
            claude_api(&model.unwrap_or_default(), &key, effort, prompt, on_text)
        }
        ProviderId::ClaudeCode => claude_code(model, effort, prompt, on_text),
        ProviderId::Codex => codex(model, effort, prompt, on_text),
        _ => {
            let base = settings.base_url(id).unwrap_or_default();
            let model = model.ok_or_else(|| format!("Choose a model for {} in Settings → AI.", id.label()))?;
            let key = api_key(id);
            // A custom OpenAI-compatible server may not need a key; OpenAI's own does.
            let keyless =
                id == ProviderId::Ollama || (id == ProviderId::OpenaiCompatible && !base.contains("api.openai.com"));
            if key.is_none() && !keyless {
                return Err(missing_key(id));
            }
            openai_compatible(id, &base, &model, key.as_deref(), effort, prompt, on_text)
        }
    }
}

pub(crate) fn missing_key(id: ProviderId) -> String {
    let get = id.key_url().map(|u| format!(" (get one at {u})")).unwrap_or_default();
    format!("No API key for {}{get}. Add it in Settings → AI.", id.label())
}

pub(crate) fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).build().into()
}

/// The message inside a provider's error response, or the raw body.
pub(crate) fn error_message(status: u16, body: &str) -> String {
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

/// The request for an OpenAI-style chat, with each service's own rules for length,
/// temperature and thinking.
fn chat_body(id: ProviderId, base: &str, model: &str, effort: Effort, prompt: &Prompt) -> Value {
    let mut body = json!({
        "model": model,
        "stream": true,
        "messages": [
            { "role": "system", "content": prompt.system },
            { "role": "user", "content": prompt.user },
        ],
    });
    let max_tokens = prompt.max_tokens.unwrap_or(4096);
    // OpenAI's own reasoning models reject `temperature` and the old `max_tokens`.
    let openai = id == ProviderId::OpenaiCompatible && base.contains("api.openai.com");
    if openai {
        body["max_completion_tokens"] = json!(max_tokens);
    } else {
        body["max_tokens"] = json!(max_tokens);
        if !matches!(id, ProviderId::OpenRouter | ProviderId::OpenaiCompatible) {
            body["temperature"] = json!(prompt.temperature.unwrap_or(0.2));
        }
    }
    // Each service asks for thinking its own way; unknown fields can be refused, so
    // nothing is sent where it isn't documented.
    let level = |e: Effort, off: &'static str| if e == Effort::Off { off } else { e.key() };
    match (id, effort) {
        (_, Effort::Auto) => {}
        (ProviderId::Nvidia, e) => body["chat_template_kwargs"] = json!({ "enable_thinking": e != Effort::Off }),
        (ProviderId::OpenRouter, e) => body["reasoning"] = json!({ "effort": level(e, "none") }),
        (ProviderId::Gemini, e) => body["reasoning_effort"] = json!(level(e, "minimal")),
        // On Groq only the reasoning models take it, from "low".
        (ProviderId::Groq, e) if model.contains("gpt-oss") || model.contains("qwen3") => {
            body["reasoning_effort"] = json!(level(e, "low"))
        }
        (ProviderId::OpenaiCompatible, e) if e != Effort::Off => body["reasoning_effort"] = json!(e.key()),
        _ => {}
    }
    body
}

fn openai_compatible(
    id: ProviderId,
    base: &str,
    model: &str,
    key: Option<&str>,
    effort: Effort,
    prompt: &Prompt,
    on_text: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let mut request = agent().post(&url).header("Content-Type", "application/json");
    if let Some(key) = key {
        request = request.header("Authorization", &format!("Bearer {key}"));
    }
    let body = chat_body(id, base, model, effort, prompt);
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
/// The request for Claude's Messages API.
fn claude_body(model: &str, effort: Effort, prompt: &Prompt) -> Value {
    let mut body = json!({
        "model": model,
        "max_tokens": prompt.max_tokens.unwrap_or(16000),
        "stream": true,
        "fallbacks": "default",
        "system": prompt.system,
        "messages": [{ "role": "user", "content": prompt.user }],
    });
    // Opus and Sonnet take an effort level; Haiku 4.5 refuses the field (and thinks only
    // when given a token budget, which suggestions don't want).
    if !model.contains("haiku") && effort != Effort::Auto {
        let level = if effort == Effort::Off { "low" } else { effort.key() };
        body["output_config"] = json!({ "effort": level });
    }
    body
}

fn claude_api(
    model: &str,
    key: &str,
    effort: Effort,
    prompt: &Prompt,
    on_text: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let body = claude_body(model, effort, prompt);
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

/// Where a command line tool is installed, if it is.
pub fn find_cli(name: &str) -> Option<std::path::PathBuf> {
    crate::tools::find(name)
}

/// How to get a subscription's command line tool, for when it's missing.
pub fn install_hint(id: ProviderId) -> Option<&'static str> {
    match id {
        ProviderId::ClaudeCode => Some(
            "Install Claude Code with `curl -fsSL https://claude.ai/install.sh | bash`, then run `claude` once to sign in with your Claude plan.",
        ),
        ProviderId::Codex => Some(
            "Install Codex with `npm install -g @openai/codex`, then run `codex` once to sign in with your ChatGPT plan.",
        ),
        _ => None,
    }
}

fn spawn_cli(program: &str, args: &[String], input: &str) -> Result<std::process::Child, String> {
    spawn_cli_in(program, args, input, None)
}

fn spawn_cli_in(
    program: &str,
    args: &[String],
    input: &str,
    dir: Option<&std::path::Path>,
) -> Result<std::process::Child, String> {
    let id = if program == "claude" { ProviderId::ClaudeCode } else { ProviderId::Codex };
    let path = find_cli(program)
        .ok_or_else(|| format!("`{program}` isn't installed. {}", install_hint(id).unwrap_or_default()))?;
    // Tools installed with npm start with `#!/usr/bin/env node`: give them a PATH that finds node.
    let search_path = crate::tools::search_path();
    let mut command = Command::new(&path);
    command.env("PATH", search_path);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    // Claude Code bills an ANTHROPIC_API_KEY over the person's plan when one is set: this
    // provider is about the plan.
    if id == ProviderId::ClaudeCode {
        command.env_remove("ANTHROPIC_API_KEY");
    }
    let mut child =
        command.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(
            |e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    format!("`{program}` isn't installed, or isn't on the PATH Null was started with.")
                } else {
                    format!("Couldn't start `{program}`: {e}")
                }
            },
        )?;
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
fn claude_code(
    model: Option<String>,
    effort: Effort,
    prompt: &Prompt,
    on_text: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let mut args: Vec<String> = [
        "-p",
        // Don't add Null's requests to the person's own Claude Code history.
        "--no-session-persistence",
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
    if !matches!(effort, Effort::Auto | Effort::Off) {
        args.extend(["--effort".into(), effort.key().into()]);
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
fn codex(model: Option<String>, effort: Effort, prompt: &Prompt, on_text: &mut dyn FnMut(&str)) -> Result<(), String> {
    let mut args: Vec<String> = ["exec", "--sandbox", "read-only", "--skip-git-repo-check", "--ephemeral"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if let Some(model) = model {
        args.extend(["--model".into(), model]);
    }
    if !matches!(effort, Effort::Auto | Effort::Off) {
        args.extend(["-c".into(), format!("model_reasoning_effort=\"{}\"", effort.key())]);
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

// ---------- tasks ----------

/// What a task reports while it works.
pub enum TaskEvent {
    /// It's at a file (reading or changing it), relative to the project when it can be.
    File(String),
}

/// Whether the provider can carry out a task: Claude Code and Codex by themselves, the
/// others through Null's own tools (see `ai_agent`).
pub fn can_run_tasks(id: ProviderId) -> bool {
    id != ProviderId::Off
}

/// Hands `task` to the person's Claude Code or Codex, working in `root`: they read and
/// change files there themselves (no shell commands for Claude Code; Codex stays in its
/// workspace sandbox). Blocks until done; returns what the tool said at the end.
/// `child` holds the running process, so it can be stopped from elsewhere.
pub fn run_task(
    settings: &AiSettings,
    root: &std::path::Path,
    task: &str,
    child: &std::sync::Mutex<Option<std::process::Child>>,
    stop: &std::sync::atomic::AtomicBool,
    on_event: &mut dyn FnMut(TaskEvent),
) -> Result<String, String> {
    let id = settings.active();
    if !matches!(id, ProviderId::ClaudeCode | ProviderId::Codex) {
        return crate::ai_agent::run(settings, root, task, stop, on_event);
    }
    let model = settings.model(id);
    let effort = settings.effort(id);
    let system = "You are working on the person's project from inside their code editor, Null. Make the change \
                  they ask for by editing the project's files directly. Keep changes focused on the task. Every \
                  change will be reviewed in the editor before it's kept, so don't ask for confirmation. When done, \
                  say in one or two sentences what you changed.";
    let relative = |path: &str| -> String {
        let p = std::path::Path::new(path);
        p.strip_prefix(root).unwrap_or(p).display().to_string()
    };
    let (program, args, input) = match id {
        ProviderId::ClaudeCode => {
            let mut args: Vec<String> = [
                "-p",
                "--no-session-persistence",
                "--permission-mode",
                "acceptEdits",
                "--allowedTools",
                "Read,Edit,MultiEdit,Write,Glob,Grep,LS",
                "--disallowedTools",
                "Bash,WebFetch,WebSearch,mcp__*",
                "--output-format",
                "stream-json",
                "--verbose",
                "--append-system-prompt",
                system,
            ]
            .iter()
            .map(|s| s.to_string())
            .collect();
            if let Some(model) = model {
                args.extend(["--model".into(), model]);
            }
            if !matches!(effort, Effort::Auto | Effort::Off) {
                args.extend(["--effort".into(), effort.key().into()]);
            }
            ("claude", args, task.to_string())
        }
        ProviderId::Codex => {
            let mut args: Vec<String> =
                ["exec", "--sandbox", "workspace-write", "--skip-git-repo-check", "--ephemeral", "--json"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
            if let Some(model) = model {
                args.extend(["--model".into(), model]);
            }
            if !matches!(effort, Effort::Auto | Effort::Off) {
                args.extend(["-c".into(), format!("model_reasoning_effort=\"{}\"", effort.key())]);
            }
            args.push("-".into());
            ("codex", args, format!("{system}\n\nTask: {task}"))
        }
        _ => unreachable!("API providers run through ai_agent"),
    };
    let mut process = spawn_cli_in(program, &args, &input, Some(root))?;
    let stdout = process.stdout.take().expect("stdout is piped");
    // Read on its own thread: a tool chatty on stderr would otherwise fill the pipe and stall.
    let stderr = process.stderr.take().map(|mut err| {
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = err.read_to_string(&mut text);
            text
        })
    });
    *child.lock().unwrap_or_else(|e| e.into_inner()) = Some(process);
    let mut summary = String::new();
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        let Ok(event) = serde_json::from_str::<Value>(&line) else { continue };
        match program {
            // Claude Code: tool calls name the file; the result says what was done.
            "claude" => match event["type"].as_str() {
                Some("assistant") => {
                    for part in event["message"]["content"].as_array().into_iter().flatten() {
                        if part["type"] == "tool_use"
                            && let Some(path) = part["input"]["file_path"].as_str().or(part["input"]["path"].as_str())
                        {
                            on_event(TaskEvent::File(relative(path)));
                        }
                    }
                }
                Some("result") => summary = event["result"].as_str().unwrap_or_default().to_string(),
                _ => {}
            },
            // Codex: items for file changes and messages (read loosely: the format moves).
            _ => {
                let item = &event["item"];
                let kind = item["type"].as_str().unwrap_or_default();
                if kind.contains("file") || kind.contains("patch") {
                    for change in item["changes"].as_array().into_iter().flatten() {
                        if let Some(path) = change["path"].as_str() {
                            on_event(TaskEvent::File(relative(path)));
                        }
                    }
                } else if kind.contains("message")
                    && let Some(text) = item["text"].as_str()
                {
                    summary = text.to_string();
                }
            }
        }
    }
    let process = child.lock().unwrap_or_else(|e| e.into_inner()).take();
    let Some(mut process) = process else { return Err("Stopped.".into()) };
    let errors = stderr.and_then(|t| t.join().ok()).unwrap_or_default();
    let status = process.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(summary.trim().to_string())
    } else {
        let detail: Vec<&str> = errors.lines().rev().take(3).collect();
        Err(format!(
            "`{program}` stopped with an error. Is it signed in? {}",
            detail.into_iter().rev().collect::<Vec<_>>().join(" ")
        )
        .trim()
        .to_string())
    }
}

// ---------- fill in the middle ----------

/// The code around the caret, for a model trained to write what goes between.
#[derive(Clone)]
pub struct Fim {
    pub prefix: String,
    pub suffix: String,
    pub max_tokens: u32,
    pub temperature: f32,
}

/// How a code model expects the text before and after the gap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FimFormat {
    StarCoder,
    /// CodeGemma and Qwen2.5-Coder.
    Pipes,
    DeepSeek,
    CodeLlama,
    Codestral,
}

impl FimFormat {
    fn for_model(model: &str) -> Option<Self> {
        let m = model.to_lowercase();
        Some(if m.contains("starcoder") || m.contains("santacoder") {
            FimFormat::StarCoder
        } else if m.contains("codegemma") || (m.contains("qwen") && m.contains("coder")) {
            FimFormat::Pipes
        } else if m.contains("deepseek-coder") {
            FimFormat::DeepSeek
        } else if m.contains("codellama") {
            FimFormat::CodeLlama
        } else if m.contains("codestral") {
            FimFormat::Codestral
        } else {
            return None;
        })
    }

    fn prompt(self, prefix: &str, suffix: &str) -> String {
        match self {
            FimFormat::StarCoder => format!("<fim_prefix>{prefix}<fim_suffix>{suffix}<fim_middle>"),
            FimFormat::Pipes => format!("<|fim_prefix|>{prefix}<|fim_suffix|>{suffix}<|fim_middle|>"),
            FimFormat::DeepSeek => format!("<｜fim▁begin｜>{prefix}<｜fim▁hole｜>{suffix}<｜fim▁end｜>"),
            FimFormat::CodeLlama => format!("<PRE> {prefix} <SUF>{suffix} <MID>"),
            FimFormat::Codestral => format!("[SUFFIX]{suffix}[PREFIX]{prefix}"),
        }
    }
}

/// Markers code models end a fill with, or start the next file with.
const FIM_STOPS: [&str; 4] = ["<|endoftext|>", "<file_sep>", "<|file_separator|>", "<EOT>"];

/// Whether suggestions can use a fill-in-the-middle model with these settings.
pub fn fim_available(settings: &AiSettings) -> bool {
    let id = settings.active();
    match id {
        ProviderId::Ollama => true,
        ProviderId::Mistral => settings.completion_model(id).is_some_and(|m| m.contains("codestral")),
        ProviderId::Nvidia | ProviderId::OpenaiCompatible => {
            settings.completion_model(id).is_some_and(|m| FimFormat::for_model(&m).is_some())
        }
        _ => false,
    }
}

/// Like [`stream`], for a fill: the text that goes between `prefix` and `suffix`.
pub fn stream_fim(settings: AiSettings, fim: Fim) -> futures::channel::mpsc::UnboundedReceiver<AiEvent> {
    let (tx, rx) = futures::channel::mpsc::unbounded();
    std::thread::Builder::new()
        .name("ai-fill".into())
        .spawn(move || {
            let started = std::time::Instant::now();
            let text_tx = tx.clone();
            let mut first = None;
            // (Hidden text left out, as from any request.)
            let fim = Fim {
                prefix: crate::editor::invisible::without_hidden(&fim.prefix).0,
                suffix: crate::editor::invisible::without_hidden(&fim.suffix).0,
                ..fim
            };
            let result = fill(&settings, &fim, &mut |text| {
                first.get_or_insert_with(|| started.elapsed());
                text_tx.unbounded_send(AiEvent::Text(text.to_string())).ok();
            });
            log_request(&settings, "fill", started, first, &result);
            tx.unbounded_send(match result {
                Ok(()) => AiEvent::Done,
                Err(message) => AiEvent::Failed(message),
            })
            .ok();
        })
        .ok();
    rx
}

/// With `NULL_AI_LOG=<file>`, one line per request: how long until the first text, how long in all.
pub fn log_request(
    settings: &AiSettings,
    kind: &str,
    started: std::time::Instant,
    first: Option<std::time::Duration>,
    result: &Result<(), String>,
) {
    let Some(path) = std::env::var_os("NULL_AI_LOG") else { return };
    let id = settings.active();
    let model = settings.completion_model(id).or_else(|| settings.model(id)).unwrap_or_default();
    let line = format!(
        "{kind} {} {model}: first text {}, total {:.0?}{}\n",
        id.key(),
        first.map_or("never".to_string(), |d| format!("{d:.0?}")),
        started.elapsed(),
        result.as_ref().err().map(|e| format!(", failed: {e}")).unwrap_or_default(),
    );
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        file.write_all(line.as_bytes()).ok();
    }
}

fn fill(settings: &AiSettings, fim: &Fim, on_text: &mut dyn FnMut(&str)) -> Result<(), String> {
    let id = settings.active();
    let model = settings.completion_model(id).ok_or("No model for suggestions")?;
    let base = settings.base_url(id).unwrap_or_default();
    match id {
        // Ollama formats the fill itself, from the model's own template.
        ProviderId::Ollama => {
            let host = base.trim_end_matches('/').trim_end_matches("/v1").to_string();
            let body = json!({
                "model": model,
                "prompt": fim.prefix,
                "suffix": fim.suffix,
                "stream": true,
                "options": { "num_predict": fim.max_tokens, "temperature": fim.temperature, "stop": FIM_STOPS },
            });
            let response = agent()
                .post(&format!("{host}/api/generate"))
                .send_json(&body)
                .map_err(|e| format!("Couldn't reach Ollama at {host}. Is it running? ({e})"))?;
            let status = response.status().as_u16();
            let mut body = response.into_body();
            if status >= 400 {
                return Err(error_message(status, &body.read_to_string().unwrap_or_default()));
            }
            for line in BufReader::new(body.into_reader()).lines() {
                let line = line.map_err(|e| format!("The connection dropped: {e}"))?;
                let Ok(event) = serde_json::from_str::<Value>(&line) else { continue };
                if let Some(text) = event["response"].as_str() {
                    on_text(text);
                }
                if event["done"].as_bool() == Some(true) {
                    break;
                }
            }
            Ok(())
        }
        // Mistral has an endpoint just for this: the code before and after, as they are.
        ProviderId::Mistral => {
            let key = api_key(id).ok_or_else(|| missing_key(id))?;
            let body = json!({
                "model": model,
                "prompt": fim.prefix,
                "suffix": fim.suffix,
                "max_tokens": fim.max_tokens,
                "temperature": fim.temperature,
                "stream": true,
            });
            let response = agent()
                .post(&format!("{}/fim/completions", base.trim_end_matches('/')))
                .header("Content-Type", "application/json")
                .header("Authorization", &format!("Bearer {key}"))
                .send_json(&body)
                .map_err(|e| format!("Couldn't reach Mistral: {e}"))?;
            let status = response.status().as_u16();
            let mut body = response.into_body();
            if status >= 400 {
                return Err(error_message(status, &body.read_to_string().unwrap_or_default()));
            }
            read_sse(body.into_reader(), |data| {
                if data == "[DONE]" {
                    return false;
                }
                if let Ok(event) = serde_json::from_str::<Value>(data) {
                    let choice = &event["choices"][0];
                    if let Some(text) = choice["delta"]["content"].as_str().or(choice["text"].as_str()) {
                        on_text(text);
                    }
                }
                true
            })
        }
        ProviderId::Nvidia | ProviderId::OpenaiCompatible => {
            let format = FimFormat::for_model(&model).ok_or("Not a fill-in-the-middle model")?;
            let key = api_key(id);
            if key.is_none() && id == ProviderId::Nvidia {
                return Err(missing_key(id));
            }
            let body = json!({
                "model": model,
                "prompt": format.prompt(&fim.prefix, &fim.suffix),
                "max_tokens": fim.max_tokens,
                "temperature": fim.temperature,
                "stream": true,
                "stop": FIM_STOPS,
            });
            let mut request = agent()
                .post(&format!("{}/completions", base.trim_end_matches('/')))
                .header("Content-Type", "application/json");
            if let Some(key) = &key {
                request = request.header("Authorization", &format!("Bearer {key}"));
            }
            let response = request.send_json(&body).map_err(|e| format!("Couldn't reach {}: {e}", id.label()))?;
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
                    && let Some(text) = event["choices"][0]["text"].as_str()
                {
                    on_text(text);
                }
                true
            })
        }
        _ => Err("Not a fill-in-the-middle provider".into()),
    }
}

#[cfg(test)]
mod request_tests {
    use super::*;

    fn prompt() -> Prompt {
        Prompt { system: "s".into(), user: "u".into(), ..Default::default() }
    }

    /// What each service's docs say about length, temperature and thinking.
    #[test]
    fn chat_requests_follow_each_service() {
        let openai =
            chat_body(ProviderId::OpenaiCompatible, "https://api.openai.com/v1", "gpt-6.1-sol", Effort::Low, &prompt());
        assert!(openai.get("temperature").is_none() && openai.get("max_tokens").is_none());
        assert_eq!(openai["max_completion_tokens"], 4096);
        assert_eq!(openai["reasoning_effort"], "low");

        let auto = chat_body(ProviderId::OpenaiCompatible, "http://localhost:8000/v1", "m", Effort::Auto, &prompt());
        assert!(auto.get("reasoning_effort").is_none());

        let nvidia = chat_body(ProviderId::Nvidia, "", "nvidia/nemotron-3-super-120b-a12b", Effort::Off, &prompt());
        assert_eq!(nvidia["chat_template_kwargs"]["enable_thinking"], false);

        // Groq's non-reasoning models don't take the field.
        let llama = chat_body(ProviderId::Groq, "", "llama-3.3-70b-versatile", Effort::Low, &prompt());
        assert!(llama.get("reasoning_effort").is_none());
        let oss = chat_body(ProviderId::Groq, "", "openai/gpt-oss-120b", Effort::Medium, &prompt());
        assert_eq!(oss["reasoning_effort"], "medium");

        assert_eq!(chat_body(ProviderId::Gemini, "", "g", Effort::Off, &prompt())["reasoning_effort"], "minimal");
        assert_eq!(chat_body(ProviderId::OpenRouter, "", "o", Effort::Off, &prompt())["reasoning"]["effort"], "none");
    }

    #[test]
    fn claude_requests_follow_each_model() {
        assert_eq!(claude_body("claude-opus-5-5", Effort::High, &prompt())["output_config"]["effort"], "high");
        // Haiku 4.5 refuses an effort level.
        assert!(claude_body("claude-haiku-4-5", Effort::Low, &prompt()).get("output_config").is_none());
        // No sampling settings: Opus 5.5 refuses them.
        assert!(claude_body("claude-opus-5-5", Effort::Low, &prompt()).get("temperature").is_none());
    }

    #[test]
    fn every_provider_has_a_default_model_and_valid_thinking_default() {
        for id in [
            ProviderId::Nvidia,
            ProviderId::Ollama,
            ProviderId::OpenaiCompatible,
            ProviderId::Claude,
            ProviderId::ClaudeCode,
            ProviderId::Codex,
            ProviderId::Mistral,
            ProviderId::Groq,
            ProviderId::Gemini,
            ProviderId::OpenRouter,
        ] {
            assert!(id.default_model().is_some() || matches!(id, ProviderId::ClaudeCode | ProviderId::Codex), "{id:?}");
            let efforts = id.efforts();
            assert!(efforts.is_empty() || efforts.contains(&id.default_effort()), "{id:?}");
        }
    }
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
        let prompt = Prompt {
            system: "Answer with one word.".into(),
            user: "What color is the sky on a clear day?".into(),
            ..Default::default()
        };
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
