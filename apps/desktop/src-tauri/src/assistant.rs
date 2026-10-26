use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{Emitter, State};

use crate::app::Shared;

const OLLAMA_CONTEXT: u32 = 16384;
const MAX_ROUNDS: usize = 10;
const ABOUT: &str = include_str!("assistant_about.md");
const README_CHUNK: usize = 6000;
const STREAM_IDLE: Duration = Duration::from_secs(180);

fn key_name(provider: &str) -> Result<String, String> {
    if !["openai", "anthropic", "gemini", "openai-compatible"].contains(&provider) {
        return Err("That provider does not use an API key.".into());
    }
    Ok(format!("assistant_api_key_{provider}"))
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    kind: String,
    #[serde(default)]
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ok: Option<bool>,
}

impl Part {
    fn new(kind: &str) -> Self {
        Part {
            kind: kind.into(),
            ..Part::default()
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Turn {
    role: String,
    content: String,
    #[serde(default)]
    project_ids: Vec<String>,
    #[serde(default)]
    artifact_urls: Vec<Option<String>>,
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    parts: Vec<Part>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    elapsed_ms: Option<u64>,
    #[serde(default)]
    stopped: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    steps: Vec<ToolStep>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    activity: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStep {
    name: String,
    input: Value,
    output: String,
    ok: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    id: String,
    title: String,
    messages: Vec<Turn>,
    updated_at: u128,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    id: String,
    title: String,
    description: String,
    category: Option<String>,
    url: Option<String>,
    recommended: Option<String>,
    links: Vec<Value>,
    video: Option<Value>,
    artifacts: Vec<Value>,
    version: Option<String>,
    #[serde(default)]
    released: Option<String>,
    installed: bool,
    installed_version: Option<String>,
    update: Option<String>,
    release_notes: Option<String>,
    installable: bool,
    removable: bool,
    #[serde(default)]
    buildable: bool,
    #[serde(skip)]
    repo: Option<(String, String)>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reply {
    text: String,
    project_ids: Vec<String>,
    action: String,
    artifact_urls: Vec<Option<String>>,
    parts: Vec<Part>,
    model: String,
    elapsed_ms: u64,
    stopped: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent<'a> {
    chat_id: &'a str,
    kind: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    part: Option<&'a Part>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ms: Option<u64>,
}

const CONVERSATIONS_KEY: &str = "assistant_conversations_v1";

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn conversations(state: &Shared) -> Result<Vec<Conversation>, String> {
    state
        .store
        .setting(CONVERSATIONS_KEY)
        .map(|items| items.unwrap_or_default())
        .map_err(|e| e.to_string())
}

fn save_conversations(state: &Shared, items: &[Conversation]) -> Result<(), String> {
    let items = items.to_vec();
    state
        .store
        .set_setting(CONVERSATIONS_KEY, &items)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn assistant_conversations(
    state: State<'_, Shared>,
) -> Result<Vec<Conversation>, String> {
    let state = state.inner();
    let _guard = state
        .assistant_chats_lock
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut items = conversations(state)?;
    items.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
    Ok(items)
}

#[tauri::command]
pub async fn assistant_get_conversation(
    state: State<'_, Shared>,
    chat_id: String,
) -> Result<Conversation, String> {
    let state = state.inner();
    let _guard = state
        .assistant_chats_lock
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    conversations(state)?
        .into_iter()
        .find(|thread| thread.id == chat_id)
        .ok_or_else(|| "This chat is no longer available.".into())
}

#[tauri::command]
pub async fn assistant_new_conversation(state: State<'_, Shared>) -> Result<Conversation, String> {
    let state = state.inner();
    let _guard = state
        .assistant_chats_lock
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut items = conversations(state)?;
    if let Some(empty) = items.iter_mut().find(|c| c.messages.is_empty()) {
        empty.updated_at = now_nanos();
        let found = empty.clone();
        save_conversations(state, &items)?;
        return Ok(found);
    }
    let timestamp = now_nanos();
    let conversation = Conversation {
        id: format!("assistant-{timestamp}"),
        title: "New chat".into(),
        messages: Vec::new(),
        updated_at: timestamp,
    };
    items.push(conversation.clone());
    save_conversations(state, &items)?;
    Ok(conversation)
}

#[tauri::command]
pub async fn assistant_delete_conversation(
    state: State<'_, Shared>,
    chat_id: String,
) -> Result<(), String> {
    let state = state.inner();
    let _guard = state
        .assistant_chats_lock
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut items = conversations(state)?;
    items.retain(|conversation| conversation.id != chat_id);
    save_conversations(state, &items)
}

fn stop_requests() -> &'static Mutex<HashSet<String>> {
    static STOPS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    STOPS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn stop_requested(chat_id: &str) -> bool {
    stop_requests()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(chat_id)
}

#[tauri::command]
pub fn assistant_stop(chat_id: String) {
    stop_requests()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(chat_id);
}

fn key(state: &Shared, provider: &str) -> Result<Option<String>, String> {
    state
        .store
        .setting::<String>(&key_name(provider)?)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn assistant_key_status(
    state: State<'_, Shared>,
    provider: String,
) -> Result<bool, String> {
    Ok(key(state.inner(), &provider)?.is_some_and(|k| !k.is_empty()))
}

#[tauri::command]
pub async fn assistant_set_key(
    state: State<'_, Shared>,
    provider: String,
    key: String,
) -> Result<(), String> {
    let key = key.trim().to_string();
    let setting = key_name(&provider)?;
    if key.is_empty() {
        state
            .store
            .set_setting(&setting, &Option::<String>::None)
            .map_err(|e| e.to_string())
    } else {
        state
            .store
            .set_setting(&setting, &key)
            .map_err(|e| e.to_string())
    }
}

fn connection(state: &Shared) -> Result<(String, String, Option<String>), String> {
    let s = state.settings();
    let p = s.assistant_provider;
    if ![
        "ollama",
        "openai",
        "anthropic",
        "gemini",
        "openai-compatible",
    ]
    .contains(&p.as_str())
    {
        return Err("Choose a supported assistant provider in Settings.".into());
    }
    let base = if s.assistant_base_url.trim().is_empty() {
        match p.as_str() {
            "openai" => "https://api.openai.com/v1".to_string(),
            "anthropic" => "https://api.anthropic.com".to_string(),
            "gemini" => "https://generativelanguage.googleapis.com/v1beta".to_string(),
            "ollama" => "http://localhost:11434".to_string(),
            _ => return Err("Enter a base URL for the OpenAI-compatible provider.".into()),
        }
    } else {
        s.assistant_base_url
            .trim()
            .trim_end_matches('/')
            .to_string()
    };
    if !(base.starts_with("https://")
        || (p == "ollama" || p == "openai-compatible") && base.starts_with("http://"))
    {
        return Err("Hosted provider URLs must use HTTPS. Plain HTTP is supported for configured local model endpoints.".into());
    }
    let secret = if p == "ollama" { None } else { key(state, &p)? };
    if ["openai", "anthropic", "gemini"].contains(&p.as_str())
        && secret.as_deref().unwrap_or("").is_empty()
    {
        return Err("Add your provider API key in Settings first.".into());
    }
    Ok((p, base, secret))
}

fn provider(state: &Shared) -> Result<(String, String, String, Option<String>), String> {
    let model = state.settings().assistant_model.trim().to_string();
    if model.is_empty() {
        return Err("Enter a model name in Settings.".into());
    }
    let (p, base, secret) = connection(state)?;
    Ok((p, model, base, secret))
}

fn provider_label(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "Anthropic",
        "gemini" => "Gemini",
        "ollama" => "Ollama",
        "openai" => "OpenAI",
        _ => "The model server",
    }
}

fn chat_url(provider: &str, base: &str) -> String {
    match provider {
        "anthropic" => {
            if base.ends_with("/v1") {
                format!("{base}/messages")
            } else {
                format!("{base}/v1/messages")
            }
        }
        "gemini" => base.to_string(),
        _ if base.ends_with("/chat/completions") => base.to_string(),
        _ if base.ends_with("/v1") => format!("{base}/chat/completions"),
        _ => format!("{}/v1/chat/completions", base.trim_end_matches('/')),
    }
}

fn ollama_chat_url(base: &str) -> String {
    let root = base
        .trim_end_matches('/')
        .trim_end_matches("/v1")
        .trim_end_matches("/api");
    format!("{root}/api/chat")
}

fn can_build(tool: &Tool, host_os: &Value) -> bool {
    tool.repo.is_some()
        && (tool.artifacts.is_empty() || tool.artifacts.iter().any(|a| a["os"] == *host_os))
        && host_os != "android"
        && host_os != "ios"
}

fn is_loopback(base: &str) -> bool {
    let rest = base.split_once("://").map_or(base, |(_, r)| r);
    let host = if let Some(v6) = rest.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        rest.split([':', '/']).next().unwrap_or("")
    };
    host.eq_ignore_ascii_case("localhost") || host == "::1" || host.starts_with("127.")
}

fn client_for<'a>(shared: &'a reqwest::Client, base: &str) -> &'a reqwest::Client {
    static LOCAL: OnceLock<reqwest::Client> = OnceLock::new();
    if !is_loopback(base) {
        return shared;
    }
    LOCAL.get_or_init(|| {
        reqwest::Client::builder()
            .no_proxy()
            .user_agent(zl_crawler::fetch::user_agent())
            .connect_timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

fn endpoint(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
) -> reqwest::RequestBuilder {
    client
        .request(method, url)
        .timeout(std::time::Duration::from_secs(90))
}

fn openai_request(
    client: &reqwest::Client,
    url: &str,
    model: &str,
    secret: Option<&str>,
    messages: &[Value],
) -> reqwest::RequestBuilder {
    let mut request = endpoint(client, reqwest::Method::POST, url)
        .json(&json!({"model": model, "messages": messages}));
    if let Some(k) = secret.filter(|k| !k.is_empty()) {
        request = request.bearer_auth(k);
    }
    request
}

async fn complete(state: &Shared, messages: &[Value]) -> Result<String, String> {
    let (p, model, base, secret) = provider(state)?;
    complete_with(
        client_for(&state.client, &base),
        &p,
        &model,
        &base,
        secret,
        messages,
    )
    .await
}

async fn complete_with(
    client: &reqwest::Client,
    p: &str,
    model: &str,
    base: &str,
    secret: Option<String>,
    messages: &[Value],
) -> Result<String, String> {
    let response = match p {
        "anthropic" => {
            let mut system = String::new();
            let mut list = Vec::new();
            for m in messages {
                let role = m["role"].as_str().unwrap_or("user");
                let content = m["content"].as_str().unwrap_or("");
                if role == "system" {
                    system = content.to_string();
                } else {
                    list.push(json!({"role": if role == "assistant" {"assistant"} else {"user"}, "content": content}));
                }
            }
            endpoint(client, reqwest::Method::POST, &chat_url(p, base))
                .header("x-api-key", secret.unwrap_or_default())
                .header("anthropic-version", "2023-06-01")
                .json(&json!({"model": model, "max_tokens": 1200, "system": system, "messages": list}))
                .send().await.map_err(|e| format!("Could not reach Anthropic: {e}"))?
        }
        "gemini" => {
            let system = messages
                .iter()
                .find(|m| m["role"] == "system")
                .and_then(|m| m["content"].as_str())
                .unwrap_or("");
            let contents: Vec<Value> = messages
                .iter()
                .filter(|m| m["role"] != "system")
                .map(|m| {
                    json!({
                        "role": if m["role"] == "assistant" {"model"} else {"user"},
                        "parts": [{"text": m["content"].as_str().unwrap_or("")}]
                    })
                })
                .collect();
            let url = format!(
                "{}/models/{}:generateContent",
                base.trim_end_matches('/'),
                model
            );
            endpoint(client, reqwest::Method::POST, &url)
                .header("x-goog-api-key", secret.unwrap_or_default())
                .json(&json!({"systemInstruction": {"parts": [{"text": system}]}, "contents": contents, "generationConfig": {"maxOutputTokens": 1200}}))
                .send().await.map_err(|e| format!("Could not reach Gemini: {e}"))?
        }
        _ => openai_request(
            client,
            &chat_url(p, base),
            model,
            secret.as_deref(),
            messages,
        )
        .send()
        .await
        .map_err(|e| format!("Could not reach {p}: {e}"))?,
    };
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("Could not read provider response: {e}"))?;
    if !status.is_success() {
        let detail: String = body.chars().take(500).collect();
        return Err(format!(
            "Provider returned HTTP {}: {}",
            status.as_u16(),
            detail
        ));
    }
    let value: Value =
        serde_json::from_str(&body).map_err(|e| format!("Provider returned invalid JSON: {e}"))?;
    let text = match p {
        "anthropic" => value["content"]
            .as_array()
            .and_then(|a| a.iter().find_map(|v| v["text"].as_str()))
            .map(str::to_string),
        "gemini" => value["candidates"][0]["content"]["parts"]
            .as_array()
            .and_then(|a| a.iter().find_map(|v| v["text"].as_str()))
            .map(str::to_string),
        _ => value["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_string),
    };
    text.ok_or_else(|| "Provider response did not contain a text reply.".into())
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Protocol {
    Anthropic,
    Gemini,
    Ollama,
    OpenAi,
}

#[derive(Clone)]
struct Conn {
    provider: String,
    protocol: Protocol,
    model: String,
    base: String,
    secret: Option<String>,
    thinking: bool,
}

impl Conn {
    fn official_anthropic(&self) -> bool {
        self.base.starts_with("https://api.anthropic.com")
    }
}

fn conn(state: &Shared) -> Result<Conn, String> {
    let (provider, model, base, secret) = provider(state)?;
    let protocol = match provider.as_str() {
        "anthropic" => Protocol::Anthropic,
        "gemini" => Protocol::Gemini,
        "ollama" => Protocol::Ollama,
        _ => Protocol::OpenAi,
    };
    Ok(Conn {
        provider,
        protocol,
        model,
        base,
        secret,
        thinking: state.settings().assistant_thinking,
    })
}

struct Call {
    id: String,
    name: String,
    arguments: Value,
    invalid: Option<String>,
}

struct ToolResult {
    id: String,
    name: String,
    content: String,
    is_error: bool,
}

enum Msg {
    User(String),
    Assistant { text: String, raw: Option<Value> },
    Results(Vec<ToolResult>),
}

struct Round {
    text: String,
    calls: Vec<Call>,
    raw: Value,
    stop: Option<String>,
    stopped: bool,
}

struct Live {
    app: Option<tauri::AppHandle>,
    chat_id: String,
    parts: Vec<Part>,
    thinking_since: Option<Instant>,
}

impl Live {
    fn emit(&self, kind: &str, text: Option<&str>, part: Option<&Part>, ms: Option<u64>) {
        let Some(app) = &self.app else {
            return;
        };
        let _ = app.emit(
            "assistant-progress",
            ProgressEvent {
                chat_id: &self.chat_id,
                kind,
                text,
                part,
                ms,
            },
        );
    }

    fn thinking(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.thinking_since.is_none() {
            self.thinking_since = Some(Instant::now());
            self.parts.push(Part::new("thinking"));
        }
        if let Some(part) = self.parts.last_mut() {
            part.text.push_str(text);
        }
        self.emit("thinking", Some(text), None, None);
    }

    fn end_thinking(&mut self) {
        let Some(since) = self.thinking_since.take() else {
            return;
        };
        let ms = since.elapsed().as_millis() as u64;
        if let Some(part) = self.parts.last_mut().filter(|p| p.kind == "thinking") {
            part.ms = Some(ms);
            part.text = part.text.trim().to_string();
        }
        self.emit("thinking_done", None, None, Some(ms));
    }

    fn text(&mut self, text: &str) {
        self.end_thinking();
        let continuing = self.parts.last().is_some_and(|p| p.kind == "text");
        let text = if continuing { text } else { text.trim_start() };
        if text.is_empty() {
            return;
        }
        if !continuing {
            self.parts.push(Part::new("text"));
        }
        if let Some(part) = self.parts.last_mut() {
            part.text.push_str(text);
        }
        self.emit("text", Some(text), None, None);
    }

    fn tool_started(&mut self, id: &str, name: &str, input: &Value) {
        self.end_thinking();
        let part = Part {
            kind: "tool".into(),
            id: Some(id.into()),
            name: Some(name.into()),
            input: Some(input.clone()),
            ..Part::default()
        };
        self.emit("tool_started", None, Some(&part), None);
        self.parts.push(part);
    }

    fn tool_finished(&mut self, id: &str, output: String, summary: String, ok: bool) {
        let Some(at) = self
            .parts
            .iter()
            .rposition(|p| p.kind == "tool" && p.id.as_deref() == Some(id))
        else {
            return;
        };
        let part = &mut self.parts[at];
        part.output = Some(output);
        part.summary = Some(summary);
        part.ok = Some(ok);
        let part = part.clone();
        self.emit("tool_finished", None, Some(&part), None);
    }

    fn error(&mut self, message: &str) {
        self.end_thinking();
        let part = Part {
            kind: "error".into(),
            text: message.into(),
            ..Part::default()
        };
        self.emit("error", None, Some(&part), None);
        self.parts.push(part);
    }

    fn answer(&self) -> String {
        self.parts
            .iter()
            .filter(|p| p.kind == "text")
            .map(|p| p.text.trim())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

#[derive(Default)]
struct ThinkTags {
    inside: bool,
    pending: String,
}

impl ThinkTags {
    fn push(&mut self, chunk: &str, out: &mut Vec<(bool, String)>) {
        self.pending.push_str(chunk);
        loop {
            let tag = if self.inside { "</think>" } else { "<think>" };
            if let Some(at) = self.pending.find(tag) {
                if at > 0 {
                    out.push((self.inside, self.pending[..at].to_string()));
                }
                self.pending.drain(..at + tag.len());
                self.inside = !self.inside;
                continue;
            }
            let keep = (1..tag.len())
                .rev()
                .find(|&n| self.pending.ends_with(&tag[..n]))
                .unwrap_or(0);
            let cut = self.pending.len() - keep;
            if cut > 0 {
                out.push((self.inside, self.pending[..cut].to_string()));
                self.pending.drain(..cut);
            }
            break;
        }
    }

    fn finish(&mut self, out: &mut Vec<(bool, String)>) {
        if !self.pending.is_empty() {
            out.push((self.inside, std::mem::take(&mut self.pending)));
        }
    }
}

fn route(live: &mut Live, pieces: Vec<(bool, String)>) {
    for (thinking, text) in pieces {
        if thinking {
            live.thinking(&text);
        } else {
            live.text(&text);
        }
    }
}

struct Lines {
    response: reqwest::Response,
    buf: Vec<u8>,
    done: bool,
}

impl Lines {
    fn new(response: reqwest::Response) -> Self {
        Lines {
            response,
            buf: Vec::new(),
            done: false,
        }
    }

    async fn next(&mut self) -> Result<Option<String>, String> {
        loop {
            if let Some(at) = self.buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.buf.drain(..=at).collect();
                return Ok(Some(String::from_utf8_lossy(&line).trim_end().to_string()));
            }
            if self.done {
                if self.buf.is_empty() {
                    return Ok(None);
                }
                let line = String::from_utf8_lossy(&self.buf).trim_end().to_string();
                self.buf.clear();
                return Ok(Some(line));
            }
            match tokio::time::timeout(STREAM_IDLE, self.response.chunk()).await {
                Err(_) => return Err("The model stopped responding.".into()),
                Ok(Err(e)) => return Err(format!("The connection to the model broke: {e}")),
                Ok(Ok(Some(bytes))) => self.buf.extend_from_slice(&bytes),
                Ok(Ok(None)) => self.done = true,
            }
        }
    }
}

fn sse_data(line: &str) -> Option<&str> {
    line.strip_prefix("data:").map(str::trim)
}

fn function_definitions() -> Vec<Value> {
    let object = |properties: Value, required: &[&str]| {
        json!({
            "type":"object", "properties":properties, "required":required, "additionalProperties":false
        })
    };
    let id_props = || json!({"project_id":{"type":"string","description":"The tool's id from the catalogue, e.g. \"zpad\"."}});
    vec![
        json!({"name":"search_projects","description":"Search the ZSync catalogue by name or by what the user wants to do (\"typing speed test\", \"encrypt files\"). Use short, corrected keywords. Returns the best matches with a summary and install state.","parameters":object(json!({"query":{"type":"string","description":"A few keywords: a tool name or the capability the user needs."}}), &["query"])}),
        json!({"name":"get_project_details","description":"Everything about one tool: full description, current version and release date, release notes, supported platforms and downloads, website and source links, promo video, and whether it is installed or can be installed, built or removed here.","parameters":object(id_props(), &["project_id"])}),
        json!({"name":"list_installed","description":"Tools installed or found on this computer, with their versions and pending updates.","parameters":object(json!({}), &[])}),
        json!({"name":"check_updates","description":"Installed tools that have a newer version available.","parameters":object(json!({}), &[])}),
        json!({"name":"about_zsync","description":"Who builds ZSync and these tools (one person: TheHolyOneZ, Etienne Busch), the story, the zsync.eu and zlogic.eu brands, ZLauncher, the principles every tool follows, the tech stack, and how to contact or support the developer. Also live numbers: how many tools exist, the newest releases.","parameters":{"type":"object","properties":{"section":{"type":"string","description":"Optional: one part, e.g. \"The developer\", \"Story\", \"Contact and support\", \"How the tools are built\"."}},"required":[],"additionalProperties":false}}),
        json!({"name":"get_system_info","description":"This computer's operating system, distribution and processor architecture.","parameters":object(json!({}), &[])}),
        json!({"name":"get_project_readme","description":"Read a tool's README from its GitHub repository: how to use it, features, setup, requirements, shortcuts, configuration, FAQ. Without a section you get the outline and the start; then ask for the section you need.","parameters":{"type":"object","properties":{"project_id":{"type":"string","description":"The tool's id from the catalogue."},"section":{"type":"string","description":"Optional: a heading (or part of one) from the outline, e.g. \"Installation\" or \"Keyboard shortcuts\"."}},"required":["project_id"],"additionalProperties":false}}),
        json!({"name":"search_project_text","description":"Find specific facts in a tool's README, description and release notes with a regular expression (Rust regex syntax, case-insensitive, matched line by line). Capture groups come back separately. Examples: ctrl\\+\\w finds shortcuts like Ctrl+R; requires? (.+) captures requirements; v(\\d+\\.\\d+) captures version numbers. Write the pattern as plain regex text, without extra escaping.","parameters":{"type":"object","properties":{"project_id":{"type":"string","description":"The tool's id from the catalogue."},"pattern":{"type":"string","description":"A regular expression."},"source":{"type":"string","enum":["all","readme","description","release_notes"],"description":"Where to search. Default: all."}},"required":["project_id","pattern"],"additionalProperties":false}}),
        json!({"name":"get_github_info","description":"A tool's GitHub repository: stars, forks, open issues, license, main language, topics, when it was last worked on, and its latest releases with their notes. Use it to judge whether a tool is maintained or to see its history.","parameters":object(id_props(), &["project_id"])}),
        json!({"name":"install_project","description":"Install a tool on this computer. Only when the user asks to install or set it up.","parameters":object(id_props(), &["project_id"])}),
        json!({"name":"download_project","description":"Download a tool's file without installing it. Only when the user asks for a download.","parameters":object(json!({"project_id":{"type":"string","description":"The tool's id from the catalogue."},"artifact_url":{"type":"string","description":"Optional: a download URL from get_project_details, when the user wants a specific file."}}), &["project_id"])}),
        json!({"name":"update_project","description":"Update one installed tool that has a newer version. Only when the user asks.","parameters":object(id_props(), &["project_id"])}),
        json!({"name":"update_all","description":"Update every installed tool that has a newer version. Only when the user asks to update everything.","parameters":object(json!({}), &[])}),
        json!({"name":"uninstall_project","description":"Remove a tool installed through the launcher. The user always confirms in the app first.","parameters":object(id_props(), &["project_id"])}),
        json!({"name":"launch_project","description":"Open an installed tool. Only when the user asks to open or start it.","parameters":object(id_props(), &["project_id"])}),
        json!({"name":"build_project","description":"Build a tool from its source code on this computer and install the result. Takes a few minutes. Only when the user asks for a build.","parameters":object(id_props(), &["project_id"])}),
    ]
}

fn anthropic_tools(definitions: &[Value], eager: bool) -> Value {
    json!(definitions
        .iter()
        .map(|tool| {
            let mut t = json!({"name":tool["name"], "description":tool["description"], "input_schema":tool["parameters"]});
            if eager {
                t["eager_input_streaming"] = json!(true);
            }
            t
        })
        .collect::<Vec<_>>())
}

fn gemini_tools(definitions: &[Value]) -> Value {
    let declarations = definitions
        .iter()
        .map(|tool| {
            let params = &tool["parameters"];
            let mut d = json!({"name":tool["name"], "description":tool["description"]});
            if params["properties"]
                .as_object()
                .is_some_and(|p| !p.is_empty())
            {
                d["parameters"] = json!({"type":"object","properties":params["properties"],"required":params["required"]});
            }
            d
        })
        .collect::<Vec<_>>();
    json!([{ "functionDeclarations": declarations }])
}

fn openai_tools(definitions: &[Value]) -> Value {
    json!(definitions
        .iter()
        .map(|tool| json!({"type":"function", "function":tool}))
        .collect::<Vec<_>>())
}

fn anthropic_budget_thinking(model: &str) -> bool {
    [
        "claude-3",
        "-4-5",
        "-4-1",
        "-4-0",
        "opus-4-2025",
        "sonnet-4-2025",
    ]
    .iter()
    .any(|marker| model.contains(marker))
}

fn anthropic_fallbacks(model: &str) -> bool {
    [
        "claude-opus-5-5",
        "claude-opus-5",
        "claude-sonnet-5-5",
        "claude-fable-5-1",
    ]
    .contains(&model)
}

fn anthropic_body(c: &Conn, system: &str, history: &[Msg], lite: bool) -> Value {
    let mut messages = Vec::new();
    for m in history {
        match m {
            Msg::User(text) => messages.push(json!({"role":"user","content":text})),
            Msg::Assistant { raw: Some(raw), .. } => {
                messages.push(json!({"role":"assistant","content":raw}))
            }
            Msg::Assistant { text, raw: None } => {
                messages.push(json!({"role":"assistant","content":text}))
            }
            Msg::Results(results) => messages.push(json!({"role":"user","content":results.iter().map(|r| json!({"type":"tool_result","tool_use_id":r.id,"content":r.content,"is_error":r.is_error})).collect::<Vec<_>>()})),
        }
    }
    let official = c.official_anthropic() && !lite;
    let mut body = json!({
        "model": c.model,
        "max_tokens": 16000,
        "stream": true,
        "system": system,
        "messages": messages,
        "tools": anthropic_tools(&function_definitions(), official),
    });
    if c.thinking && !lite {
        body["thinking"] = if anthropic_budget_thinking(&c.model) {
            json!({"type":"enabled","budget_tokens":4096})
        } else {
            json!({"type":"adaptive","display":"summarized"})
        };
    }
    if official && anthropic_fallbacks(&c.model) {
        body["fallbacks"] = json!("default");
    }
    body
}

fn gemini_body(c: &Conn, system: &str, history: &[Msg], lite: bool) -> Value {
    let mut contents = Vec::new();
    for m in history {
        match m {
            Msg::User(text) => contents.push(json!({"role":"user","parts":[{"text":text}]})),
            Msg::Assistant { raw: Some(raw), .. } => contents.push(raw.clone()),
            Msg::Assistant { text, raw: None } => {
                contents.push(json!({"role":"model","parts":[{"text":text}]}))
            }
            Msg::Results(results) => contents.push(json!({"role":"user","parts":results.iter().map(|r| {
                let parsed: Value = serde_json::from_str(&r.content).unwrap_or_else(|_| json!({"result": r.content}));
                let response = if parsed.is_object() { parsed } else { json!({"result": parsed}) };
                let mut call = json!({"name": r.name, "response": response});
                if !r.id.starts_with("gemini-") {
                    call["id"] = json!(r.id);
                }
                json!({"functionResponse": call})
            }).collect::<Vec<_>>()})),
        }
    }
    let mut config = json!({"maxOutputTokens": 8192});
    if c.thinking && !lite {
        config["thinkingConfig"] = json!({"includeThoughts": true});
    }
    json!({
        "systemInstruction": {"parts": [{"text": system}]},
        "contents": contents,
        "tools": gemini_tools(&function_definitions()),
        "generationConfig": config,
    })
}

fn chat_messages(system: &str, history: &[Msg], ollama: bool) -> Vec<Value> {
    let mut messages = vec![json!({"role":"system","content":system})];
    for m in history {
        match m {
            Msg::User(text) => messages.push(json!({"role":"user","content":text})),
            Msg::Assistant { raw: Some(raw), .. } => messages.push(raw.clone()),
            Msg::Assistant { text, raw: None } => {
                messages.push(json!({"role":"assistant","content":text}))
            }
            Msg::Results(results) => {
                for r in results {
                    messages.push(if ollama {
                        json!({"role":"tool","content":r.content,"tool_name":r.name})
                    } else {
                        json!({"role":"tool","tool_call_id":r.id,"content":r.content})
                    });
                }
            }
        }
    }
    messages
}

fn ollama_body(c: &Conn, system: &str, history: &[Msg], lite: bool) -> Value {
    let mut body = json!({
        "model": c.model,
        "stream": true,
        "messages": chat_messages(system, history, true),
        "tools": openai_tools(&function_definitions()),
        "options": {"num_ctx": OLLAMA_CONTEXT},
    });
    if !lite {
        body["think"] = json!(c.thinking);
    }
    body
}

fn openai_body(c: &Conn, system: &str, history: &[Msg]) -> Value {
    json!({
        "model": c.model,
        "stream": true,
        "messages": chat_messages(system, history, false),
        "tools": openai_tools(&function_definitions()),
    })
}

fn friendly_http_error(c: &Conn, status: u16, body: &str) -> String {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let detail = parsed
        .as_ref()
        .and_then(|v| {
            v["error"]["message"]
                .as_str()
                .or_else(|| v["error"].as_str())
                .or_else(|| v["message"].as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.chars().take(300).collect());
    let who = provider_label(&c.provider);
    match status {
        401 | 403 => format!("{who} rejected the API key (HTTP {status}). Check it in Settings → AI assistant. {detail}"),
        404 => format!("{who} couldn't find the model “{}” (HTTP 404). Pick another one in Settings → AI assistant. {detail}", c.model),
        429 => format!("{who} is limiting requests right now (HTTP 429). Wait a moment and try again. {detail}"),
        _ => format!("{who} returned HTTP {status}: {detail}"),
    }
}

async fn open_stream(
    client: &reqwest::Client,
    c: &Conn,
    system: &str,
    history: &[Msg],
) -> Result<reqwest::Response, String> {
    let mut lite = false;
    loop {
        let request = match c.protocol {
            Protocol::Anthropic => {
                let body = anthropic_body(c, system, history, lite);
                let mut r = client
                    .post(chat_url(&c.provider, &c.base))
                    .header("x-api-key", c.secret.clone().unwrap_or_default())
                    .header("anthropic-version", "2023-06-01");
                if body.get("fallbacks").is_some() {
                    r = r.header("anthropic-beta", "server-side-fallback-2026-07-01");
                }
                r.json(&body)
            }
            Protocol::Gemini => client
                .post(format!(
                    "{}/models/{}:streamGenerateContent?alt=sse",
                    c.base.trim_end_matches('/'),
                    c.model
                ))
                .header("x-goog-api-key", c.secret.clone().unwrap_or_default())
                .json(&gemini_body(c, system, history, lite)),
            Protocol::Ollama => client
                .post(ollama_chat_url(&c.base))
                .json(&ollama_body(c, system, history, lite)),
            Protocol::OpenAi => {
                let mut r = client
                    .post(chat_url(&c.provider, &c.base))
                    .json(&openai_body(c, system, history));
                if let Some(k) = c.secret.as_deref().filter(|k| !k.is_empty()) {
                    r = r.bearer_auth(k);
                }
                r
            }
        };
        let response = request
            .timeout(Duration::from_secs(15 * 60))
            .send()
            .await
            .map_err(|e| {
                if c.protocol == Protocol::Ollama && e.is_connect() {
                    format!(
                        "Ollama isn't running at {}. Start it (ollama serve) and try again.",
                        c.base
                    )
                } else {
                    format!("Could not reach {}: {e}", provider_label(&c.provider))
                }
            })?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = response.text().await.unwrap_or_default();
        if status.as_u16() == 400 && !lite && c.protocol != Protocol::OpenAi {
            tracing::debug!("assistant: retrying without extras after: {body}");
            lite = true;
            continue;
        }
        return Err(friendly_http_error(c, status.as_u16(), &body));
    }
}

async fn stream_round(
    client: &reqwest::Client,
    c: &Conn,
    system: &str,
    history: &[Msg],
    live: &mut Live,
) -> Result<Round, String> {
    let response = open_stream(client, c, system, history).await?;
    let mut lines = Lines::new(response);
    let mut stopped = false;
    let mut stop = None;
    let mut calls = Vec::new();
    let mut text = String::new();
    let raw = match c.protocol {
        Protocol::Anthropic => {
            let mut blocks: Vec<Value> = Vec::new();
            let mut partial: HashMap<usize, String> = HashMap::new();
            let mut invalid: HashMap<usize, String> = HashMap::new();
            while let Some(line) = lines.next().await? {
                if stop_requested(&live.chat_id) {
                    stopped = true;
                    break;
                }
                let Some(data) = sse_data(&line) else {
                    continue;
                };
                let Ok(event) = serde_json::from_str::<Value>(data) else {
                    continue;
                };
                let index = event["index"].as_u64().unwrap_or(0) as usize;
                match event["type"].as_str().unwrap_or("") {
                    "content_block_start" => {
                        if blocks.len() <= index {
                            blocks.resize(index + 1, Value::Null);
                        }
                        blocks[index] = event["content_block"].clone();
                        if let Some(t) = blocks[index]["text"].as_str().filter(|t| !t.is_empty()) {
                            let t = t.to_string();
                            emit_text(live, &t, &mut text);
                        }
                    }
                    "content_block_delta" => {
                        let delta = &event["delta"];
                        let Some(block) = blocks.get_mut(index) else {
                            continue;
                        };
                        match delta["type"].as_str().unwrap_or("") {
                            "text_delta" => {
                                let piece = delta["text"].as_str().unwrap_or("");
                                block["text"] = json!(format!(
                                    "{}{piece}",
                                    block["text"].as_str().unwrap_or("")
                                ));
                                emit_text(live, piece, &mut text);
                            }
                            "thinking_delta" => {
                                let piece = delta["thinking"].as_str().unwrap_or("");
                                block["thinking"] = json!(format!(
                                    "{}{piece}",
                                    block["thinking"].as_str().unwrap_or("")
                                ));
                                live.thinking(piece);
                            }
                            "signature_delta" => block["signature"] = delta["signature"].clone(),
                            "input_json_delta" => partial
                                .entry(index)
                                .or_default()
                                .push_str(delta["partial_json"].as_str().unwrap_or("")),
                            _ => {}
                        }
                    }
                    "content_block_stop" => {
                        if let Some(block) =
                            blocks.get_mut(index).filter(|b| b["type"] == "tool_use")
                        {
                            let json_text = partial.remove(&index).unwrap_or_default();
                            if json_text.trim().is_empty() {
                                block["input"] = json!({});
                            } else {
                                match serde_json::from_str::<Value>(&json_text) {
                                    Ok(v) if v.is_object() => block["input"] = v,
                                    _ => {
                                        block["input"] = json!({});
                                        invalid.insert(index, json_text);
                                    }
                                }
                            }
                        }
                    }
                    "message_delta" => {
                        if let Some(reason) = event["delta"]["stop_reason"].as_str() {
                            stop = Some(reason.to_string());
                        }
                    }
                    "error" => {
                        return Err(format!(
                            "Anthropic: {}",
                            event["error"]["message"].as_str().unwrap_or("stream error")
                        ))
                    }
                    _ => {}
                }
            }
            let boundary = blocks.iter().rposition(|b| b["type"] == "fallback");
            let keep: Vec<(usize, Value)> = blocks
                .into_iter()
                .enumerate()
                .filter(|(i, b)| {
                    !b.is_null()
                        && !(boundary.is_some_and(|f| *i < f)
                            && ["thinking", "redacted_thinking", "tool_use"]
                                .contains(&b["type"].as_str().unwrap_or("")))
                })
                .collect();
            for (i, b) in &keep {
                if b["type"] == "tool_use" {
                    calls.push(Call {
                        id: b["id"].as_str().unwrap_or("").to_string(),
                        name: b["name"].as_str().unwrap_or("").to_string(),
                        arguments: b["input"].clone(),
                        invalid: invalid.get(i).cloned(),
                    });
                }
            }
            json!(keep.into_iter().map(|(_, b)| b).collect::<Vec<_>>())
        }
        Protocol::Gemini => {
            let mut parts: Vec<Value> = Vec::new();
            while let Some(line) = lines.next().await? {
                if stop_requested(&live.chat_id) {
                    stopped = true;
                    break;
                }
                let Some(data) = sse_data(&line) else {
                    continue;
                };
                let Ok(event) = serde_json::from_str::<Value>(data) else {
                    continue;
                };
                if let Some(message) = event["error"]["message"].as_str() {
                    return Err(format!("Gemini: {message}"));
                }
                let candidate = &event["candidates"][0];
                for part in candidate["content"]["parts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    if let Some(t) = part["text"].as_str() {
                        if part["thought"] == true {
                            live.thinking(t);
                        } else {
                            emit_text(live, t, &mut text);
                        }
                    }
                    if part.get("functionCall").is_some() {
                        let call = &part["functionCall"];
                        let name = call["name"].as_str().unwrap_or("").to_string();
                        let n = calls.len();
                        calls.push(Call {
                            id: call["id"]
                                .as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| format!("gemini-{n}-{name}")),
                            name,
                            arguments: call["args"].clone(),
                            invalid: None,
                        });
                    }
                    merge_gemini_part(&mut parts, part.clone());
                }
                if let Some(reason) = candidate["finishReason"].as_str() {
                    stop = Some(reason.to_string());
                }
                if let Some(reason) = event["promptFeedback"]["blockReason"].as_str() {
                    stop = Some(format!("blocked: {reason}"));
                }
            }
            json!({"role":"model","parts":parts})
        }
        Protocol::Ollama => {
            let mut tags = ThinkTags::default();
            let mut tool_calls = Vec::new();
            while let Some(line) = lines.next().await? {
                if stop_requested(&live.chat_id) {
                    stopped = true;
                    break;
                }
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(event) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if let Some(message) = event["error"].as_str() {
                    return Err(format!("Ollama: {message}"));
                }
                let message = &event["message"];
                if let Some(t) = message["thinking"].as_str() {
                    live.thinking(t);
                }
                if let Some(t) = message["content"].as_str().filter(|t| !t.is_empty()) {
                    let mut out = Vec::new();
                    tags.push(t, &mut out);
                    for (_, piece) in out.iter().filter(|(thinking, _)| !thinking) {
                        text.push_str(piece);
                    }
                    route(live, out);
                }
                for call in message["tool_calls"].as_array().into_iter().flatten() {
                    let f = &call["function"];
                    let name = f["name"].as_str().unwrap_or("").to_string();
                    let arguments = match &f["arguments"] {
                        Value::String(s) => serde_json::from_str(s).unwrap_or(json!({})),
                        Value::Null => json!({}),
                        v => v.clone(),
                    };
                    tool_calls.push(json!({"function":{"name":name,"arguments":arguments}}));
                    let n = calls.len();
                    calls.push(Call {
                        id: call["id"]
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| format!("call_{n}")),
                        name,
                        arguments,
                        invalid: None,
                    });
                }
                if event["done"] == true {
                    stop = event["done_reason"].as_str().map(str::to_string);
                    break;
                }
            }
            let mut out = Vec::new();
            tags.finish(&mut out);
            for (_, piece) in out.iter().filter(|(thinking, _)| !thinking) {
                text.push_str(piece);
            }
            route(live, out);
            if tool_calls.is_empty() {
                json!({"role":"assistant","content":text})
            } else {
                json!({"role":"assistant","content":text,"tool_calls":tool_calls})
            }
        }
        Protocol::OpenAi => {
            let mut tags = ThinkTags::default();
            let mut pending: Vec<(String, String, String)> = Vec::new();
            while let Some(line) = lines.next().await? {
                if stop_requested(&live.chat_id) {
                    stopped = true;
                    break;
                }
                let Some(data) = sse_data(&line) else {
                    continue;
                };
                if data == "[DONE]" {
                    break;
                }
                let Ok(event) = serde_json::from_str::<Value>(data) else {
                    continue;
                };
                if let Some(message) = event["error"]["message"].as_str() {
                    return Err(format!("{}: {message}", provider_label(&c.provider)));
                }
                let choice = &event["choices"][0];
                let delta = &choice["delta"];
                for field in ["reasoning_content", "reasoning"] {
                    if let Some(t) = delta[field].as_str() {
                        live.thinking(t);
                    }
                }
                if let Some(t) = delta["content"].as_str().filter(|t| !t.is_empty()) {
                    let mut out = Vec::new();
                    tags.push(t, &mut out);
                    for (_, piece) in out.iter().filter(|(thinking, _)| !thinking) {
                        text.push_str(piece);
                    }
                    route(live, out);
                }
                for (n, call) in delta["tool_calls"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    let index = call["index"].as_u64().map(|i| i as usize).unwrap_or(n);
                    if pending.len() <= index {
                        pending.resize(index + 1, Default::default());
                    }
                    let slot = &mut pending[index];
                    if let Some(id) = call["id"].as_str() {
                        slot.0 = id.to_string();
                    }
                    if let Some(name) = call["function"]["name"].as_str() {
                        if !slot.1.ends_with(name) {
                            slot.1.push_str(name);
                        }
                    }
                    match &call["function"]["arguments"] {
                        Value::String(s) => slot.2.push_str(s),
                        Value::Object(_) => slot.2 = call["function"]["arguments"].to_string(),
                        _ => {}
                    }
                }
                if let Some(reason) = choice["finish_reason"].as_str() {
                    stop = Some(reason.to_string());
                }
            }
            let mut out = Vec::new();
            tags.finish(&mut out);
            for (_, piece) in out.iter().filter(|(thinking, _)| !thinking) {
                text.push_str(piece);
            }
            route(live, out);
            let mut tool_calls = Vec::new();
            for (n, (id, name, args)) in pending.into_iter().enumerate() {
                if name.is_empty() {
                    continue;
                }
                let id = if id.is_empty() {
                    format!("call_{n}")
                } else {
                    id
                };
                let args = if args.trim().is_empty() {
                    "{}".to_string()
                } else {
                    args
                };
                let parsed = serde_json::from_str::<Value>(&args)
                    .ok()
                    .filter(Value::is_object);
                tool_calls.push(
                    json!({"id":id,"type":"function","function":{"name":name,"arguments":args}}),
                );
                calls.push(Call {
                    id,
                    name,
                    invalid: parsed.is_none().then(|| args.clone()),
                    arguments: parsed.unwrap_or_else(|| json!({})),
                });
            }
            if tool_calls.is_empty() {
                json!({"role":"assistant","content":text})
            } else {
                json!({"role":"assistant","content":if text.is_empty() {Value::Null} else {json!(text)},"tool_calls":tool_calls})
            }
        }
    };
    Ok(Round {
        text,
        calls,
        raw,
        stop,
        stopped,
    })
}

fn emit_text(live: &mut Live, piece: &str, text: &mut String) {
    text.push_str(piece);
    live.text(piece);
}

fn merge_gemini_part(parts: &mut Vec<Value>, part: Value) {
    let plain_text = |p: &Value| p.get("text").is_some() && p.get("functionCall").is_none();
    if plain_text(&part) {
        if let Some(last) = parts.last_mut() {
            if plain_text(last)
                && last.get("thoughtSignature").is_none()
                && last["thought"] == part["thought"]
            {
                let joined = format!(
                    "{}{}",
                    last["text"].as_str().unwrap_or(""),
                    part["text"].as_str().unwrap_or("")
                );
                last["text"] = json!(joined);
                if let Some(signature) = part.get("thoughtSignature") {
                    last["thoughtSignature"] = signature.clone();
                }
                return;
            }
        }
    }
    parts.push(part);
}

const STOP_WORDS: &[&str] = &[
    "a",
    "an",
    "the",
    "for",
    "to",
    "me",
    "my",
    "i",
    "im",
    "please",
    "need",
    "want",
    "looking",
    "searching",
    "search",
    "find",
    "app",
    "apps",
    "application",
    "applications",
    "program",
    "programs",
    "programm",
    "software",
    "tool",
    "tools",
    "something",
    "some",
    "good",
    "best",
    "is",
    "are",
    "like",
    "with",
    "where",
    "which",
    "that",
    "can",
    "how",
    "what",
    "there",
    "any",
    "of",
    "and",
    "or",
    "in",
    "on",
    "it",
    "do",
    "does",
    "you",
    "your",
];

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

fn stem(word: &str) -> String {
    for suffix in ["ings", "ing", "ers", "er", "ed", "es", "s", "e"] {
        if let Some(rest) = word.strip_suffix(suffix) {
            if rest.chars().count() >= 3 {
                return rest.to_string();
            }
        }
    }
    word.to_string()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let here = row[j + 1];
            row[j + 1] = if ca == cb {
                prev
            } else {
                1 + prev.min(row[j]).min(row[j + 1])
            };
            prev = here;
        }
    }
    row[b.len()]
}

fn word_match(query: &str, query_stem: &str, word: &str) -> u8 {
    if query == word || query_stem == stem(word) {
        return 2;
    }
    let n = query.chars().count();
    let allowed = if n >= 8 {
        2
    } else if n >= 5 {
        1
    } else {
        0
    };
    if allowed > 0
        && n.abs_diff(word.chars().count()) <= allowed
        && edit_distance(query, word) <= allowed
    {
        return 1;
    }
    let (short, long) = if n <= word.chars().count() {
        (query, word)
    } else {
        (word, query)
    };
    if short.chars().count() >= 5 && long.starts_with(short) {
        return 1;
    }
    0
}

#[cfg(test)]
fn search_tools<'a>(query: &str, tools: &'a [Tool]) -> Vec<&'a Tool> {
    search_hits(query, tools)
        .into_iter()
        .map(|(tool, _)| tool)
        .collect()
}

fn search_hits<'a>(query: &str, tools: &'a [Tool]) -> Vec<(&'a Tool, Vec<String>)> {
    let ranked = search_scored(query, tools);
    let terms = words(query)
        .into_iter()
        .filter(|w| !STOP_WORDS.contains(&w.as_str()))
        .count();
    let cutoff = if terms >= 3 { 0.3 } else { 0.45 };
    let top = ranked.first().map_or(0.0, |(score, _, _)| *score);
    ranked
        .into_iter()
        .take_while(|(score, _, _)| *score >= top * cutoff)
        .take(6)
        .map(|(_, tool, matched)| (tool, matched))
        .collect()
}

fn search_scored<'a>(query: &str, tools: &'a [Tool]) -> Vec<(f64, &'a Tool, Vec<String>)> {
    let terms: Vec<(String, String)> = words(query)
        .into_iter()
        .filter(|w| !STOP_WORDS.contains(&w.as_str()))
        .map(|w| {
            let s = stem(&w);
            (w, s)
        })
        .collect();
    let whole = query.trim().to_lowercase();
    let phrase = terms
        .iter()
        .map(|(w, _)| w.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let matches: Vec<Vec<u32>> = tools
        .iter()
        .map(|tool| {
            let title: Vec<String> = words(&tool.title)
                .into_iter()
                .flat_map(|w| {
                    let bare = w
                        .strip_prefix('z')
                        .filter(|rest| rest.chars().count() >= 3)
                        .map(str::to_string);
                    std::iter::once(w).chain(bare)
                })
                .collect();
            let category = words(tool.category.as_deref().unwrap_or(""));
            let description: HashSet<String> = words(&tool.description).into_iter().collect();
            let best = |q: &str, s: &str, list: &mut dyn Iterator<Item = &String>| {
                list.map(|w| word_match(q, s, w)).max().unwrap_or(0) as u32
            };
            terms
                .iter()
                .map(|(q, s)| {
                    [
                        title
                            .iter()
                            .map(|w| {
                                let inside = q.chars().count() >= 4 && w.contains(q.as_str());
                                word_match(q, s, w).max(if inside { 2 } else { 0 }) as u32
                            })
                            .max()
                            .unwrap_or(0)
                            * 6,
                        best(q, s, &mut category.iter()) * 2,
                        best(q, s, &mut description.iter()) * 2,
                    ]
                    .into_iter()
                    .max()
                    .unwrap_or(0)
                })
                .collect()
        })
        .collect();
    let n = tools.len() as f64;
    let weight: Vec<f64> = (0..terms.len())
        .map(|t| {
            let df = matches.iter().filter(|m| m[t] > 0).count() as f64;
            ((n + 1.0) / (df + 0.5)).ln().max(0.1)
        })
        .collect();
    let mut ranked: Vec<(f64, &Tool, Vec<String>)> = tools
        .iter()
        .zip(&matches)
        .map(|(tool, m)| {
            let matched: Vec<String> = m
                .iter()
                .zip(&terms)
                .filter(|(s, _)| **s > 0)
                .map(|(_, (w, _))| w.clone())
                .collect();
            let mut score: f64 = m.iter().zip(&weight).map(|(s, w)| *s as f64 * w).sum();
            if score > 0.0 && terms.len() > 1 && tool.description.to_lowercase().contains(&phrase) {
                score *= 1.5;
            }
            if whole.len() >= 3 && tool.title.to_lowercase().contains(&whole) {
                score += 40.0;
            }
            (score, tool, matched)
        })
        .filter(|(score, _, _)| *score > 0.0)
        .collect();
    ranked.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then_with(|| b.2.len().cmp(&a.2.len()))
            .then_with(|| a.1.title.cmp(&b.1.title))
    });
    ranked
}

fn title_from(message: &str) -> String {
    let flat = message.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut title = String::new();
    for word in flat.split(' ') {
        if !title.is_empty() && title.chars().count() + 1 + word.chars().count() > 40 {
            title.push('…');
            break;
        }
        if !title.is_empty() {
            title.push(' ');
        }
        title.push_str(word);
    }
    if title.chars().count() > 44 {
        title = clip(&title, 40);
    }
    let mut chars = title.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "New chat".into(),
    }
}

fn first_sentence(text: &str, max: usize) -> String {
    let text = text.trim();
    let end = text
        .char_indices()
        .find(|(i, c)| matches!(c, '.' | '!' | '?') && text[i + 1..].starts_with(' '))
        .map(|(i, _)| i + 1)
        .unwrap_or(text.len());
    clip(&text[..end], max)
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(max).collect::<String>().trim_end())
    }
}

fn status(tool: &Tool) -> Value {
    json!({
        "installed": tool.installed,
        "installedVersion": tool.installed_version,
        "updateAvailable": tool.update,
        "canInstall": !tool.installed && tool.installable,
        "canBuild": tool.buildable,
    })
}

fn platforms(tool: &Tool) -> Vec<String> {
    let mut list: Vec<String> = tool
        .artifacts
        .iter()
        .filter_map(|a| a["os"].as_str().map(str::to_string))
        .collect();
    list.sort();
    list.dedup();
    list
}

fn run_tool(name: &str, args: &Value, tools: &[Tool], system: &Value) -> Value {
    let requested_id = args["project_id"]
        .as_str()
        .or_else(|| args["id"].as_str())
        .unwrap_or("")
        .trim();
    let requested_tool = || {
        tools
            .iter()
            .find(|tool| tool.id == requested_id)
            .or_else(|| {
                tools
                    .iter()
                    .find(|tool| tool.title.eq_ignore_ascii_case(requested_id))
            })
    };
    let unknown = || json!({"error": format!("No tool in the catalogue has the id \"{requested_id}\". Use an id from the catalogue list or search_projects.")});
    match name {
        "search_projects" => {
            let query = args["query"].as_str().unwrap_or("");
            let matches = search_hits(query, tools)
                .into_iter()
                .map(|(tool, matched)| {
                    json!({
                        "id":tool.id,
                        "matchedWords":matched,
                        "title":tool.title,
                        "category":tool.category,
                        "summary":clip(&tool.description, 220),
                        "version":tool.version,
                        "installed":tool.installed,
                        "updateAvailable":tool.update,
                    })
                })
                .collect::<Vec<_>>();
            json!({"query": query, "count": matches.len(), "matches": matches})
        }
        "get_project_details" | "get_project_links" | "get_project_video" => match requested_tool()
        {
            Some(tool) => json!({
                "id": tool.id,
                "title": tool.title,
                "category": tool.category,
                "description": clip(&tool.description, 1500),
                "version": tool.version,
                "released": tool.released,
                "releaseNotes": tool.release_notes.as_deref().map(|n| clip(n, 900)),
                "platforms": platforms(tool),
                "downloads": tool.artifacts.iter().take(12).map(|a| json!({
                    "file": a["filename"], "os": a["os"], "arch": a["arch"], "kind": a["kind"], "sizeBytes": a["size"], "url": a["url"],
                })).collect::<Vec<_>>(),
                "website": tool.url,
                "links": tool.links.iter().map(|l| json!({"label": l["label"], "url": l["url"]})).collect::<Vec<_>>(),
                "video": tool.video.as_ref().and_then(|v| v["mp4"].as_str()),
                "status": status(tool),
                "canUninstall": tool.removable,
            }),
            None => unknown(),
        },
        "list_installed" => {
            let items: Vec<Value> = tools
                .iter()
                .filter(|tool| tool.installed)
                .map(|tool| json!({"id": tool.id, "title": tool.title, "version": tool.installed_version, "updateAvailable": tool.update}))
                .collect();
            json!({"count": items.len(), "installed": items})
        }
        "check_updates" => {
            let items: Vec<Value> = tools
                .iter()
                .filter(|tool| tool.installed && tool.update.is_some())
                .map(|tool| json!({"id": tool.id, "title": tool.title, "installedVersion": tool.installed_version, "newVersion": tool.update}))
                .collect();
            json!({"count": items.len(), "updates": items})
        }
        "get_system_info" => system.clone(),
        "about_zsync" => {
            let mut newest: Vec<&Tool> = tools.iter().filter(|t| t.released.is_some()).collect();
            newest.sort_by(|a, b| b.released.cmp(&a.released));
            let categories: HashSet<&str> =
                tools.iter().filter_map(|t| t.category.as_deref()).collect();
            let live = json!({
                "toolsInCatalogue": tools.len(),
                "withSourceOnGitHub": tools.iter().filter(|t| t.repo.is_some()).count(),
                "categories": categories.len(),
                "newestReleases": newest.iter().take(3).map(|t| json!({"title": t.title, "version": t.version, "released": t.released})).collect::<Vec<_>>(),
                "installedOnThisComputer": tools.iter().filter(|t| t.installed).count(),
            });
            let requested = args["section"]
                .as_str()
                .map(str::trim)
                .filter(|q| !q.is_empty());
            match requested.and_then(|q| readme_section(ABOUT, q)) {
                Some((title, body)) => {
                    json!({"section": title, "content": body, "live": live, "source": "https://zlogic.eu/about/"})
                }
                None => {
                    json!({"content": ABOUT, "live": live, "source": "https://zlogic.eu/about/"})
                }
            }
        }
        "install_project" => match requested_tool() {
            Some(tool) if tool.installed => {
                json!({"error":format!("{} is already installed (version {}).", tool.title, tool.installed_version.as_deref().unwrap_or("unknown"))})
            }
            Some(tool) if tool.installable => {
                json!({"requestedAction":"install","projectIds":[tool.id],"status":"The app starts the install after your reply and shows its progress."})
            }
            Some(tool) if tool.buildable => {
                json!({"error":format!("{} has no ready-made download for this computer, but it can be built from source with build_project.", tool.title)})
            }
            Some(tool) => {
                json!({"error":format!("{} has no version for this computer.", tool.title)})
            }
            None => unknown(),
        },
        "download_project" => match requested_tool() {
            Some(tool) => {
                let requested_url = args["artifact_url"].as_str();
                let selected = requested_url
                    .filter(|url| {
                        tool.artifacts
                            .iter()
                            .any(|artifact| artifact["url"] == *url)
                    })
                    .or(tool.recommended.as_deref())
                    .or_else(|| {
                        tool.artifacts
                            .first()
                            .and_then(|artifact| artifact["url"].as_str())
                    });
                selected.map(|url| json!({"requestedAction":"download","projectIds":[tool.id],"artifactUrl":url,"status":"The app starts the download after your reply."})).unwrap_or_else(|| json!({"error":format!("{} has no downloadable file.", tool.title)}))
            }
            None => unknown(),
        },
        "update_project" => match requested_tool() {
            Some(tool) if tool.installed && tool.update.is_some() => {
                json!({"requestedAction":"update_now","projectIds":[tool.id],"status":"The app starts the update after your reply."})
            }
            Some(tool) if !tool.installed => {
                json!({"error":format!("{} is not installed on this computer.", tool.title)})
            }
            Some(tool) => json!({"error":format!("{} is already up to date.", tool.title)}),
            None => unknown(),
        },
        "update_all" => {
            let ids: Vec<&str> = tools
                .iter()
                .filter(|tool| tool.installed && tool.update.is_some())
                .map(|tool| tool.id.as_str())
                .collect();
            if ids.is_empty() {
                json!({"error":"Every installed tool is already up to date."})
            } else {
                json!({"requestedAction":"update_now","projectIds":ids,"status":"The app starts all updates after your reply."})
            }
        }
        "uninstall_project" => match requested_tool() {
            Some(tool) if tool.removable => {
                json!({"requestedAction":"uninstall","projectIds":[tool.id],"status":"The app asks the user to confirm before removing it."})
            }
            Some(tool) if tool.installed => {
                json!({"error":format!("{} wasn't installed through the launcher, so remove it with your system's tools.", tool.title)})
            }
            Some(tool) => json!({"error":format!("{} is not installed.", tool.title)}),
            None => unknown(),
        },
        "launch_project" => match requested_tool() {
            Some(tool) if tool.installed => {
                json!({"requestedAction":"launch","projectIds":[tool.id],"status":"The app opens it after your reply."})
            }
            Some(tool) => json!({"error":format!("{} is not installed.", tool.title)}),
            None => unknown(),
        },
        "build_project" => match requested_tool() {
            Some(tool) if tool.buildable => {
                json!({"requestedAction":"build","projectIds":[tool.id],"status":"The app starts the build after your reply and shows its progress."})
            }
            Some(tool) => {
                json!({"error":format!("{} has no source repository the launcher can build.", tool.title)})
            }
            None => unknown(),
        },
        _ => json!({"error": format!("There is no tool called {name}.")}),
    }
}

fn summarize(name: &str, result: &Value) -> String {
    if let Some(error) = result["error"].as_str() {
        return error.to_string();
    }
    let count = result["count"].as_u64().unwrap_or(0);
    let plural = |n: u64, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    match name {
        "search_projects" => {
            if count == 0 {
                "No matches".into()
            } else {
                let titles: Vec<&str> = result["matches"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(3)
                    .filter_map(|m| m["title"].as_str())
                    .collect();
                format!(
                    "{} · {}",
                    plural(count, "match", "matches"),
                    titles.join(", ")
                )
            }
        }
        "get_project_details" | "get_project_links" | "get_project_video" => format!(
            "{} {}",
            result["title"].as_str().unwrap_or(""),
            result["version"].as_str().unwrap_or("")
        )
        .trim()
        .to_string(),
        "list_installed" => plural(count, "tool installed", "tools installed"),
        "check_updates" => {
            if count == 0 {
                "Everything is up to date".into()
            } else {
                plural(count, "update available", "updates available")
            }
        }
        "get_system_info" => result["name"].as_str().unwrap_or("").to_string(),
        "about_zsync" => match result["section"].as_str() {
            Some(section) => format!("Section “{section}”"),
            None => format!(
                "TheHolyOneZ · {} tools",
                result["live"]["toolsInCatalogue"].as_u64().unwrap_or(0)
            ),
        },
        "get_project_readme" => match result["section"].as_str() {
            Some(section) => format!("Section “{section}”"),
            None => format!(
                "README · {}",
                plural(
                    result["outline"].as_array().map_or(0, |o| o.len()) as u64,
                    "section",
                    "sections"
                )
            ),
        },
        "search_project_text" => {
            if count == 0 {
                "No matches".into()
            } else {
                plural(count, "match", "matches")
            }
        }
        "get_github_info" => {
            let pushed = result["lastPush"].as_str().unwrap_or("");
            format!(
                "{} stars · last change {}",
                result["stars"].as_u64().unwrap_or(0),
                pushed.get(..10).unwrap_or("unknown")
            )
        }
        _ => match result["requestedAction"].as_str() {
            Some("uninstall") => "Waiting for your confirmation".into(),
            Some(_) => "Ready to start".into(),
            None => "Done".into(),
        },
    }
}

fn catalogue_index(tools: &[Tool]) -> String {
    tools
        .iter()
        .map(|tool| {
            let mut flags = Vec::new();
            if tool.installed {
                flags.push(format!(
                    "installed {}",
                    tool.installed_version.as_deref().unwrap_or("")
                ));
            }
            if let Some(update) = &tool.update {
                flags.push(format!("update {update} available"));
            }
            if !tool.installed && !tool.installable && !tool.buildable {
                flags.push("not available for this computer".into());
            } else if !tool.installed && !tool.installable {
                flags.push("build from source only".into());
            }
            let flags = if flags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", flags.join(", ").replace(" ,", ","))
            };
            format!(
                "- {} · {} · {}{} — {}",
                tool.id,
                tool.title,
                tool.category.as_deref().unwrap_or("Other"),
                flags,
                first_sentence(&tool.description, 110)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn system_prompt(tools: &[Tool], system: &Value) -> String {
    format!(
        "You are the ZSync assistant inside ZLauncher, the desktop app that installs and updates the tools published on zsync.eu. You help the user find the right tool in the ZSync catalogue and manage tools on this computer.

ZSync is a one-person software workshop: every tool in the catalogue, and ZLauncher itself, is made by one developer, Etienne Busch, known as TheHolyOneZ (TheZ). It is not a team or a company, so never say \"the ZSync team\", \"the developers\" or \"we\". For anything about ZSync, zlogic.eu or the developer (who it is, the story, principles, stack, how to get in touch or support the work), call about_zsync and answer from it.

How to work:
- The catalogue below lists every tool. Only recommend tools from it. Never invent tools, features, versions or links, and don't suggest software from elsewhere. Use the exact titles.
- People misspell things or describe what they want to do instead of naming a tool. Work out what they mean and match it to the catalogue (\"test how fast i type\" → a typing-speed trainer).
- Before describing a tool in detail, quoting versions or release notes, or saying whether it runs here, call get_project_details. Use search_projects when the list alone isn't enough.
- For what's installed or has updates, call list_installed or check_updates. Never guess.
- For how a tool works (features, setup, requirements, shortcuts, configuration), read its README with get_project_readme: first the outline, then the section you need. To pull out specific facts fast, use search_project_text with a regular expression; capture groups come back separately.
- To judge whether a tool is maintained, or for its release history, call get_github_info.
- Only call an action tool (install, download, update, launch, build, uninstall) when the user clearly asks for that action. If it's unclear, ask first. The app runs the action after your reply, shows its progress and asks for confirmation where needed, so never say it already finished.
- If nothing fits, say so plainly.

Style: warm, direct and short. Lead with the answer. Use **bold** for tool names, short paragraphs and bullet lists where they help; no tables, and no headings for short answers.

This computer: {}. Today is {}.

Catalogue ({} tools; id · title · category [status] — summary):
{}",
        system["name"].as_str().unwrap_or("unknown"),
        today(),
        tools.len(),
        catalogue_index(tools)
    )
}

fn is_action_tool(name: &str) -> bool {
    matches!(
        name,
        "install_project"
            | "download_project"
            | "update_project"
            | "update_all"
            | "uninstall_project"
            | "launch_project"
            | "build_project"
    )
}

fn mentioned(text: &str, tools: &[Tool]) -> Vec<String> {
    let boundary = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric() && c != '-');
    let mut found: Vec<(usize, String)> = tools
        .iter()
        .filter(|tool| tool.title.chars().count() >= 3)
        .filter_map(|tool| {
            text.match_indices(tool.title.as_str())
                .find(|(at, m)| {
                    boundary(text[..*at].chars().next_back())
                        && boundary(text[at + m.len()..].chars().next())
                })
                .map(|(at, _)| (at, tool.id.clone()))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, id)| id).take(4).collect()
}

fn today() -> String {
    date_of(zl_store::now().div_euclid(86_400))
}

fn date_of(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

struct ToolCtx<'a> {
    client: &'a reqwest::Client,
    github_token: Option<&'a str>,
    tools: &'a [Tool],
    system: &'a Value,
}

impl ToolCtx<'_> {
    fn find(&self, args: &Value) -> Result<&Tool, Value> {
        let id = args["project_id"].as_str().unwrap_or("").trim();
        self.tools
            .iter()
            .find(|tool| tool.id == id)
            .or_else(|| {
                self.tools
                    .iter()
                    .find(|tool| tool.title.eq_ignore_ascii_case(id))
            })
            .ok_or_else(
                || json!({"error": format!("No tool in the catalogue has the id \"{id}\".")}),
            )
    }

    fn repo<'t>(&self, tool: &'t Tool) -> Result<&'t (String, String), Value> {
        tool.repo
            .as_ref()
            .ok_or_else(|| json!({"error": format!("{} has no GitHub repository linked in the catalogue.", tool.title)}))
    }

    async fn github(&self, url: &str, accept: &str) -> Result<reqwest::Response, String> {
        let mut request = self
            .client
            .get(url)
            .header("Accept", accept)
            .header("X-GitHub-Api-Version", "2022-11-28")
            .timeout(Duration::from_secs(20));
        if let Some(token) = self.github_token {
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .await
            .map_err(|e| format!("Couldn't reach GitHub: {e}"))?;
        match response.status().as_u16() {
            200..=299 => Ok(response),
            404 => Err(
                "GitHub has nothing public there (the repository may be private or renamed)."
                    .into(),
            ),
            403 | 429 => Err(
                "GitHub's rate limit was hit. Signing in to GitHub in Settings raises it.".into(),
            ),
            status => Err(format!("GitHub returned HTTP {status}.")),
        }
    }

    async fn readme(&self, owner: &str, repo: &str) -> Result<String, String> {
        let key = format!("{owner}/{repo}").to_lowercase();
        if let Some((at, text)) = readme_cache()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
        {
            if at.elapsed() < Duration::from_secs(30 * 60) {
                return Ok(text.clone());
            }
        }
        let api = format!("https://api.github.com/repos/{owner}/{repo}/readme");
        let raw = match self.github(&api, "application/vnd.github.raw+json").await {
            Ok(response) => response
                .text()
                .await
                .map_err(|e| format!("Couldn't read the README: {e}"))?,
            Err(api_error) => {
                let mut found = None;
                for name in ["README.md", "readme.md", "Readme.md", "README"] {
                    let url =
                        format!("https://raw.githubusercontent.com/{owner}/{repo}/HEAD/{name}");
                    if let Ok(response) = self
                        .client
                        .get(&url)
                        .timeout(Duration::from_secs(20))
                        .send()
                        .await
                    {
                        if response.status().is_success() {
                            found = response.text().await.ok();
                            break;
                        }
                    }
                }
                found.ok_or(api_error)?
            }
        };
        let text = clean_readme(&raw);
        readme_cache()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, (Instant::now(), text.clone()));
        Ok(text)
    }
}

fn readme_cache() -> &'static Mutex<HashMap<String, (Instant, String)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (Instant, String)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn fence(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
}

fn clean_readme(raw: &str) -> String {
    static RE: OnceLock<[Regex; 4]> = OnceLock::new();
    let [comments, badges, images, tags] = RE.get_or_init(|| {
        [
            Regex::new(r"(?s)<!--.*?-->").unwrap(),
            Regex::new(r"\[\s*!\[[^\]]*\]\([^)]*\)\s*\]\([^)]*\)").unwrap(),
            Regex::new(r"!\[[^\]]*\]\([^)]*\)").unwrap(),
            Regex::new(r"</?[A-Za-z][^<>]*>").unwrap(),
        ]
    });
    let text = comments
        .replace_all(&raw.replace('\r', ""), "")
        .into_owned();
    let mut out = Vec::new();
    let mut code = false;
    for line in text.lines() {
        if fence(line) {
            code = !code;
            out.push(line.trim_end().to_string());
            continue;
        }
        if code {
            out.push(line.trim_end().to_string());
            continue;
        }
        let line = badges.replace_all(line, "");
        let line = images.replace_all(&line, "");
        let line = tags.replace_all(&line, "");
        let line = line.replace("&nbsp;", " ").replace("&amp;", "&");
        out.push(line.trim_end().to_string());
    }
    let mut cleaned = String::new();
    let mut blank = 0;
    for line in out {
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        cleaned.push_str(&line);
        cleaned.push('\n');
    }
    cleaned.trim().to_string()
}

fn readme_headings(text: &str) -> Vec<(usize, usize, String)> {
    let mut code = false;
    let mut found = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if fence(line) {
            code = !code;
            continue;
        }
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if !code && (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
            let title = line[hashes..]
                .trim()
                .trim_end_matches('#')
                .trim()
                .to_string();
            if !title.is_empty() {
                found.push((i, hashes, title));
            }
        }
    }
    found
}

fn readme_section(text: &str, query: &str) -> Option<(String, String)> {
    let query = query.trim().trim_start_matches('#').trim().to_lowercase();
    let headings = readme_headings(text);
    let at = headings
        .iter()
        .position(|(_, _, title)| title.to_lowercase() == query)
        .or_else(|| {
            headings
                .iter()
                .position(|(_, _, title)| title.to_lowercase().contains(&query))
        })
        .or_else(|| {
            let wanted: Vec<(String, String)> = words(&query)
                .into_iter()
                .filter(|w| !STOP_WORDS.contains(&w.as_str()))
                .map(|w| {
                    let s = stem(&w);
                    (w, s)
                })
                .collect();
            headings
                .iter()
                .enumerate()
                .map(|(i, (_, _, title))| {
                    let have = words(title);
                    let shared = wanted
                        .iter()
                        .filter(|(q, s)| have.iter().any(|h| word_match(q, s, h) > 0))
                        .count();
                    (shared, std::cmp::Reverse(i))
                })
                .max()
                .filter(|(shared, _)| *shared > 0)
                .map(|(_, std::cmp::Reverse(i))| i)
        })?;
    let (start, level, title) = &headings[at];
    let end = headings[at + 1..]
        .iter()
        .find(|(_, l, _)| l <= level)
        .map_or(usize::MAX, |(i, _, _)| *i);
    let body = text
        .lines()
        .enumerate()
        .filter(|(i, _)| i >= start && *i < end)
        .map(|(_, l)| l)
        .collect::<Vec<_>>()
        .join("\n");
    Some((title.clone(), body.trim().to_string()))
}

fn snippet(line: &str, start: usize, end: usize) -> String {
    let line = line.trim();
    if line.chars().count() <= 300 {
        return line.to_string();
    }
    let mut from = start.saturating_sub(120);
    while !line.is_char_boundary(from) {
        from -= 1;
    }
    let mut to = (end + 180).min(line.len());
    while !line.is_char_boundary(to) {
        to += 1;
    }
    format!(
        "{}{}{}",
        if from > 0 { "…" } else { "" },
        line[from..to].trim(),
        if to < line.len() { "…" } else { "" }
    )
}

fn regex_search(re: &Regex, sources: &[(&str, &str)], limit: usize) -> Vec<Value> {
    let mut found = Vec::new();
    for (source, text) in sources {
        for (n, line) in text.lines().enumerate() {
            let Some(caps) = re.captures(line) else {
                continue;
            };
            let whole = caps.get(0).expect("group 0 always matches");
            let offset = line.len() - line.trim_start().len();
            let mut hit = json!({
                "source": source,
                "line": n + 1,
                "text": snippet(line, whole.start().saturating_sub(offset), whole.end().saturating_sub(offset)),
                "match": whole.as_str(),
            });
            if caps.len() > 1 {
                hit["groups"] = json!((1..caps.len())
                    .map(|g| caps.get(g).map(|m| m.as_str()))
                    .collect::<Vec<_>>());
            }
            found.push(hit);
            if found.len() >= limit {
                return found;
            }
        }
    }
    found
}

async fn execute(ctx: &ToolCtx<'_>, name: &str, args: &Value) -> Value {
    match name {
        "get_project_readme" => {
            let tool = match ctx.find(args) {
                Ok(tool) => tool,
                Err(e) => return e,
            };
            let (owner, repo) = match ctx.repo(tool) {
                Ok(r) => r,
                Err(e) => return e,
            };
            let text = match ctx.readme(owner, repo).await {
                Ok(text) => text,
                Err(e) => return json!({"error": e}),
            };
            let outline: Vec<String> = readme_headings(&text)
                .into_iter()
                .take(60)
                .map(|(_, level, title)| format!("{} {title}", "#".repeat(level)))
                .collect();
            let base = json!({
                "id": tool.id,
                "title": tool.title,
                "repo": format!("{owner}/{repo}"),
                "url": format!("https://github.com/{owner}/{repo}"),
                "outline": outline,
            });
            let requested = args["section"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let (section, body) = match requested {
                Some(query) => match readme_section(&text, query) {
                    Some((title, body)) => (Some(title), body),
                    None => {
                        let mut miss = base;
                        miss["error"] = json!(format!(
                            "No README heading matches \"{query}\". Pick one from the outline."
                        ));
                        return miss;
                    }
                },
                None => (None, text.clone()),
            };
            let mut result = base;
            result["section"] = json!(section);
            result["length"] = json!(body.chars().count());
            result["truncated"] = json!(body.chars().count() > README_CHUNK);
            result["content"] = json!(clip(&body, README_CHUNK));
            result
        }
        "search_project_text" => {
            let tool = match ctx.find(args) {
                Ok(tool) => tool,
                Err(e) => return e,
            };
            let pattern = args["pattern"].as_str().unwrap_or("");
            if pattern.trim().is_empty() {
                return json!({"error": "Give a pattern to search for."});
            }
            let re = match RegexBuilder::new(pattern)
                .case_insensitive(true)
                .size_limit(1 << 20)
                .build()
            {
                Ok(re) => re,
                Err(e) => {
                    return json!({"error": format!("That isn't a valid regular expression: {e}")})
                }
            };
            let source = args["source"].as_str().unwrap_or("all");
            let mut notes = Vec::new();
            let readme = if matches!(source, "all" | "readme") {
                match ctx.repo(tool) {
                    Ok((owner, repo)) => match ctx.readme(owner, repo).await {
                        Ok(text) => Some(text),
                        Err(e) => {
                            notes.push(format!("README not searched: {e}"));
                            None
                        }
                    },
                    Err(_) => {
                        notes.push("No README: no GitHub repository is linked.".to_string());
                        None
                    }
                }
            } else {
                None
            };
            let mut sources: Vec<(&str, &str)> = Vec::new();
            if matches!(source, "all" | "description") {
                sources.push(("description", &tool.description));
            }
            if matches!(source, "all" | "release_notes") {
                if let Some(notes) = &tool.release_notes {
                    sources.push(("release_notes", notes));
                }
            }
            if let Some(text) = &readme {
                sources.push(("readme", text));
            }
            let mut matches = regex_search(&re, &sources, 25);
            if matches.is_empty() && pattern.contains("\\\\") {
                let single = pattern.replace("\\\\", "\\");
                if let Ok(re) = RegexBuilder::new(&single)
                    .case_insensitive(true)
                    .size_limit(1 << 20)
                    .build()
                {
                    matches = regex_search(&re, &sources, 25);
                    if !matches.is_empty() {
                        notes.push(format!(
                            "The pattern had doubled backslashes; searched for {single} instead."
                        ));
                    }
                }
            }
            let mut result = json!({
                "id": tool.id,
                "title": tool.title,
                "pattern": pattern,
                "searched": sources.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
                "count": matches.len(),
                "matches": matches,
            });
            if !notes.is_empty() {
                result["notes"] = json!(notes);
            }
            result
        }
        "get_github_info" => {
            let tool = match ctx.find(args) {
                Ok(tool) => tool,
                Err(e) => return e,
            };
            let (owner, repo) = match ctx.repo(tool) {
                Ok(r) => r,
                Err(e) => return e,
            };
            let base = format!("https://api.github.com/repos/{owner}/{repo}");
            let info: Value = match ctx.github(&base, "application/vnd.github+json").await {
                Ok(response) => response.json().await.unwrap_or_default(),
                Err(e) => return json!({"error": e}),
            };
            let releases: Value = match ctx
                .github(
                    &format!("{base}/releases?per_page=5"),
                    "application/vnd.github+json",
                )
                .await
            {
                Ok(response) => response.json().await.unwrap_or_default(),
                Err(_) => json!([]),
            };
            json!({
                "id": tool.id,
                "title": tool.title,
                "repo": format!("{owner}/{repo}"),
                "url": info["html_url"],
                "about": info["description"],
                "stars": info["stargazers_count"],
                "forks": info["forks_count"],
                "openIssues": info["open_issues_count"],
                "license": info["license"]["spdx_id"],
                "language": info["language"],
                "topics": info["topics"],
                "archived": info["archived"],
                "createdAt": info["created_at"],
                "lastPush": info["pushed_at"],
                "releases": releases.as_array().into_iter().flatten().map(|r| json!({
                    "tag": r["tag_name"],
                    "name": r["name"],
                    "published": r["published_at"],
                    "prerelease": r["prerelease"],
                    "notes": clip(r["body"].as_str().unwrap_or("").trim(), 500),
                })).collect::<Vec<_>>(),
            })
        }
        _ => run_tool(name, args, ctx.tools, ctx.system),
    }
}

type Requested = Option<(String, Vec<String>, Vec<Option<String>>)>;

async fn run_agent(
    client: &reqwest::Client,
    c: &Conn,
    system: &str,
    mut history: Vec<Msg>,
    ctx: &ToolCtx<'_>,
    live: &mut Live,
) -> (Requested, bool) {
    let mut requested: Requested = None;
    let mut stopped = false;
    let mut nudged = false;
    let mut finished = false;
    let quiet_conn = Conn {
        thinking: false,
        ..c.clone()
    };
    let mut quiet = false;
    for _ in 0..MAX_ROUNDS {
        let conn = if std::mem::take(&mut quiet) && c.protocol != Protocol::Anthropic {
            &quiet_conn
        } else {
            c
        };
        let round = match stream_round(client, conn, system, &history, live).await {
            Ok(round) => round,
            Err(e) => {
                live.error(&e);
                finished = true;
                break;
            }
        };
        live.end_thinking();
        if round.stopped {
            stopped = true;
            finished = true;
            break;
        }
        let refused = round.stop.as_deref() == Some("refusal")
            || round
                .stop
                .as_deref()
                .is_some_and(|s| s.starts_with("blocked") || s == "SAFETY");
        history.push(Msg::Assistant {
            text: round.text.clone(),
            raw: Some(round.raw),
        });
        if refused {
            live.error("The model declined to answer this. Try rephrasing the request.");
            finished = true;
            break;
        }
        if round.calls.is_empty() {
            if round.text.trim().is_empty() && !nudged {
                nudged = true;
                quiet = true;
                history.push(Msg::User(
                    "Please answer my question now, based on what you found.".into(),
                ));
                continue;
            }
            finished = true;
            break;
        }
        let truncated = round.stop.as_deref() == Some("max_tokens");
        let mut results = Vec::new();
        for call in round.calls {
            live.tool_started(&call.id, &call.name, &call.arguments);
            let result = if let Some(raw) = &call.invalid {
                json!({"error": "The arguments were not valid JSON.", "INVALID_JSON": raw})
            } else if truncated {
                json!({"error": "The reply was cut off before this call was complete. Try again with a shorter call."})
            } else {
                execute(ctx, &call.name, &call.arguments).await
            };
            let ok = result.get("error").is_none();
            let summary = summarize(&call.name, &result);
            let pretty = serde_json::to_string_pretty(&result).unwrap_or_default();
            live.tool_finished(&call.id, clip(&pretty, 6000), summary, ok);
            if let (true, Some(action)) = (
                is_action_tool(&call.name),
                result["requestedAction"].as_str(),
            ) {
                let ids: Vec<String> = result["projectIds"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                let url = result["artifactUrl"].as_str().map(str::to_string);
                let entry =
                    requested.get_or_insert_with(|| (action.to_string(), Vec::new(), Vec::new()));
                if entry.0 == action {
                    for id in ids {
                        if !entry.1.contains(&id) {
                            entry.1.push(id);
                            entry.2.push(url.clone());
                        }
                    }
                }
            }
            results.push(ToolResult {
                id: call.id,
                name: call.name,
                content: result.to_string(),
                is_error: !ok,
            });
        }
        history.push(Msg::Results(results));
    }
    if !finished {
        live.error(
            "The assistant kept calling tools without answering. Try a more specific question.",
        );
    }
    (requested, stopped)
}

#[tauri::command]
pub async fn assistant_chat(
    app: tauri::AppHandle,
    state: State<'_, Shared>,
    chat_id: String,
    message: String,
    projects: Vec<Tool>,
) -> Result<Reply, String> {
    let message = message.trim();
    if message.is_empty() {
        return Err("Write a message first.".into());
    }
    let started = Instant::now();
    let state_ref = state.inner().clone();
    stop_requests()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&chat_id);
    let mut live = Live {
        app: Some(app),
        chat_id: chat_id.clone(),
        parts: Vec::new(),
        thinking_since: None,
    };
    live.emit("turn_started", None, None, None);
    let mut thread = {
        let _guard = state_ref
            .assistant_chats_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut threads = conversations(&state_ref)?;
        let thread = threads
            .iter_mut()
            .find(|thread| thread.id == chat_id)
            .ok_or("This chat was deleted or is no longer available.")?;
        if thread.title == "New chat" {
            thread.title = title_from(message);
        }
        thread.messages.push(Turn {
            role: "user".into(),
            content: message.to_string(),
            project_ids: Vec::new(),
            artifact_urls: Vec::new(),
            action: None,
            parts: Vec::new(),
            model: None,
            elapsed_ms: None,
            stopped: false,
            steps: Vec::new(),
            activity: Vec::new(),
        });
        thread.updated_at = now_nanos();
        let saved_thread = thread.clone();
        save_conversations(&state_ref, &threads)?;
        saved_thread
    };
    let mut tools = projects;
    tools.truncate(500);
    let host_os = json!(state_ref.host.os);
    for tool in &mut tools {
        tool.repo = state_ref
            .project(&tool.id)
            .and_then(|project| project.github_repo());
        tool.buildable = can_build(tool, &host_os);
    }
    let github_token = state_ref
        .github_token
        .read()
        .ok()
        .and_then(|token| token.clone());
    let tools = &tools[..];
    let host = &state_ref.host;
    let mut system_info = json!({
        "name": host.name,
        "os": host.os,
        "arch": host.arch,
    });
    if host_os == "linux" {
        system_info["linuxFamily"] = json!(host.linux_family);
        system_info["installFormat"] = json!(state_ref.settings().linux_format);
    }
    let system = system_prompt(tools, &system_info);

    let mut history = Vec::new();
    let earlier = &thread.messages[..thread.messages.len() - 1];
    for m in earlier
        .iter()
        .rev()
        .take(12)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let text = clip(m.content.trim(), 3000);
        if text.is_empty() {
            continue;
        }
        history.push(if m.role == "user" {
            Msg::User(text)
        } else {
            Msg::Assistant { text, raw: None }
        });
    }
    history.push(Msg::User(message.to_string()));

    let model_label = match conn(&state_ref) {
        Ok(c) => c.model.clone(),
        Err(_) => state_ref.settings().assistant_model,
    };
    let (requested, stopped) = match conn(&state_ref) {
        Err(e) => {
            live.error(&e);
            (None, false)
        }
        Ok(c) => {
            let ctx = ToolCtx {
                client: &state_ref.client,
                github_token: github_token.as_deref(),
                tools,
                system: &system_info,
            };
            let client = client_for(&state_ref.client, &c.base);
            run_agent(client, &c, &system, history, &ctx, &mut live).await
        }
    };
    live.end_thinking();
    let mut text = live.answer();
    let (action, project_ids, artifact_urls) = match requested {
        Some((action, ids, urls)) if !stopped => {
            if text.is_empty() {
                let names = ids
                    .iter()
                    .filter_map(|id| tools.iter().find(|t| &t.id == id).map(|t| t.title.as_str()))
                    .collect::<Vec<_>>()
                    .join(" and ");
                text = match action.as_str() {
                    "install" => format!("Installing {names}."),
                    "download" => format!("Downloading {names}."),
                    "update_now" => format!("Updating {names}."),
                    "uninstall" => format!("Review the removal of {names} below."),
                    "launch" => format!("Opening {names}."),
                    "build" => format!("Building {names} from source."),
                    _ => String::new(),
                };
            }
            (action, ids, urls)
        }
        _ => ("none".to_string(), mentioned(&text, tools), Vec::new()),
    };
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let reply = Reply {
        text: text.clone(),
        project_ids,
        action,
        artifact_urls,
        parts: live.parts.clone(),
        model: model_label,
        elapsed_ms,
        stopped,
    };
    if thread.messages.len() == 1
        && message.split_whitespace().count() <= 3
        && !reply.project_ids.is_empty()
    {
        thread.title = reply
            .project_ids
            .iter()
            .take(2)
            .filter_map(|id| tools.iter().find(|t| &t.id == id).map(|t| t.title.as_str()))
            .collect::<Vec<_>>()
            .join(" & ");
    }
    thread.messages.push(Turn {
        role: "assistant".into(),
        content: text,
        project_ids: reply.project_ids.clone(),
        artifact_urls: reply.artifact_urls.clone(),
        action: Some(reply.action.clone()),
        parts: reply.parts.clone(),
        model: Some(reply.model.clone()),
        elapsed_ms: Some(elapsed_ms),
        stopped,
        steps: Vec::new(),
        activity: Vec::new(),
    });
    thread.updated_at = now_nanos();
    {
        let _guard = state_ref
            .assistant_chats_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut threads = conversations(&state_ref)?;
        if let Some(saved) = threads.iter_mut().find(|saved| saved.id == chat_id) {
            *saved = thread;
            save_conversations(&state_ref, &threads)?;
        }
    }
    stop_requests()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&chat_id);
    Ok(reply)
}

#[tauri::command]
pub async fn assistant_test(state: State<'_, Shared>) -> Result<String, String> {
    complete(
        state.inner(),
        &[json!({"role":"user", "content":"Reply with exactly: Connection successful"})],
    )
    .await
}

#[tauri::command]
pub async fn assistant_models(state: State<'_, Shared>) -> Result<Vec<String>, String> {
    let (p, base, secret) = connection(state.inner())?;
    let client = client_for(&state.client, &base);
    list_models(client, &p, base, secret).await
}

async fn list_models(
    client: &reqwest::Client,
    p: &str,
    base: String,
    secret: Option<String>,
) -> Result<Vec<String>, String> {
    let (url, ollama) = if p == "ollama" {
        (
            format!(
                "{}/api/tags",
                base.trim_end_matches('/').trim_end_matches("/v1")
            ),
            true,
        )
    } else if p == "gemini" {
        (format!("{}/models", base.trim_end_matches('/')), false)
    } else if p == "anthropic" {
        (
            format!(
                "{}/v1/models?limit=100",
                base.trim_end_matches('/').trim_end_matches("/v1")
            ),
            false,
        )
    } else {
        (
            if base.ends_with("/models") {
                base
            } else if base.ends_with("/v1") {
                format!("{base}/models")
            } else {
                format!("{}/v1/models", base.trim_end_matches('/'))
            },
            false,
        )
    };
    let mut req = endpoint(client, reqwest::Method::GET, &url);
    if p == "gemini" {
        req = req.header("x-goog-api-key", secret.unwrap_or_default());
    } else if p == "anthropic" {
        req = req
            .header("x-api-key", secret.unwrap_or_default())
            .header("anthropic-version", "2023-06-01");
    } else if let Some(k) = secret.filter(|k| !k.is_empty()) {
        req = req.bearer_auth(k);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("Could not reach model provider: {e}"))?;
    let status = resp.status();
    let body = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!(
            "Provider returned HTTP {}: {}",
            status.as_u16(),
            body.chars().take(300).collect::<String>()
        ));
    }
    let v: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let list = if ollama {
        v["models"].as_array()
    } else {
        v["data"].as_array().or_else(|| v["models"].as_array())
    };
    Ok(list
        .into_iter()
        .flatten()
        .filter(|m| {
            m["supportedGenerationMethods"]
                .as_array()
                .is_none_or(|methods| methods.iter().any(|x| x == "generateContent"))
        })
        .filter_map(|m| {
            m["name"]
                .as_str()
                .or_else(|| m["id"].as_str())
                .map(str::to_string)
        })
        .map(|m| m.strip_prefix("models/").unwrap_or(&m).to_string())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn real_catalogue() -> Vec<Tool> {
        let projects: Vec<Value> =
            serde_json::from_str(include_str!("../testdata/catalogue.json")).unwrap();
        projects
            .iter()
            .map(|p| {
                test_tool(
                    p["id"].as_str().unwrap(),
                    p["title"].as_str().unwrap(),
                    p["description"].as_str().unwrap_or(""),
                    false,
                    None,
                )
            })
            .collect()
    }

    fn test_tool(
        id: &str,
        title: &str,
        description: &str,
        installed: bool,
        update: Option<&str>,
    ) -> Tool {
        Tool {
            id: id.into(),
            title: title.into(),
            description: description.into(),
            category: None,
            url: None,
            recommended: None,
            links: Vec::new(),
            video: None,
            artifacts: Vec::new(),
            version: Some("2.0".into()),
            released: None,
            installed,
            installed_version: installed.then(|| "1.0".into()),
            update: update.map(str::to_string),
            release_notes: None,
            installable: true,
            removable: installed,
            buildable: false,
            repo: None,
        }
    }

    fn catalogue() -> Vec<Tool> {
        vec![
            test_tool(
                "zpad",
                "ZPad",
                "Fast, native, distraction-free text editor for Windows, macOS, and Linux.",
                false,
                None,
            ),
            test_tool(
                "zhell",
                "Zhell",
                "A fast GPU terminal that remembers every command.",
                false,
                None,
            ),
            test_tool(
                "ztype",
                "ZType",
                "A typing-speed trainer that runs entirely in your terminal.",
                false,
                None,
            ),
            test_tool(
                "zencrypt",
                "ZEncrypt",
                "Drag-and-drop file & folder encryptor. Password, keyfile, or both.",
                false,
                None,
            ),
            test_tool(
                "zgamelib",
                "ZGameLib",
                "Free, open-source personal game library replacing Steam.",
                false,
                None,
            ),
        ]
    }

    #[test]
    fn compatible_endpoint_paths_cover_common_base_urls() {
        assert_eq!(
            chat_url("openai-compatible", "http://localhost:1234/v1"),
            "http://localhost:1234/v1/chat/completions"
        );
        assert_eq!(
            chat_url(
                "openai-compatible",
                "http://localhost:1234/v1/chat/completions"
            ),
            "http://localhost:1234/v1/chat/completions"
        );
        assert_eq!(
            ollama_chat_url("http://localhost:11434"),
            "http://localhost:11434/api/chat"
        );
        assert_eq!(
            ollama_chat_url("http://localhost:11434/v1/"),
            "http://localhost:11434/api/chat"
        );
    }

    #[test]
    fn search_understands_intent_and_typos() {
        let tools = catalogue();
        assert_eq!(search_tools("test how fast i type", &tools)[0].id, "ztype");
        assert_eq!(search_tools("typing speed", &tools)[0].id, "ztype");
        assert_eq!(search_tools("text editr", &tools)[0].id, "zpad");
        assert_eq!(search_tools("encrypt my files", &tools)[0].id, "zencrypt");
        assert_eq!(search_tools("games", &tools)[0].id, "zgamelib");
        assert_eq!(search_tools("zhell", &tools)[0].id, "zhell");
        assert!(search_tools("spreadsheet", &tools).is_empty());
    }

    #[test]
    fn search_on_the_real_catalogue_puts_the_right_tool_first() {
        let tools = real_catalogue();
        for (query, want) in [
            ("typing speed test", "ztype"),
            ("encrypt files", "zencrypt"),
            ("text editor", "zpad"),
            ("live wallpaper", "zwall"),
            ("rename many files", "zrename"),
            ("clipboard history", "zclipboardmanager"),
            ("disk space", "zdriveclean"),
            ("jwt decode json format", "zdataforge"),
            ("hide files inside images", "z-stego"),
            ("password generator browser", "zpassword"),
        ] {
            let found = search_tools(query, &tools);
            let scores: Vec<String> = search_scored(query, &tools)
                .iter()
                .take(6)
                .map(|(s, t, _)| format!("{}={s:.1}", t.id))
                .collect();
            assert_eq!(
                found.first().map(|t| t.id.as_str()),
                Some(want),
                "{query}: {scores:?}"
            );
            assert!(
                found.len()
                    <= if query.split_whitespace().count() >= 3 {
                        6
                    } else {
                        4
                    },
                "{query}: too many weak matches: {:?}",
                found.iter().map(|t| &t.id).collect::<Vec<_>>()
            );
        }
    }

    const README: &str = "# ZType <img src=\"x.png\">\n\n[![CI](https://img.shields.io/ci.svg)](https://ci) ![shot](a.png)\n\nA typing trainer.<!-- hidden\nnote -->\n\n## Install\n\nRun `cargo install ztype`.\n\n### From source\n\n```sh\n# not a heading\ncargo build <T>\n```\n\n## Keyboard shortcuts\n\n- Ctrl+R restart\n- Ctrl+Q quit\n\n\n\n## License\n\nMIT\n";

    #[test]
    fn readmes_are_cleaned_for_the_model() {
        let text = clean_readme(README);
        assert!(text.starts_with("# ZType\n\nA typing trainer."));
        assert!(
            !text.contains("shields.io") && !text.contains("hidden") && !text.contains("a.png")
        );
        assert!(
            text.contains("cargo build <T>"),
            "code blocks stay untouched"
        );
        assert!(!text.contains("\n\n\n"));
        let headings: Vec<String> = readme_headings(&text)
            .into_iter()
            .map(|(_, l, t)| format!("{l} {t}"))
            .collect();
        assert_eq!(
            headings,
            [
                "1 ZType",
                "2 Install",
                "3 From source",
                "2 Keyboard shortcuts",
                "2 License"
            ]
        );
    }

    #[test]
    fn readme_sections_end_at_the_next_heading_of_their_level() {
        let text = clean_readme(README);
        let (title, body) = readme_section(&text, "install").unwrap();
        assert_eq!(title, "Install");
        assert!(body.contains("From source") && body.contains("cargo build"));
        assert!(!body.contains("Ctrl+R"));
        let (title, body) = readme_section(&text, "## shortcuts").unwrap();
        assert_eq!(title, "Keyboard shortcuts");
        assert!(body.ends_with("Ctrl+Q quit"));
        assert!(readme_section(&text, "changelog").is_none());
        let (title, _) = readme_section(&text, "shortcut keys").unwrap();
        assert_eq!(title, "Keyboard shortcuts");
        let (title, _) = readme_section(&text, "installation").unwrap();
        assert_eq!(title, "Install");
    }

    #[test]
    fn regex_search_returns_lines_and_capture_groups() {
        let text = clean_readme(README);
        let re = RegexBuilder::new(r"ctrl\+(\w)\s+(\w+)")
            .case_insensitive(true)
            .build()
            .unwrap();
        let hits = regex_search(&re, &[("readme", &text)], 25);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0]["groups"], json!(["R", "restart"]));
        assert_eq!(hits[1]["match"], "Ctrl+Q quit");
        let long = format!("{} needle {}", "a".repeat(400), "b".repeat(400));
        let re = Regex::new("needle").unwrap();
        let hit = &regex_search(&re, &[("description", &long)], 25)[0];
        let snippet = hit["text"].as_str().unwrap();
        assert!(snippet.starts_with('…') && snippet.ends_with('…') && snippet.contains("needle"));
        assert!(snippet.chars().count() < 320);
    }

    #[tokio::test]
    async fn regex_search_forgives_double_escaping() {
        let tools = [test_tool(
            "zhell",
            "Zhell",
            "Press Ctrl+R to search history. Ctrl+T opens a tab.",
            true,
            None,
        )];
        let client = reqwest::Client::new();
        let ctx = ToolCtx {
            client: &client,
            github_token: None,
            tools: &tools,
            system: &json!({}),
        };
        let args =
            json!({"project_id": "zhell", "pattern": "ctrl\\\\+(\\\\w)", "source": "description"});
        let result = execute(&ctx, "search_project_text", &args).await;
        assert_eq!(result["count"], 1, "{result}");
        assert_eq!(result["matches"][0]["groups"], json!(["R"]));
        assert!(result["notes"][0]
            .as_str()
            .unwrap()
            .contains("doubled backslashes"));
        let bad = execute(
            &ctx,
            "search_project_text",
            &json!({"project_id": "zhell", "pattern": "(unclosed"}),
        )
        .await;
        assert!(bad["error"]
            .as_str()
            .unwrap()
            .contains("valid regular expression"));
        let none = execute(&ctx, "get_project_readme", &json!({"project_id": "zhell"})).await;
        assert!(none["error"]
            .as_str()
            .unwrap()
            .contains("no GitHub repository"));
    }

    #[test]
    fn chat_titles_cut_at_a_word() {
        assert_eq!(title_from("hey"), "Hey");
        assert_eq!(
            title_from("im searching for a programm wher i cna test how fast i ype"),
            "Im searching for a programm wher i cna…"
        );
        assert_eq!(title_from("  two\n lines "), "Two lines");
        assert!(title_from(&"x".repeat(80)).chars().count() <= 41);
    }

    #[test]
    fn about_zsync_knows_the_one_person_behind_it() {
        let mut tools = catalogue();
        tools[0].released = Some("2026-09-01".into());
        tools[2].released = Some("2026-10-01".into());
        let all = run_tool("about_zsync", &json!({}), &tools, &json!({}));
        let text = all["content"].as_str().unwrap();
        assert!(
            text.contains("TheHolyOneZ")
                && text.contains("Etienne Busch")
                && text.contains("one-person")
        );
        assert_eq!(all["live"]["toolsInCatalogue"], 5);
        assert_eq!(all["live"]["newestReleases"][0]["title"], "ZType");
        assert_eq!(summarize("about_zsync", &all), "TheHolyOneZ · 5 tools");
        let contact = run_tool(
            "about_zsync",
            &json!({"section": "contact"}),
            &tools,
            &json!({}),
        );
        assert_eq!(contact["section"], "Contact and support");
        assert!(contact["content"]
            .as_str()
            .unwrap()
            .contains("zsync.eu/ttz"));
        assert!(
            system_prompt(&tools, &json!({"name": "x"})).contains("never say \"the ZSync team\"")
        );
    }

    #[test]
    fn local_servers_skip_the_proxy_and_builds_follow_the_tool_page_rule() {
        for local in [
            "http://localhost:11434",
            "http://127.0.0.1:1234/v1",
            "http://[::1]:11434",
            "http://LOCALHOST",
        ] {
            assert!(is_loopback(local), "{local}");
        }
        for remote in [
            "https://api.anthropic.com",
            "http://192.168.1.5:11434",
            "https://localhost.example.com",
            "http://my-127.host",
        ] {
            assert!(!is_loopback(remote), "{remote}");
        }
        let mut tool = test_tool("zx", "ZX", "", false, None);
        let linux = json!("linux");
        assert!(!can_build(&tool, &linux), "no repository");
        tool.repo = Some(("TheHolyOneZ".into(), "ZX".into()));
        assert!(can_build(&tool, &linux), "no files listed at all");
        tool.artifacts = vec![json!({"os": "windows", "url": "u"})];
        assert!(!can_build(&tool, &linux), "Windows-only tool on Linux");
        assert!(can_build(&tool, &json!("windows")));
    }

    #[test]
    fn dates_come_out_right() {
        assert_eq!(date_of(0), "1970-01-01");
        assert_eq!(date_of(19_723), "2024-01-01");
        assert_eq!(date_of(19_782), "2024-02-29");
        assert_eq!(today().len(), 10);
    }

    #[test]
    fn broad_searches_keep_the_tool_that_fits_one_word_well() {
        let tools = real_catalogue();
        for query in [
            "hex converter base64 calculator math utility",
            "calculator encoder decoder math hex base64",
        ] {
            let hits = search_hits(query, &tools);
            let ids: Vec<&str> = hits.iter().map(|(t, _)| t.id.as_str()).collect();
            let forge = hits.iter().find(|(t, _)| t.id == "zdataforge");
            assert!(forge.is_some(), "{query}: {ids:?}");
            assert!(forge.unwrap().1.contains(&"hex".to_string()));
            assert!(hits.len() <= 6);
        }
        let result = run_tool(
            "search_projects",
            &json!({"query": "hex converter base64"}),
            &tools,
            &json!({}),
        );
        let forge = result["matches"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == "zdataforge")
            .unwrap();
        assert_eq!(forge["matchedWords"], json!(["hex"]));
    }

    #[test]
    fn think_tags_are_split_across_chunks() {
        let mut tags = ThinkTags::default();
        let mut out = Vec::new();
        for chunk in ["<thi", "nk>plan", " this</th", "ink>Hello", " <", "b>"] {
            tags.push(chunk, &mut out);
        }
        tags.finish(&mut out);
        let thinking: String = out
            .iter()
            .filter(|(t, _)| *t)
            .map(|(_, s)| s.as_str())
            .collect();
        let text: String = out
            .iter()
            .filter(|(t, _)| !*t)
            .map(|(_, s)| s.as_str())
            .collect();
        assert_eq!(thinking, "plan this");
        assert_eq!(text, "Hello <b>");
    }

    #[test]
    fn answers_link_the_tools_they_name() {
        let tools = catalogue();
        let ids = mentioned("Try **ZType** — or ZPad-like editors. Also Zhell.", &tools);
        assert_eq!(ids, vec!["ztype", "zhell"]);
    }

    #[test]
    fn update_check_tool_reads_current_metadata() {
        let tools = [
            test_tool("zpad", "ZPad", "Text editor", true, Some("2.0")),
            test_tool("zshell", "ZShell", "Terminal", true, None),
        ];
        let result = run_tool("check_updates", &json!({}), &tools, &json!({}));
        assert_eq!(result["count"], 1);
        assert_eq!(result["updates"][0]["id"], "zpad");
        assert_eq!(result["updates"][0]["newVersion"], "2.0");
        assert_eq!(summarize("check_updates", &result), "1 update available");
    }

    #[test]
    fn action_tools_validate_project_capability() {
        let tools = [test_tool("zpad", "ZPad", "Text editor", false, None)];
        let allowed = run_tool(
            "install_project",
            &json!({"project_id":"zpad"}),
            &tools,
            &json!({}),
        );
        assert_eq!(allowed["requestedAction"], "install");
        assert_eq!(allowed["projectIds"][0], "zpad");
        let removed = run_tool(
            "uninstall_project",
            &json!({"project_id":"zpad"}),
            &tools,
            &json!({}),
        );
        assert!(removed["error"].is_string());
        let unknown = run_tool(
            "install_project",
            &json!({"project_id":"nope"}),
            &tools,
            &json!({}),
        );
        assert!(unknown["error"].as_str().unwrap().contains("nope"));
    }

    #[test]
    fn native_tool_schemas_are_exposed_for_each_provider_protocol() {
        let defs = function_definitions();
        assert!(defs.iter().any(|tool| tool["name"] == "get_system_info"));
        let openai = openai_tools(&defs);
        assert_eq!(openai[0]["type"], "function");
        assert!(openai[0]["function"]["parameters"]["properties"]["query"].is_object());
        let anthropic = anthropic_tools(&defs, true);
        assert!(anthropic[0]["input_schema"]["properties"]["query"].is_object());
        assert_eq!(anthropic[0]["eager_input_streaming"], true);
        let gemini = gemini_tools(&defs);
        let declarations = gemini[0]["functionDeclarations"].as_array().unwrap();
        let list = declarations
            .iter()
            .find(|d| d["name"] == "list_installed")
            .unwrap();
        assert!(list.get("parameters").is_none());
        let search = declarations
            .iter()
            .find(|d| d["name"] == "search_projects")
            .unwrap();
        assert!(search["parameters"].get("additionalProperties").is_none());
    }

    #[test]
    fn anthropic_requests_pick_the_right_thinking_mode() {
        let c = |model: &str| Conn {
            provider: "anthropic".into(),
            protocol: Protocol::Anthropic,
            model: model.into(),
            base: "https://api.anthropic.com".into(),
            secret: None,
            thinking: true,
        };
        let history = [Msg::User("hi".into())];
        let new = anthropic_body(&c("claude-opus-5-5"), "s", &history, false);
        assert_eq!(new["thinking"]["type"], "adaptive");
        assert_eq!(new["thinking"]["display"], "summarized");
        assert_eq!(new["fallbacks"], "default");
        let old = anthropic_body(&c("claude-haiku-4-5"), "s", &history, false);
        assert_eq!(old["thinking"]["type"], "enabled");
        assert!(old.get("fallbacks").is_none());
        let lite = anthropic_body(&c("claude-opus-5-5"), "s", &history, true);
        assert!(lite.get("thinking").is_none());
        assert!(lite.get("fallbacks").is_none());
    }

    #[tokio::test]
    #[ignore]
    async fn live_ollama() {
        let library: Value = serde_json::from_str(
            &std::fs::read_to_string("../.dev-data/mock-library.json").unwrap(),
        )
        .unwrap();
        let tools: Vec<Tool> = library["projects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| Tool {
                id: p["id"].as_str().unwrap().into(),
                title: p["title"].as_str().unwrap().into(),
                description: p["description"].as_str().unwrap_or("").into(),
                category: p["category"].as_str().map(str::to_string),
                url: p["url"].as_str().map(str::to_string),
                recommended: p["recommended"].as_str().map(str::to_string),
                links: p["links"].as_array().cloned().unwrap_or_default(),
                video: Some(p["video"].clone()).filter(|v| !v.is_null()),
                artifacts: p["artifacts"].as_array().cloned().unwrap_or_default(),
                version: p["version"].as_str().map(str::to_string),
                released: p["released"].as_str().map(str::to_string),
                installed: p["installed"]["present"] == true,
                installed_version: p["installed"]["version"].as_str().map(str::to_string),
                update: p["installed"]["update"].as_str().map(str::to_string),
                release_notes: p["releaseNotes"].as_str().map(str::to_string),
                installable: p["recommended"].is_string(),
                removable: p["installed"]["present"] == true,
                buildable: false,
                repo: p["links"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find_map(|l| zl_catalog::parse_github(l["url"].as_str().unwrap_or(""))),
            })
            .collect();
        let info = json!({"name": "CachyOS Linux (x64)"});
        let c = Conn {
            provider: "ollama".into(),
            protocol: Protocol::Ollama,
            model: std::env::var("ZL_ASSISTANT_MODEL").unwrap_or("qwen3.5:9b".into()),
            base: "http://localhost:11434".into(),
            secret: None,
            thinking: std::env::var("ZL_ASSISTANT_THINK").map_or(true, |v| v != "0"),
        };
        let question = std::env::var("ZL_ASSISTANT_Q")
            .unwrap_or("im searching for a programm wher i cna test how fast i ype".into());
        let mut live = Live {
            app: None,
            chat_id: "test".into(),
            parts: Vec::new(),
            thinking_since: None,
        };
        let t = Instant::now();
        let client = reqwest::Client::builder()
            .user_agent("zlauncher-test")
            .build()
            .unwrap();
        let ctx = ToolCtx {
            client: &client,
            github_token: None,
            tools: &tools,
            system: &info,
        };
        let (requested, _) = run_agent(
            &client,
            &c,
            &system_prompt(&tools, &info),
            vec![Msg::User(question)],
            &ctx,
            &mut live,
        )
        .await;
        for part in &live.parts {
            match part.kind.as_str() {
                "tool" => println!(
                    "[tool] {} {} -> {}",
                    part.name.as_deref().unwrap_or(""),
                    part.input.clone().unwrap_or_default(),
                    part.summary.as_deref().unwrap_or("")
                ),
                kind => println!("[{kind} {:?}ms] {}", part.ms, clip(&part.text, 600)),
            }
        }
        println!(
            "cards: {:?}  action: {:?}  in {:?}",
            mentioned(&live.answer(), &tools),
            requested,
            t.elapsed()
        );
        assert!(!live.answer().is_empty());
    }

    #[tokio::test]
    async fn openai_compatible_request_round_trips_through_a_mock_api() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = stream.read(&mut buf).unwrap();
                request.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&request);
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text[..end]
                        .lines()
                        .find_map(|line| {
                            let (k, v) = line.split_once(':')?;
                            k.eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let request_text = String::from_utf8_lossy(&request);
            assert!(request_text.starts_with("POST /v1/chat/completions "));
            assert!(request_text
                .to_ascii_lowercase()
                .contains("authorization: bearer test-secret"));
            assert!(request_text.contains("\"model\":\"qwen-local\""));
            let body = r#"{"choices":[{"message":{"content":"mocked reply"}}]}"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        });
        let client = reqwest::Client::new();
        let messages = vec![json!({"role":"user", "content":"hello"})];
        let response = openai_request(
            &client,
            &format!("http://{addr}/v1/chat/completions"),
            "qwen-local",
            Some("test-secret"),
            &messages,
        )
        .send()
        .await
        .unwrap();
        let value: Value = response.json().await.unwrap();
        let reply = value["choices"][0]["message"]["content"].as_str().unwrap();
        assert_eq!(reply, "mocked reply");
        server.join().unwrap();
    }

    use std::sync::Arc;

    #[derive(Clone, Debug)]
    struct Seen {
        method: String,
        path: String,
        headers: HashMap<String, String>,
        body: Value,
    }

    enum Answer {
        Json(u16, String),
        Stream(&'static str, Vec<String>),
    }

    struct MockApi {
        base: String,
        seen: Arc<Mutex<Vec<Seen>>>,
    }

    impl MockApi {
        fn start(answers: Vec<Answer>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let seen = Arc::new(Mutex::new(Vec::new()));
            let log = seen.clone();
            std::thread::spawn(move || {
                for answer in answers {
                    let Ok((mut stream, _)) = listener.accept() else {
                        return;
                    };
                    let request = read_request(&mut stream);
                    log.lock().unwrap().push(request);
                    match answer {
                        Answer::Json(status, body) => {
                            let _ = write!(stream, "HTTP/1.1 {status} Mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                        }
                        Answer::Stream(kind, chunks) => {
                            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n");
                            for chunk in chunks {
                                if stream
                                    .write_all(chunk.as_bytes())
                                    .and_then(|_| stream.flush())
                                    .is_err()
                                {
                                    break;
                                }
                                std::thread::sleep(Duration::from_millis(2));
                            }
                        }
                    }
                }
            });
            MockApi { base, seen }
        }

        fn seen(&self) -> Vec<Seen> {
            self.seen.lock().unwrap().clone()
        }
    }

    fn read_request(stream: &mut std::net::TcpStream) -> Seen {
        let mut data = Vec::new();
        let mut buf = [0u8; 8192];
        let (head_end, length) = loop {
            let n = stream.read(&mut buf).unwrap();
            assert!(n > 0, "client closed before sending a request");
            data.extend_from_slice(&buf[..n]);
            if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&data[..end]).to_string();
                let length = head
                    .lines()
                    .find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case("content-length")
                            .then(|| v.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                break (end, length);
            }
        };
        while data.len() < head_end + 4 + length {
            let n = stream.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            data.extend_from_slice(&buf[..n]);
        }
        let head = String::from_utf8_lossy(&data[..head_end]).to_string();
        let mut lines = head.lines();
        let mut first = lines.next().unwrap().split(' ');
        let method = first.next().unwrap().to_string();
        let path = first.next().unwrap().to_string();
        let headers = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        let body = serde_json::from_slice(&data[head_end + 4..]).unwrap_or(Value::Null);
        Seen {
            method,
            path,
            headers,
            body,
        }
    }

    fn split_in_two(text: String, chunks: &mut Vec<String>) {
        let mut mid = text.len() / 2;
        while !text.is_char_boundary(mid) {
            mid += 1;
        }
        chunks.push(text[..mid].to_string());
        chunks.push(text[mid..].to_string());
    }

    fn sse(events: Vec<(String, Value)>, crlf: bool) -> Answer {
        let nl = if crlf { "\r\n" } else { "\n" };
        let mut chunks = Vec::new();
        for (name, data) in events {
            let data = match data {
                Value::String(s) => s,
                v => v.to_string(),
            };
            let mut event = String::new();
            if !name.is_empty() {
                event += &format!("event: {name}{nl}");
            }
            event += &format!("data: {data}{nl}{nl}");
            split_in_two(event, &mut chunks);
        }
        Answer::Stream("text/event-stream", chunks)
    }

    fn anthropic_sse(events: Vec<Value>) -> Answer {
        sse(
            events
                .into_iter()
                .map(|e| (e["type"].as_str().unwrap_or("").to_string(), e))
                .collect(),
            false,
        )
    }
    fn a_begin() -> Value {
        json!({"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-opus-5-5","content":[],"stop_reason":null,"usage":{"input_tokens":50,"output_tokens":1}}})
    }
    fn a_start(i: usize, block: Value) -> Value {
        json!({"type":"content_block_start","index":i,"content_block":block})
    }
    fn a_delta(i: usize, delta: Value) -> Value {
        json!({"type":"content_block_delta","index":i,"delta":delta})
    }
    fn a_stop(i: usize) -> Value {
        json!({"type":"content_block_stop","index":i})
    }
    fn a_end(stop: &str) -> [Value; 2] {
        [
            json!({"type":"message_delta","delta":{"stop_reason":stop,"stop_sequence":null},"usage":{"output_tokens":20}}),
            json!({"type":"message_stop"}),
        ]
    }
    fn a_text(text: &str) -> Answer {
        let mut events = vec![
            a_begin(),
            a_start(0, json!({"type":"text","text":""})),
            a_delta(0, json!({"type":"text_delta","text":text})),
            a_stop(0),
        ];
        events.extend(a_end("end_turn"));
        anthropic_sse(events)
    }

    fn openai_sse(chunks: Vec<Value>) -> Answer {
        let mut events: Vec<(String, Value)> =
            chunks.into_iter().map(|c| (String::new(), c)).collect();
        events.push((String::new(), json!("[DONE]")));
        sse(events, false)
    }
    fn oa(delta: Value, finish: Value) -> Value {
        json!({"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
    }

    fn gemini_sse(chunks: Vec<Value>) -> Answer {
        sse(
            chunks.into_iter().map(|c| (String::new(), c)).collect(),
            true,
        )
    }
    fn gm(parts: Value, finish: Option<&str>) -> Value {
        let mut chunk = json!({"candidates":[{"content":{"role":"model","parts":parts},"index":0}],"usageMetadata":{"promptTokenCount":10},"modelVersion":"gemini-3-flash"});
        if let Some(finish) = finish {
            chunk["candidates"][0]["finishReason"] = json!(finish);
        }
        chunk
    }

    fn ndjson(lines: Vec<Value>) -> Answer {
        let mut chunks = Vec::new();
        for line in lines {
            split_in_two(format!("{line}\n"), &mut chunks);
        }
        Answer::Stream("application/x-ndjson", chunks)
    }
    fn ol(message: Value, done: bool) -> Value {
        let mut line = json!({"model":"qwen3.5:9b","created_at":"2026-10-26T12:00:00Z","message":message,"done":done});
        if done {
            line["done_reason"] = json!("stop");
            line["eval_count"] = json!(42);
        }
        line
    }

    fn mock_conn(provider: &str, base: &str, model: &str, secret: Option<&str>) -> Conn {
        Conn {
            provider: provider.into(),
            protocol: match provider {
                "anthropic" => Protocol::Anthropic,
                "gemini" => Protocol::Gemini,
                "ollama" => Protocol::Ollama,
                _ => Protocol::OpenAi,
            },
            model: model.into(),
            base: base.into(),
            secret: secret.map(str::to_string),
            thinking: true,
        }
    }

    fn test_client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    async fn agent_in(c: &Conn, question: &str, chat_id: &str) -> (Live, Requested, bool) {
        let tools = catalogue();
        let client = test_client();
        let info = json!({"name": "Test Linux (x64)"});
        let ctx = ToolCtx {
            client: &client,
            github_token: None,
            tools: &tools,
            system: &info,
        };
        let mut live = Live {
            app: None,
            chat_id: chat_id.into(),
            parts: Vec::new(),
            thinking_since: None,
        };
        let (requested, stopped) = run_agent(
            &client,
            c,
            &system_prompt(&tools, &info),
            vec![Msg::User(question.into())],
            &ctx,
            &mut live,
        )
        .await;
        (live, requested, stopped)
    }

    async fn agent(c: &Conn, question: &str) -> (Live, Requested, bool) {
        agent_in(c, question, &format!("mock-{}", now_nanos())).await
    }

    fn kinds(live: &Live) -> Vec<String> {
        live.parts
            .iter()
            .map(|p| match p.kind.as_str() {
                "tool" => format!(
                    "tool:{}{}",
                    p.name.as_deref().unwrap_or(""),
                    if p.ok == Some(true) { "" } else { "!" }
                ),
                kind => kind.to_string(),
            })
            .collect()
    }

    fn last_error(live: &Live) -> String {
        live.parts
            .iter()
            .rev()
            .find(|p| p.kind == "error")
            .map(|p| p.text.clone())
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn anthropic_streams_thinking_text_and_a_tool_loop() {
        let mut round1 = vec![
            json!({"type":"ping"}),
            a_begin(),
            a_start(0, json!({"type":"thinking","thinking":"","signature":""})),
            a_delta(
                0,
                json!({"type":"thinking_delta","thinking":"The user wants"}),
            ),
            a_delta(
                0,
                json!({"type":"thinking_delta","thinking":" a typing test."}),
            ),
            a_delta(0, json!({"type":"signature_delta","signature":"sig-1"})),
            a_stop(0),
            a_start(1, json!({"type":"text","text":""})),
            a_delta(1, json!({"type":"text_delta","text":"Let me check."})),
            a_stop(1),
            a_start(
                2,
                json!({"type":"tool_use","id":"toolu_1","name":"search_projects","input":{}}),
            ),
            a_delta(2, json!({"type":"input_json_delta","partial_json":""})),
            a_delta(
                2,
                json!({"type":"input_json_delta","partial_json":"{\"query\": \"typ"}),
            ),
            a_delta(
                2,
                json!({"type":"input_json_delta","partial_json":"ing speed\"}"}),
            ),
            a_stop(2),
        ];
        round1.extend(a_end("tool_use"));
        let api = MockApi::start(vec![
            anthropic_sse(round1),
            a_text("**ZType** fits your typing test."),
        ]);
        let c = mock_conn(
            "anthropic",
            &api.base,
            "claude-opus-5-5",
            Some("sk-ant-test"),
        );
        let (live, requested, stopped) = agent(&c, "is there a typing test?").await;

        assert_eq!(
            kinds(&live),
            ["thinking", "text", "tool:search_projects", "text"]
        );
        assert_eq!(live.parts[0].text, "The user wants a typing test.");
        assert!(live.parts[2].summary.as_deref().unwrap().contains("ZType"));
        assert_eq!(
            live.answer(),
            "Let me check.\n\n**ZType** fits your typing test."
        );
        assert!(requested.is_none() && !stopped);

        let seen = api.seen();
        assert_eq!(seen.len(), 2);
        let first = &seen[0];
        assert_eq!(
            (first.method.as_str(), first.path.as_str()),
            ("POST", "/v1/messages")
        );
        assert_eq!(first.headers["x-api-key"], "sk-ant-test");
        assert_eq!(first.headers["anthropic-version"], "2023-06-01");
        assert!(
            !first.headers.contains_key("anthropic-beta"),
            "fallbacks only on api.anthropic.com"
        );
        assert_eq!(first.body["stream"], true);
        assert_eq!(
            first.body["thinking"],
            json!({"type":"adaptive","display":"summarized"})
        );
        assert!(first.body["system"]
            .as_str()
            .unwrap()
            .contains("one-person"));
        assert!(first.body["tools"][0]["input_schema"]["properties"].is_object());
        assert!(first.body["tools"][0]
            .get("eager_input_streaming")
            .is_none());

        let msgs = seen[1].body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(
            msgs[0],
            json!({"role":"user","content":"is there a typing test?"})
        );
        assert_eq!(msgs[1]["role"], "assistant");
        assert_eq!(
            msgs[1]["content"][0],
            json!({"type":"thinking","thinking":"The user wants a typing test.","signature":"sig-1"})
        );
        assert_eq!(
            msgs[1]["content"][1],
            json!({"type":"text","text":"Let me check."})
        );
        assert_eq!(
            msgs[1]["content"][2],
            json!({"type":"tool_use","id":"toolu_1","name":"search_projects","input":{"query":"typing speed"}})
        );
        let result = &msgs[2]["content"][0];
        assert_eq!(
            (
                result["type"].as_str(),
                result["tool_use_id"].as_str(),
                result["is_error"].as_bool()
            ),
            (Some("tool_result"), Some("toolu_1"), Some(false))
        );
        assert!(result["content"].as_str().unwrap().contains("\"ztype\""));
    }

    #[tokio::test]
    async fn anthropic_errors_retries_refusals_and_fallbacks() {
        let api = MockApi::start(vec![
            Answer::Json(400, r#"{"type":"error","error":{"type":"invalid_request_error","message":"thinking: adaptive is not supported on this model"}}"#.into()),
            a_text("Hi."),
        ]);
        let (live, _, _) = agent(
            &mock_conn("anthropic", &api.base, "claude-3-5-haiku", Some("k")),
            "hi",
        )
        .await;
        let seen = api.seen();
        assert!(seen[0].body.get("thinking").is_some() && seen[1].body.get("thinking").is_none());
        assert_eq!(live.answer(), "Hi.");

        let api = MockApi::start(vec![Answer::Json(401, r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#.into())]);
        let (live, _, _) = agent(
            &mock_conn("anthropic", &api.base, "claude-opus-5-5", Some("bad")),
            "hi",
        )
        .await;
        let error = last_error(&live);
        assert!(
            error.contains("rejected the API key") && error.contains("invalid x-api-key"),
            "{error}"
        );

        let api = MockApi::start(vec![anthropic_sse(vec![
            a_begin(),
            json!({"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}),
        ])]);
        let (live, _, _) = agent(
            &mock_conn("anthropic", &api.base, "claude-opus-5-5", Some("k")),
            "hi",
        )
        .await;
        assert_eq!(last_error(&live), "Anthropic: Overloaded");

        let mut refused = vec![
            a_begin(),
            a_start(0, json!({"type":"text","text":""})),
            a_delta(0, json!({"type":"text_delta","text":"I can"})),
            a_stop(0),
        ];
        refused.extend(a_end("refusal"));
        let api = MockApi::start(vec![anthropic_sse(refused)]);
        let (live, _, _) = agent(
            &mock_conn("anthropic", &api.base, "claude-opus-5-5", Some("k")),
            "hi",
        )
        .await;
        assert!(last_error(&live).contains("declined"));

        let mut fallback = vec![
            a_begin(),
            a_start(0, json!({"type":"thinking","thinking":"","signature":""})),
            a_delta(0, json!({"type":"thinking_delta","thinking":"x"})),
            a_stop(0),
            a_start(
                1,
                json!({"type":"tool_use","id":"toolu_9","name":"install_project","input":{}}),
            ),
            a_delta(
                1,
                json!({"type":"input_json_delta","partial_json":"{\"project_id\":\"zpad\"}"}),
            ),
            a_stop(1),
            a_start(
                2,
                json!({"type":"fallback","from":{"model":"claude-opus-5-5"},"to":{"model":"claude-opus-4-8"}}),
            ),
            a_stop(2),
            a_start(3, json!({"type":"text","text":""})),
            a_delta(3, json!({"type":"text_delta","text":"Here is the answer."})),
            a_stop(3),
        ];
        fallback.extend(a_end("end_turn"));
        let api = MockApi::start(vec![anthropic_sse(fallback)]);
        let (live, requested, _) = agent(
            &mock_conn("anthropic", &api.base, "claude-opus-5-5", Some("k")),
            "install zpad",
        )
        .await;
        assert!(
            requested.is_none(),
            "the declined attempt's install must not run"
        );
        assert!(!kinds(&live).iter().any(|k| k.starts_with("tool:")));
        assert_eq!(live.answer(), "Here is the answer.");
        assert_eq!(api.seen().len(), 1);
    }

    #[tokio::test]
    async fn openai_compatible_streams_reasoning_and_fragmented_parallel_tool_calls() {
        let round1 = openai_sse(vec![
            oa(
                json!({"role":"assistant","content":null,"reasoning_content":"Looking for"}),
                Value::Null,
            ),
            oa(json!({"reasoning_content":" a typing app."}), Value::Null),
            oa(
                json!({"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"search_projects","arguments":""}}]}),
                Value::Null,
            ),
            oa(
                json!({"tool_calls":[{"index":0,"function":{"arguments":"{\"que"}}]}),
                Value::Null,
            ),
            oa(
                json!({"tool_calls":[{"index":0,"function":{"arguments":"ry\": \"typing speed\"}"}}]}),
                Value::Null,
            ),
            oa(
                json!({"tool_calls":[{"index":1,"id":"call_2","type":"function","function":{"name":"get_project_details","arguments":"{\"project_id\":\"ztype\"}"}}]}),
                Value::Null,
            ),
            oa(json!({}), json!("tool_calls")),
        ]);
        let round2 = openai_sse(vec![
            oa(
                json!({"role":"assistant","content":"**ZType**"}),
                Value::Null,
            ),
            oa(json!({"content":" fits."}), Value::Null),
            oa(json!({}), json!("stop")),
        ]);
        let api = MockApi::start(vec![round1, round2]);
        let c = mock_conn(
            "openai-compatible",
            &format!("{}/v1", api.base),
            "local-model",
            Some("sk-test"),
        );
        let (live, _, _) = agent(&c, "typing test app?").await;

        assert_eq!(
            kinds(&live),
            [
                "thinking",
                "tool:search_projects",
                "tool:get_project_details",
                "text"
            ]
        );
        assert_eq!(live.parts[0].text, "Looking for a typing app.");
        assert_eq!(live.answer(), "**ZType** fits.");

        let seen = api.seen();
        assert_eq!(seen[0].path, "/v1/chat/completions");
        assert_eq!(seen[0].headers["authorization"], "Bearer sk-test");
        assert_eq!(seen[0].body["stream"], true);
        assert_eq!(seen[0].body["tools"][0]["type"], "function");
        assert!(seen[0].body.get("think").is_none() && seen[0].body.get("options").is_none());
        let msgs = seen[1].body["messages"].as_array().unwrap();
        assert_eq!(
            msgs.iter()
                .map(|m| m["role"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["system", "user", "assistant", "tool", "tool"]
        );
        assert!(msgs[2]["content"].is_null());
        assert_eq!(
            msgs[2]["tool_calls"][0],
            json!({"id":"call_1","type":"function","function":{"name":"search_projects","arguments":"{\"query\": \"typing speed\"}"}})
        );
        assert_eq!(
            msgs[2]["tool_calls"][1]["function"]["name"],
            "get_project_details"
        );
        assert_eq!(
            (
                msgs[3]["tool_call_id"].as_str(),
                msgs[4]["tool_call_id"].as_str()
            ),
            (Some("call_1"), Some("call_2"))
        );
        assert!(msgs[4]["content"]
            .as_str()
            .unwrap()
            .contains("typing-speed trainer"));
    }

    #[tokio::test]
    async fn openai_inline_think_tags_and_repeated_tool_names() {
        let round1 = openai_sse(vec![
            oa(json!({"role":"assistant","content":"<thi"}), Value::Null),
            oa(json!({"content":"nk>Plan it</think>"}), Value::Null),
            oa(
                json!({"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"check_updates","arguments":""}}]}),
                Value::Null,
            ),
            oa(
                json!({"tool_calls":[{"index":0,"function":{"name":"check_updates","arguments":"{}"}}]}),
                Value::Null,
            ),
            oa(json!({}), json!("tool_calls")),
        ]);
        let round2 = openai_sse(vec![oa(
            json!({"content":"All up to date."}),
            json!("stop"),
        )]);
        let api = MockApi::start(vec![round1, round2]);
        let (live, _, _) = agent(
            &mock_conn("openai", &format!("{}/v1", api.base), "gpt-5", Some("sk")),
            "updates?",
        )
        .await;
        assert_eq!(kinds(&live), ["thinking", "tool:check_updates", "text"]);
        assert_eq!(live.parts[0].text, "Plan it");
        assert_eq!(live.answer(), "All up to date.");

        let api = MockApi::start(vec![Answer::Json(401, r#"{"error":{"message":"Incorrect API key provided: sk-bad.","type":"invalid_request_error","param":null,"code":"invalid_api_key"}}"#.into())]);
        let (live, _, _) = agent(
            &mock_conn(
                "openai",
                &format!("{}/v1", api.base),
                "gpt-5",
                Some("sk-bad"),
            ),
            "hi",
        )
        .await;
        assert!(last_error(&live).contains("OpenAI rejected the API key"));
        assert_eq!(
            api.seen().len(),
            1,
            "no lite retry for OpenAI-style servers"
        );
    }

    #[tokio::test]
    async fn gemini_streams_thoughts_and_function_calls_with_signatures() {
        let round1 = gemini_sse(vec![
            gm(json!([{"text":"Checking the","thought":true}]), None),
            gm(json!([{"text":" catalogue.","thought":true}]), None),
            gm(
                json!([{"functionCall":{"name":"search_projects","args":{"query":"typing"}},"thoughtSignature":"c2ln"}]),
                Some("STOP"),
            ),
        ]);
        let round2 = gemini_sse(vec![
            gm(json!([{"text":"**ZType**"}]), None),
            gm(json!([{"text":" fits."}]), Some("STOP")),
        ]);
        let api = MockApi::start(vec![round1, round2]);
        let c = mock_conn(
            "gemini",
            &format!("{}/v1beta", api.base),
            "gemini-3-flash",
            Some("AIza-test"),
        );
        let (live, _, _) = agent(&c, "typing test?").await;

        assert_eq!(kinds(&live), ["thinking", "tool:search_projects", "text"]);
        assert_eq!(live.parts[0].text, "Checking the catalogue.");
        assert_eq!(live.answer(), "**ZType** fits.");

        let seen = api.seen();
        assert_eq!(
            seen[0].path,
            "/v1beta/models/gemini-3-flash:streamGenerateContent?alt=sse"
        );
        assert_eq!(seen[0].headers["x-goog-api-key"], "AIza-test");
        assert_eq!(
            seen[0].body["generationConfig"]["thinkingConfig"]["includeThoughts"],
            true
        );
        assert!(seen[0].body["systemInstruction"]["parts"][0]["text"]
            .as_str()
            .unwrap()
            .contains("TheHolyOneZ"));
        let declarations = seen[0].body["tools"][0]["functionDeclarations"]
            .as_array()
            .unwrap();
        assert!(declarations
            .iter()
            .all(|d| d["parameters"].get("additionalProperties").is_none()));
        let contents = seen[1].body["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 3);
        assert_eq!(
            contents[1],
            json!({"role":"model","parts":[{"text":"Checking the catalogue.","thought":true},{"functionCall":{"name":"search_projects","args":{"query":"typing"}},"thoughtSignature":"c2ln"}]})
        );
        let response = &contents[2]["parts"][0]["functionResponse"];
        assert_eq!(contents[2]["role"], "user");
        assert_eq!(response["name"], "search_projects");
        assert!(
            response.get("id").is_none(),
            "Gemini sent no call id, so none goes back"
        );
        assert_eq!(response["response"]["matches"][0]["id"], "ztype");

        let bad = r#"{"error":{"code":400,"message":"API key not valid. Please pass a valid API key.","status":"INVALID_ARGUMENT"}}"#;
        let api = MockApi::start(vec![
            Answer::Json(400, bad.into()),
            Answer::Json(400, bad.into()),
        ]);
        let (live, _, _) = agent(
            &mock_conn(
                "gemini",
                &format!("{}/v1beta", api.base),
                "gemini-3-flash",
                Some("x"),
            ),
            "hi",
        )
        .await;
        assert!(last_error(&live).contains("API key not valid"));
        let seen = api.seen();
        assert_eq!(seen.len(), 2);
        assert!(seen[1].body["generationConfig"]
            .get("thinkingConfig")
            .is_none());
    }

    #[tokio::test]
    async fn ollama_streams_native_thinking_and_tool_calls() {
        let round1 = ndjson(vec![
            ol(
                json!({"role":"assistant","content":"","thinking":"Find a"}),
                false,
            ),
            ol(
                json!({"role":"assistant","content":"","thinking":" typing app."}),
                false,
            ),
            ol(
                json!({"role":"assistant","content":"","tool_calls":[{"id":"call_x","function":{"index":0,"name":"search_projects","arguments":{"query":"typing speed"}}}]}),
                false,
            ),
            ol(json!({"role":"assistant","content":""}), true),
        ]);
        let round2 = ndjson(vec![
            ol(json!({"role":"assistant","content":"**ZType**"}), false),
            ol(json!({"role":"assistant","content":" fits."}), false),
            ol(json!({"role":"assistant","content":""}), true),
        ]);
        let api = MockApi::start(vec![round1, round2]);
        let (live, _, _) = agent(
            &mock_conn("ollama", &api.base, "qwen3.5:9b", None),
            "typing test?",
        )
        .await;

        assert_eq!(kinds(&live), ["thinking", "tool:search_projects", "text"]);
        assert_eq!(live.parts[0].text, "Find a typing app.");
        assert_eq!(live.answer(), "**ZType** fits.");

        let seen = api.seen();
        assert_eq!(seen[0].path, "/api/chat");
        assert_eq!(seen[0].body["think"], true);
        assert_eq!(seen[0].body["options"]["num_ctx"], OLLAMA_CONTEXT);
        assert!(!seen[0].headers.contains_key("authorization"));
        let msgs = seen[1].body["messages"].as_array().unwrap();
        assert_eq!(
            msgs.iter()
                .map(|m| m["role"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["system", "user", "assistant", "tool"]
        );
        assert_eq!(
            msgs[2]["tool_calls"][0],
            json!({"function":{"name":"search_projects","arguments":{"query":"typing speed"}}})
        );
        assert_eq!(msgs[3]["tool_name"], "search_projects");

        let api = MockApi::start(vec![Answer::Json(
            404,
            r#"{"error":"model \"nope\" not found, try pulling it first"}"#.into(),
        )]);
        let (live, _, _) = agent(&mock_conn("ollama", &api.base, "nope", None), "hi").await;
        assert!(last_error(&live).contains("couldn't find the model “nope”"));

        let closed = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", closed.local_addr().unwrap());
        drop(closed);
        let (live, _, _) = agent(&mock_conn("ollama", &base, "qwen3.5:9b", None), "hi").await;
        assert!(last_error(&live).starts_with("Ollama isn't running at"));
    }

    #[tokio::test]
    async fn a_round_that_only_thinks_gets_its_answer_without_thinking_again() {
        let round1 = ndjson(vec![
            ol(
                json!({"role":"assistant","content":"","thinking":"The answer is ZType."}),
                false,
            ),
            ol(json!({"role":"assistant","content":""}), true),
        ]);
        let round2 = ndjson(vec![
            ol(json!({"role":"assistant","content":"**ZType**."}), false),
            ol(json!({"role":"assistant","content":""}), true),
        ]);
        let api = MockApi::start(vec![round1, round2]);
        let (live, _, _) = agent(
            &mock_conn("ollama", &api.base, "qwen3.5:9b", None),
            "typing?",
        )
        .await;
        assert_eq!(kinds(&live), ["thinking", "text"]);
        assert_eq!(live.answer(), "**ZType**.");
        let seen = api.seen();
        assert_eq!(
            (
                seen[0].body["think"].as_bool(),
                seen[1].body["think"].as_bool()
            ),
            (Some(true), Some(false))
        );
        let last = seen[1].body["messages"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .clone();
        assert_eq!(
            last,
            json!({"role":"user","content":"Please answer my question now, based on what you found."})
        );
    }

    #[tokio::test]
    async fn stop_ends_a_reply_mid_stream() {
        let chat = format!("stop-{}", now_nanos());
        assistant_stop(chat.clone());
        let lines = (0..50)
            .map(|i| {
                ol(
                    json!({"role":"assistant","content":format!("word{i} ")}),
                    false,
                )
            })
            .collect();
        let api = MockApi::start(vec![ndjson(lines)]);
        let (live, requested, stopped) = agent_in(
            &mock_conn("ollama", &api.base, "qwen3.5:9b", None),
            "hi",
            &chat,
        )
        .await;
        assert!(stopped && requested.is_none());
        assert!(live.answer().is_empty());
        stop_requests().lock().unwrap().remove(&chat);
    }

    #[tokio::test]
    async fn test_connection_and_model_lists_speak_each_api() {
        let client = test_client();
        let hello = [json!({"role":"user","content":"Reply with exactly: Connection successful"})];

        let api = MockApi::start(vec![Answer::Json(200, json!({"id":"msg_1","type":"message","role":"assistant","content":[{"type":"text","text":"Connection successful"}],"stop_reason":"end_turn"}).to_string())]);
        assert_eq!(
            complete_with(
                &client,
                "anthropic",
                "claude-opus-5-5",
                &api.base,
                Some("sk-ant".into()),
                &hello
            )
            .await
            .unwrap(),
            "Connection successful"
        );
        let seen = api.seen();
        assert_eq!(
            (seen[0].path.as_str(), seen[0].headers["x-api-key"].as_str()),
            ("/v1/messages", "sk-ant")
        );

        let api = MockApi::start(vec![Answer::Json(200, json!({"candidates":[{"content":{"role":"model","parts":[{"text":"Connection successful"}]},"finishReason":"STOP"}]}).to_string())]);
        let base = format!("{}/v1beta", api.base);
        assert_eq!(
            complete_with(
                &client,
                "gemini",
                "gemini-3-flash",
                &base,
                Some("AIza".into()),
                &hello
            )
            .await
            .unwrap(),
            "Connection successful"
        );
        assert_eq!(
            api.seen()[0].path,
            "/v1beta/models/gemini-3-flash:generateContent"
        );

        for provider in ["openai", "openai-compatible", "ollama"] {
            let api = MockApi::start(vec![Answer::Json(200, json!({"id":"c","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"Connection successful"},"finish_reason":"stop"}]}).to_string())]);
            let base = if provider == "ollama" {
                api.base.clone()
            } else {
                format!("{}/v1", api.base)
            };
            let secret = (provider != "ollama").then(|| "sk".to_string());
            assert_eq!(
                complete_with(&client, provider, "m", &base, secret, &hello)
                    .await
                    .unwrap(),
                "Connection successful",
                "{provider}"
            );
            assert_eq!(api.seen()[0].path, "/v1/chat/completions", "{provider}");
        }

        let api = MockApi::start(vec![Answer::Json(200, json!({"data":[{"type":"model","id":"claude-opus-5-5","display_name":"Claude Opus 5.5"},{"type":"model","id":"claude-haiku-5-5","display_name":"Claude Haiku 5.5"}],"has_more":false}).to_string())]);
        assert_eq!(
            list_models(
                &client,
                "anthropic",
                api.base.clone(),
                Some("sk-ant".into())
            )
            .await
            .unwrap(),
            ["claude-opus-5-5", "claude-haiku-5-5"]
        );
        let seen = api.seen();
        assert_eq!(seen[0].path, "/v1/models?limit=100");
        assert_eq!(seen[0].headers["anthropic-version"], "2023-06-01");

        let api = MockApi::start(vec![Answer::Json(
            200,
            json!({"object":"list","data":[{"id":"gpt-5","object":"model","owned_by":"openai"}]})
                .to_string(),
        )]);
        assert_eq!(
            list_models(
                &client,
                "openai",
                format!("{}/v1", api.base),
                Some("sk".into())
            )
            .await
            .unwrap(),
            ["gpt-5"]
        );
        assert_eq!(api.seen()[0].headers["authorization"], "Bearer sk");

        let api = MockApi::start(vec![Answer::Json(200, json!({"models":[{"name":"models/gemini-3-flash","supportedGenerationMethods":["generateContent","countTokens"]},{"name":"models/text-embedding-004","supportedGenerationMethods":["embedContent"]}]}).to_string())]);
        assert_eq!(
            list_models(
                &client,
                "gemini",
                format!("{}/v1beta", api.base),
                Some("AIza".into())
            )
            .await
            .unwrap(),
            ["gemini-3-flash"]
        );
        assert_eq!(api.seen()[0].headers["x-goog-api-key"], "AIza");

        let api = MockApi::start(vec![Answer::Json(
            200,
            json!({"models":[{"name":"qwen3.5:9b","model":"qwen3.5:9b"}]}).to_string(),
        )]);
        assert_eq!(
            list_models(&client, "ollama", api.base.clone(), None)
                .await
                .unwrap(),
            ["qwen3.5:9b"]
        );
        assert_eq!(api.seen()[0].path, "/api/tags");
    }
}
