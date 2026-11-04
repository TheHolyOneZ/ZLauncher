use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use crypto_box::aead::generic_array::GenericArray;
use crypto_box::aead::rand_core::RngCore;
use crypto_box::aead::{Aead, AeadCore, OsRng};
use crypto_box::{PublicKey, SalsaBox, SecretKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::app::Shared;
use crate::ttz_files::{self as files, Kind, Sealer};

const BASE: &str = "https://zsync.eu/ttz";
const VAULT_KEY: &str = "ttz";
const TYPES: [&str; 5] = ["HELP", "REQUEST", "FEEDBACK", "COLLAB", "GENERAL"];
const MAX_SUBJECT: usize = 255;
const MAX_MESSAGE: usize = 20_000;
const MAX_NAME: usize = 40;
const AUTO_PREVIEW: u64 = 8 * 1024 * 1024;
const MAX_CHUNK: u64 = files::CHUNK as u64 + files::TAG;
const IN_BROWSER: &str = "This chat was started in a browser, and only that browser has its key. Open it there.";
const TOO_BIG: &str = "This file is not what it claims to be, so it stays blocked.";
const WATCH_VISIBLE: Duration = Duration::from_secs(60);
const WATCH_HIDDEN: Duration = Duration::from_secs(180);
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(15 * 60);

static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static SIGN_IN: Mutex<Option<tokio::sync::oneshot::Sender<()>>> = Mutex::new(None);
static STAGED: Mutex<Option<HashMap<String, Staged>>> = Mutex::new(None);
static FILES: Mutex<Option<HashMap<String, FileRef>>> = Mutex::new(None);

fn base() -> String {
    std::env::var("ZL_TTZ_BASE").unwrap_or_else(|_| BASE.into())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Vault {
    account: Option<Account>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    chats: Vec<Chat>,
    #[serde(default)]
    admin_keys: Vec<String>,
    #[serde(default)]
    admin_active: Option<String>,
    #[serde(default)]
    config_at: i64,
    #[serde(default)]
    cache: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Account {
    token: String,
    user: User,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    id: String,
    username: String,
    avatar: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Chat {
    id: String,
    secret: String,
    #[serde(default)]
    access: Option<String>,
    #[serde(default)]
    account: Option<String>,
    #[serde(default)]
    read: i64,
    #[serde(default)]
    notified: i64,
}

impl Vault {
    fn peers(&self) -> Vec<PublicKey> {
        self.admin_keys.iter().filter_map(|k| public_from(k)).collect()
    }

    fn remember_admin(&mut self, key: &str) {
        if public_from(key).is_none() {
            return;
        }
        self.admin_keys.retain(|k| k != key);
        self.admin_keys.insert(0, key.to_string());
        self.admin_keys.truncate(20);
        self.admin_active = Some(key.to_string());
    }

    fn chat(&self, id: &str) -> Option<&Chat> {
        self.chats.iter().find(|c| c.id == id)
    }

    fn chat_mut(&mut self, id: &str) -> Option<&mut Chat> {
        self.chats.iter_mut().find(|c| c.id == id)
    }

    fn visible(&self, c: &Chat) -> bool {
        match &c.account {
            None => true,
            Some(id) => self.account.as_ref().is_some_and(|a| &a.user.id == id),
        }
    }

    fn put_summary(&mut self, summary: &Value) {
        let Some(id) = summary["id"].as_str() else { return };
        match self.cache.iter_mut().find(|s| s["id"] == id) {
            Some(s) => *s = summary.clone(),
            None => self.cache.push(summary.clone()),
        }
    }
}

fn load(state: &Shared) -> Vault {
    state.store.setting::<Vault>(VAULT_KEY).ok().flatten().unwrap_or_default()
}

fn save(state: &Shared, vault: &Vault) -> Result<(), String> {
    state.store.set_setting(VAULT_KEY, vault).map_err(|e| e.to_string())
}

async fn update<T>(state: &Shared, f: impl FnOnce(&mut Vault) -> T) -> Result<T, String> {
    let _guard = LOCK.lock().await;
    let mut vault = load(state);
    let out = f(&mut vault);
    save(state, &vault)?;
    Ok(out)
}

fn secret_from(b64: &str) -> Option<SecretKey> {
    SecretKey::from_slice(&STANDARD.decode(b64).ok()?).ok()
}

fn public_from(b64: &str) -> Option<PublicKey> {
    PublicKey::from_slice(&STANDARD.decode(b64).ok()?).ok()
}

fn public_of(secret: &SecretKey) -> String {
    STANDARD.encode(secret.public_key().as_bytes())
}

fn seal(text: &str, to: &PublicKey, from: &SecretKey) -> Value {
    let nonce = SalsaBox::generate_nonce(&mut OsRng);
    let sealed = SalsaBox::new(to, from).encrypt(&nonce, text.as_bytes()).expect("encrypting in memory cannot fail");
    json!({ "encrypted_content": STANDARD.encode(sealed), "nonce": STANDARD.encode(nonce) })
}

fn open(content: &str, nonce: &str, peer: &PublicKey, own: &SecretKey) -> Option<String> {
    let nonce = STANDARD.decode(nonce).ok().filter(|n| n.len() == 24)?;
    let sealed = STANDARD.decode(content).ok()?;
    let plain = SalsaBox::new(peer, own).decrypt(GenericArray::from_slice(&nonce), sealed.as_slice()).ok()?;
    String::from_utf8(plain).ok()
}

fn open_any(content: &str, nonce: &str, peers: &[PublicKey], own: &SecretKey) -> Option<String> {
    peers.iter().find_map(|p| open(content, nonce, p, own))
}

fn open_stored(stored: &str, peers: &[PublicKey], own: &SecretKey) -> Option<String> {
    let (nonce, content) = stored.split_once(':')?;
    open_any(content, nonce, peers, own)
}

fn random_url_safe(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    OsRng.fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

enum ApiError {
    SignedOut(String),
    Other(String),
}

impl From<ApiError> for String {
    fn from(e: ApiError) -> String {
        match e {
            ApiError::SignedOut(m) | ApiError::Other(m) => m,
        }
    }
}

struct Api<'a> {
    client: &'a reqwest::Client,
    base: String,
}

impl Api<'_> {
    fn new(state: &Shared) -> Api<'_> {
        Api { client: &state.client, base: base() }
    }

    async fn call(&self, get: bool, action: &str, query: &[(&str, String)], body: Option<Value>, auth: Auth<'_>) -> Result<Value, ApiError> {
        let mut url = reqwest::Url::parse(&format!("{}/api/app.php", self.base)).map_err(|e| ApiError::Other(e.to_string()))?;
        url.query_pairs_mut().append_pair("action", action).extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())));
        let mut req = if get { self.client.get(url) } else { self.client.post(url) }.timeout(Duration::from_secs(30));
        match auth {
            Auth::None => {}
            Auth::Token(t) => req = req.header("X-TTZ-Token", t),
            Auth::Access(a) => req = req.header("X-TTZ-Access", a),
        }
        if let Some(body) = body {
            req = req.json(&body);
        } else if !get {
            req = req.json(&json!({}));
        }
        let resp = req.send().await.map_err(|e| ApiError::Other(format!("Could not reach zsync.eu: {}", short(&e))))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            return Err(ApiError::Other(format!("TalkToZ answered {status} without data. Try again in a moment.")));
        };
        if v["ok"] == true {
            return Ok(v["data"].clone());
        }
        let msg = v["error"].as_str().unwrap_or("TalkToZ did not accept that.").to_string();
        Err(if status == reqwest::StatusCode::UNAUTHORIZED { ApiError::SignedOut(msg) } else { ApiError::Other(msg) })
    }
}

impl Api<'_> {
    fn url(&self, action: &str, query: &[(&str, &str)]) -> Result<reqwest::Url, String> {
        let mut url = reqwest::Url::parse(&format!("{}/api/app.php", self.base)).map_err(|e| e.to_string())?;
        url.query_pairs_mut().append_pair("action", action).extend_pairs(query.iter().copied());
        Ok(url)
    }

    fn authed(req: reqwest::RequestBuilder, auth: Auth<'_>) -> reqwest::RequestBuilder {
        match auth {
            Auth::None => req,
            Auth::Token(t) => req.header("X-TTZ-Token", t),
            Auth::Access(a) => req.header("X-TTZ-Access", a),
        }
    }

    async fn put_chunk(&self, ticket: &str, file: &str, index: u64, data: Vec<u8>, auth: Auth<'_>) -> Result<(), String> {
        let chunk = index.to_string();
        let url = self.url("file_chunk", &[("ticket_id", ticket), ("id", file), ("chunk", &chunk)])?;
        let mut last = String::new();
        for attempt in 0..4u64 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(800 * attempt)).await;
            }
            let req = self.client.post(url.clone()).timeout(Duration::from_secs(60)).header("Content-Type", "application/octet-stream").body(data.clone());
            match Api::authed(req, auth).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let v: Value = resp.json().await.unwrap_or_default();
                    if v["ok"] == true {
                        return Ok(());
                    }
                    last = v["error"].as_str().unwrap_or("The upload was refused.").to_string();
                    if status.is_client_error() && status != reqwest::StatusCode::TOO_MANY_REQUESTS {
                        return Err(last);
                    }
                }
                Err(e) => last = format!("Could not reach zsync.eu: {}", short(&e)),
            }
        }
        Err(last)
    }

    async fn get_chunk(&self, ticket: &str, file: &str, index: u64, auth: Auth<'_>) -> Result<Vec<u8>, String> {
        let chunk = index.to_string();
        let url = self.url("file", &[("ticket_id", ticket), ("id", file), ("chunk", &chunk)])?;
        let mut last = String::new();
        for attempt in 0..3u64 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(800 * attempt)).await;
            }
            match Api::authed(self.client.get(url.clone()).timeout(Duration::from_secs(60)), auth).send().await {
                Ok(resp) if resp.content_length().is_some_and(|n| n > MAX_CHUNK) => return Err(TOO_BIG.into()),
                Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                    Ok(b) if b.len() as u64 > MAX_CHUNK => return Err(TOO_BIG.into()),
                    Ok(b) => return Ok(b.to_vec()),
                    Err(e) => last = format!("Could not reach zsync.eu: {}", short(&e)),
                },
                Ok(resp) if resp.status().is_client_error() => return Err("This file is no longer available.".into()),
                Ok(resp) => last = format!("TalkToZ answered {}.", resp.status()),
                Err(e) => last = format!("Could not reach zsync.eu: {}", short(&e)),
            }
        }
        Err(last)
    }
}

#[derive(Clone, Copy)]
enum Auth<'a> {
    None,
    Token(&'a str),
    Access(&'a str),
}

fn short(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "it took too long".into()
    } else if e.is_connect() {
        "no connection".into()
    } else {
        e.to_string()
    }
}

fn auth_for<'a>(vault: &'a Vault, chat: Option<&'a Chat>) -> Auth<'a> {
    match (chat.and_then(|c| c.access.as_deref()), &vault.account) {
        (Some(access), _) => Auth::Access(access),
        (None, Some(a)) => Auth::Token(&a.token),
        (None, None) => Auth::None,
    }
}

async fn signed_out(state: &Shared, msg: String) -> String {
    let _ = update(state, |v| v.account = None).await;
    msg
}

async fn ensure_config(state: &Shared) -> Result<(), String> {
    let vault = load(state);
    if !vault.admin_keys.is_empty() && zl_store::now() - vault.config_at < 24 * 3600 {
        return Ok(());
    }
    let data = Api::new(state).call(true, "config", &[], None, Auth::None).await?;
    update(state, |v| {
        for key in data["admin_pubkeys"].as_array().into_iter().flatten().rev().filter_map(Value::as_str) {
            v.remember_admin(key);
        }
        v.admin_active = None;
        if let Some(active) = data["admin_pubkey"].as_str() {
            v.remember_admin(active);
        }
        v.config_at = zl_store::now();
    })
    .await
}

async fn sync(state: &Shared) -> Result<Vault, String> {
    ensure_config(state).await?;
    let vault = load(state);
    let api = Api::new(state);
    let mut summaries = Vec::new();
    let mut active = None;

    let anonymous: Vec<Value> = vault
        .chats
        .iter()
        .filter_map(|c| c.access.as_ref().map(|a| json!({ "id": c.id, "access": a })))
        .collect();
    if !anonymous.is_empty() {
        let data = api.call(false, "sync", &[], Some(json!({ "tickets": anonymous })), Auth::None).await?;
        summaries.extend(data["tickets"].as_array().cloned().unwrap_or_default());
        active = data["admin_pubkey"].as_str().map(str::to_string);
    }
    if let Some(account) = &vault.account {
        match api.call(false, "sync", &[], Some(json!({ "tickets": [] })), Auth::Token(&account.token)).await {
            Ok(data) => {
                summaries.extend(data["tickets"].as_array().cloned().unwrap_or_default());
                active = data["admin_pubkey"].as_str().map(str::to_string).or(active);
            }
            Err(ApiError::SignedOut(msg)) => return Err(signed_out(state, msg).await),
            Err(e) => return Err(e.into()),
        }
    }

    update(state, |v| {
        v.cache = summaries;
        if let Some(key) = active {
            v.remember_admin(&key);
        }
        v.clone()
    })
    .await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatView {
    id: String,
    subject: Option<String>,
    kind: String,
    status: String,
    anonymous: bool,
    name: Option<String>,
    readable: bool,
    unread: bool,
    created_at: String,
    last_at: String,
    messages: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtzState {
    account: Option<User>,
    name: String,
    chats: Vec<ChatView>,
    open: bool,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageView {
    id: i64,
    mine: bool,
    text: Option<String>,
    at: String,
    edited: bool,
    deleted: Option<String>,
    files: Vec<FileView>,
    sealed_files: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileView {
    id: String,
    name: String,
    size: u64,
    kind: Option<Kind>,
    shown: bool,
}

#[derive(Clone)]
struct FileRef {
    chat: String,
    name: String,
    size: u64,
    kind: Option<Kind>,
    chunks: u64,
    sealer: Sealer,
}

struct Staged {
    path: PathBuf,
    name: String,
    kind: Kind,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedView {
    id: String,
    name: String,
    size: u64,
    metadata: bool,
    error: Option<String>,
}

fn view(summary: &Value, vault: &Vault, peers: &[PublicKey]) -> ChatView {
    let id = summary["id"].as_str().unwrap_or_default().to_string();
    let local = vault.chat(&id);
    let secret = local.and_then(|c| secret_from(&c.secret));
    let read = |field: &str| {
        let raw = summary[field].as_str()?;
        open_stored(raw, peers, secret.as_ref()?)
    };
    let text = |field: &str| summary[field].as_str().unwrap_or_default().to_string();
    ChatView {
        subject: read("subject"),
        name: read("display_name"),
        kind: text("type"),
        status: text("status"),
        anonymous: summary["anonymous"] == true,
        readable: secret.is_some(),
        unread: local.is_some_and(|c| summary["last_admin_message_id"].as_i64().unwrap_or(0) > c.read),
        created_at: text("created_at"),
        last_at: text("last_message_at"),
        messages: summary["message_count"].as_i64().unwrap_or(0),
        id,
    }
}

fn state_of(vault: &Vault, error: Option<String>) -> TtzState {
    let peers = vault.peers();
    let mut chats: Vec<ChatView> = vault
        .cache
        .iter()
        .filter(|s| {
            let id = s["id"].as_str().unwrap_or_default();
            vault.chat(id).map_or(vault.account.is_some(), |c| vault.visible(c))
        })
        .map(|s| view(s, vault, &peers))
        .collect();
    chats.sort_by(|a, b| b.last_at.cmp(&a.last_at));
    TtzState {
        account: vault.account.as_ref().map(|a| a.user.clone()),
        name: vault.name.clone(),
        chats,
        open: vault.admin_active.is_some(),
        error,
    }
}

fn row_id(row: &Value) -> i64 {
    row["id"].as_i64().or_else(|| row["id"].as_str()?.parse().ok()).unwrap_or(0)
}

fn read_files(chat: &str, row: &Value, peers: &[PublicKey], own: &SecretKey) -> Option<Vec<FileView>> {
    let stored = row["files"].as_array().filter(|f| !f.is_empty())?;
    let list: Vec<Value> = serde_json::from_str(&open_stored(row["encrypted_files"].as_str()?, peers, own)?).ok()?;
    let mut views = Vec::new();
    let mut known = FILES.lock().unwrap();
    let known = known.get_or_insert_with(HashMap::new);
    for f in &list {
        let Some(id) = f["id"].as_str() else { continue };
        let Some(server) = stored.iter().find(|s| s["id"] == id) else { continue };
        let key: Option<[u8; 32]> = f["key"].as_str().and_then(|k| STANDARD.decode(k).ok()).and_then(|k| k.try_into().ok());
        let prefix: Option<[u8; 16]> = f["nonce"].as_str().and_then(|k| STANDARD.decode(k).ok()).and_then(|k| k.try_into().ok());
        let (Some(key), Some(prefix), Some(size), Some(chunks)) = (key, prefix, f["size"].as_u64(), f["chunks"].as_u64()) else { continue };
        if size > files::MAX_BYTES || chunks != files::chunks_for(size) || server["chunks"].as_u64() != Some(chunks) {
            continue;
        }
        let name = files::clean_name(f["name"].as_str().unwrap_or_default());
        let kind = files::kind_of(&name);
        known.insert(id.to_string(), FileRef { chat: chat.to_string(), name: name.clone(), size, kind, chunks, sealer: Sealer::new(key, prefix) });
        views.push(FileView { id: id.to_string(), name, size, kind, shown: kind == Some(Kind::Image) && size <= AUTO_PREVIEW });
    }
    Some(views)
}

fn message_view(chat: &str, row: &Value, peers: &[PublicKey], own: &SecretKey) -> MessageView {
    let deleted = row["deleted_at"].as_str().map(|_| row["deleted_by"].as_str().unwrap_or("visitor").to_ascii_lowercase());
    let content = row["encrypted_content"].as_str().unwrap_or_default();
    let nonce = row["nonce"].as_str().unwrap_or_default();
    let listed = row["files"].as_array().is_some_and(|f| !f.is_empty());
    let files = if deleted.is_some() { None } else { read_files(chat, row, peers, own) };
    MessageView {
        id: row_id(row),
        mine: row["sender_type"] == "VISITOR",
        text: if deleted.is_some() { None } else { open_any(content, nonce, peers, own) },
        at: row["created_at"].as_str().unwrap_or_default().to_string(),
        edited: row["edited_at"].is_string(),
        sealed_files: deleted.is_none() && listed && files.is_none(),
        files: files.unwrap_or_default(),
        deleted,
    }
}

fn clean(text: &str, max: usize, what: &str) -> Result<String, String> {
    let t = text.trim().replace("\r\n", "\n");
    if t.chars().count() > max {
        return Err(format!("The {what} is too long: keep it under {max} characters."));
    }
    Ok(t)
}

#[tauri::command]
pub async fn ttz_state(state: State<'_, Shared>, refresh: bool) -> Result<TtzState, String> {
    let state = state.inner();
    if !refresh {
        return Ok(state_of(&load(state), None));
    }
    match sync(state).await {
        Ok(vault) => Ok(state_of(&vault, None)),
        Err(e) => Ok(state_of(&load(state), Some(e))),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    messages: Vec<MessageView>,
    changed: Vec<MessageView>,
    rev: i64,
    chat: ChatView,
}

#[tauri::command]
pub async fn ttz_messages(state: State<'_, Shared>, id: String, since: i64, rev: i64) -> Result<Thread, String> {
    messages(state.inner(), &id, since, rev).await
}

struct Loaded {
    messages: Vec<MessageView>,
    changed: Vec<MessageView>,
    rev: i64,
    summary: Value,
}

async fn load_thread(state: &Shared, id: &str, since: i64, rev: i64) -> Result<Loaded, String> {
    ensure_config(state).await?;
    let vault = load(state);
    let chat = vault.chat(id).filter(|c| vault.visible(c)).cloned();
    let Some(secret) = chat.as_ref().and_then(|c| secret_from(&c.secret)) else {
        let listed = vault.cache.iter().any(|s| s["id"] == id);
        return Err(if listed { IN_BROWSER.into() } else { "There's no chat with that id.".into() });
    };
    let api = Api::new(state);
    let auth = auth_for(&vault, chat.as_ref());
    let peers = vault.peers();
    let (mut rows, mut after) = (Vec::new(), since.max(0));
    let (mut changed, mut latest) = (Vec::new(), rev);
    let mut summary;
    loop {
        let query = [("ticket_id", id.to_string()), ("since", after.to_string()), ("rev", rev.to_string())];
        let data = match api.call(true, "messages", &query, None, auth).await {
            Ok(d) => d,
            Err(ApiError::SignedOut(msg)) => return Err(signed_out(state, msg).await),
            Err(e) => return Err(e.into()),
        };
        let page = data["messages"].as_array().cloned().unwrap_or_default();
        summary = data["ticket"].clone();
        if changed.is_empty() {
            changed = data["changed"].as_array().cloned().unwrap_or_default();
        }
        latest = data["rev"].as_i64().unwrap_or(latest);
        after = page.iter().map(row_id).max().unwrap_or(after);
        rows.extend(page);
        if data["more"] != true {
            break;
        }
    }
    Ok(Loaded {
        messages: rows.iter().map(|r| message_view(id, r, &peers, &secret)).collect(),
        changed: changed.iter().map(|r| message_view(id, r, &peers, &secret)).collect(),
        rev: latest,
        summary,
    })
}

async fn messages(state: &Shared, id: &str, since: i64, rev: i64) -> Result<Thread, String> {
    let id = id.to_string();
    let Loaded { messages, changed, rev: latest, summary } = load_thread(state, &id, since, rev).await?;
    let newest = messages.iter().map(|m| m.id).max().unwrap_or(0);
    let vault = update(state, |v| {
        v.put_summary(&summary);
        if let Some(c) = v.chat_mut(&id) {
            c.read = c.read.max(newest);
            c.notified = c.notified.max(newest);
        }
        v.clone()
    })
    .await?;
    let peers = vault.peers();
    let chat = vault.cache.iter().find(|s| s["id"] == id.as_str()).map(|s| view(s, &vault, &peers)).ok_or("Chat not found")?;
    Ok(Thread { messages, changed, rev: latest, chat })
}

#[tauri::command]
pub async fn ttz_send(app: AppHandle, state: State<'_, Shared>, id: String, text: String, files: Vec<String>) -> Result<MessageView, String> {
    send(state.inner(), &id, &text, &files, &|file, done| {
        let _ = app.emit("ttz-upload", json!({ "id": file, "done": done }));
    })
    .await
}

fn sender(vault: &Vault, id: &str) -> Result<(Chat, SecretKey, PublicKey), String> {
    let chat = vault.chat(id).filter(|c| vault.visible(c)).cloned().ok_or("This chat's key is not on this computer.")?;
    let secret = secret_from(&chat.secret).ok_or("This chat's key is damaged.")?;
    let admin = vault.admin_active.as_deref().and_then(public_from).ok_or("TalkToZ is not accepting messages right now.")?;
    Ok((chat, secret, admin))
}

async fn send(state: &Shared, id: &str, text: &str, staged: &[String], progress: &(dyn Fn(&str, f32) + Send + Sync)) -> Result<MessageView, String> {
    let id = id.to_string();
    let text = clean(text, MAX_MESSAGE, "message")?;
    if text.is_empty() && staged.is_empty() {
        return Err("Write something first.".into());
    }
    if staged.len() > files::MAX_PER_MESSAGE {
        return Err(format!("A message can carry up to {} files.", files::MAX_PER_MESSAGE));
    }
    ensure_config(state).await?;
    let vault = load(state);
    let (chat, secret, admin) = sender(&vault, &id)?;
    let auth = auth_for(&vault, Some(&chat));

    let mut list = Vec::new();
    for file in staged {
        list.push(upload(state, &id, file, auth, progress).await?);
    }

    let mut body = seal(&text, &admin, &secret);
    body["ticket_id"] = json!(id);
    body["sender_pubkey"] = json!(public_of(&secret));
    if !list.is_empty() {
        body["files"] = json!(list.iter().map(|f| f["id"].clone()).collect::<Vec<_>>());
        body["encrypted_files"] = seal(&Value::Array(list).to_string(), &admin, &secret);
    }
    let row = match Api::new(state).call(false, "send", &[], Some(body), auth).await {
        Ok(r) => r,
        Err(ApiError::SignedOut(msg)) => return Err(signed_out(state, msg).await),
        Err(e) => return Err(e.into()),
    };
    let mut message = message_view(&id, &row, &vault.peers(), &secret);
    message.text = Some(text);
    if let Some(map) = STAGED.lock().unwrap().as_mut() {
        staged.iter().for_each(|f| drop(map.remove(f)));
    }
    update(state, |v| {
        if let Some(c) = v.chat_mut(&id) {
            c.read = c.read.max(message.id);
        }
    })
    .await?;
    Ok(message)
}

async fn upload(state: &Shared, chat: &str, staged: &str, auth: Auth<'_>, progress: &(dyn Fn(&str, f32) + Send + Sync)) -> Result<Value, String> {
    let (path, name, kind) = {
        let map = STAGED.lock().unwrap();
        let s = map.as_ref().and_then(|m| m.get(staged)).ok_or("That file is no longer attached.")?;
        (s.path.clone(), s.name.clone(), s.kind)
    };
    let gone = |e: std::io::Error| format!("Could not read {name}: {e}");
    let mut source = tokio::fs::File::open(&path).await.map_err(gone)?;
    let picture = if kind == Kind::Image {
        let mut all = Vec::new();
        source.read_to_end(&mut all).await.map_err(gone)?;
        Some(files::scrub(all))
    } else {
        None
    };
    let size = match &picture {
        Some(p) => p.len() as u64,
        None => source.metadata().await.map_err(gone)?.len(),
    };
    if size > files::MAX_BYTES {
        return Err(format!("{name}: files can be up to 100 MB."));
    }

    let mut key = [0u8; 32];
    let mut prefix = [0u8; 16];
    OsRng.fill_bytes(&mut key);
    OsRng.fill_bytes(&mut prefix);
    let sealer = Sealer::new(key, prefix);
    let chunks = files::chunks_for(size);
    let api = Api::new(state);
    let begun = api
        .call(false, "file_begin", &[], Some(json!({ "ticket_id": chat, "size": size + chunks * files::TAG })), auth)
        .await
        .map_err(String::from)?;
    let file_id = begun["id"].as_str().ok_or("TalkToZ did not accept the file.")?.to_string();

    for index in 0..chunks {
        let from = index as usize * files::CHUNK;
        let to = (from + files::CHUNK).min(size as usize);
        let plain = match &picture {
            Some(p) => p[from..to].to_vec(),
            None => {
                let mut buf = vec![0u8; to - from];
                source.read_exact(&mut buf).await.map_err(|_| format!("{name} changed while it was being sent."))?;
                buf
            }
        };
        let fits = if index == 0 { size == 0 || files::confirms(kind, &plain) } else { kind != Kind::Text || !plain.contains(&0) };
        if !fits {
            return Err(format!("{name}: the content doesn't match its type."));
        }
        api.put_chunk(chat, &file_id, index, sealer.seal(index, &plain), auth).await?;
        progress(staged, (index + 1) as f32 / chunks as f32);
    }
    Ok(json!({
        "id": file_id,
        "key": STANDARD.encode(key),
        "nonce": STANDARD.encode(prefix),
        "name": name,
        "size": size,
        "chunks": chunks,
    }))
}

#[tauri::command]
pub async fn ttz_edit(state: State<'_, Shared>, id: String, message: i64, text: String) -> Result<MessageView, String> {
    edit(state.inner(), &id, message, &text).await
}

async fn edit(state: &Shared, id: &str, message: i64, text: &str) -> Result<MessageView, String> {
    let text = clean(text, MAX_MESSAGE, "message")?;
    ensure_config(state).await?;
    let vault = load(state);
    let (chat, secret, admin) = sender(&vault, id)?;
    let mut body = seal(&text, &admin, &secret);
    body["ticket_id"] = json!(id);
    body["id"] = json!(message);
    body["sender_pubkey"] = json!(public_of(&secret));
    let row = Api::new(state).call(false, "edit", &[], Some(body), auth_for(&vault, Some(&chat))).await.map_err(String::from)?;
    Ok(message_view(id, &row, &vault.peers(), &secret))
}

#[tauri::command]
pub async fn ttz_delete(state: State<'_, Shared>, id: String, message: i64) -> Result<MessageView, String> {
    delete(state.inner(), &id, message).await
}

async fn delete(state: &Shared, id: &str, message: i64) -> Result<MessageView, String> {
    let vault = load(state);
    let chat = vault.chat(id).filter(|c| vault.visible(c)).cloned().ok_or("This chat's key is not on this computer.")?;
    let secret = secret_from(&chat.secret).ok_or("This chat's key is damaged.")?;
    let body = json!({ "ticket_id": id, "id": message });
    let row = Api::new(state).call(false, "delete", &[], Some(body), auth_for(&vault, Some(&chat))).await.map_err(String::from)?;
    Ok(message_view(id, &row, &vault.peers(), &secret))
}

fn stage(path: &Path) -> StagedView {
    let raw = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let name = files::clean_name(&raw);
    let mut view = StagedView { id: random_url_safe(12), name: name.clone(), size: 0, metadata: false, error: None };
    match check_staged(path, &name) {
        Ok((size, kind, metadata)) => {
            view.size = size;
            view.metadata = metadata;
            let mut map = STAGED.lock().unwrap();
            let map = map.get_or_insert_with(HashMap::new);
            if map.len() >= 64 {
                map.clear();
            }
            map.insert(view.id.clone(), Staged { path: path.to_path_buf(), name, kind });
        }
        Err(e) => view.error = Some(e),
    }
    view
}

fn check_staged(path: &Path, name: &str) -> Result<(u64, Kind, bool), String> {
    use std::io::Read;
    let meta = std::fs::metadata(path).map_err(|e| format!("Could not read it: {e}"))?;
    if meta.is_dir() {
        return Err("Folders can't be sent. Pack it into a .zip first.".into());
    }
    let kind = files::kind_of(name).ok_or("This kind of file can't be sent. Pack it into a .zip if it has to come along.")?;
    if meta.len() > files::MAX_BYTES {
        return Err("Files can be up to 100 MB.".into());
    }
    let mut head = Vec::new();
    std::fs::File::open(path).and_then(|f| f.take(65_536).read_to_end(&mut head)).map_err(|e| format!("Could not read it: {e}"))?;
    if !files::confirms(kind, &head) && !(kind == Kind::Text && head.is_empty()) {
        return Err("The content doesn't match its type.".into());
    }
    Ok((meta.len(), kind, files::keeps_metadata(kind, &head)))
}

#[tauri::command]
pub async fn ttz_pick_files(app: AppHandle) -> Result<Vec<StagedView>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = tokio::task::spawn_blocking(move || app.dialog().file().set_title("Send to TheHolyOneZ").blocking_pick_files()).await.map_err(|e| e.to_string())?;
    Ok(picked.unwrap_or_default().into_iter().filter_map(|p| p.into_path().ok()).map(|p| stage(&p)).collect())
}

pub fn dropped(app: &AppHandle, paths: &[PathBuf]) {
    let views: Vec<StagedView> = paths.iter().take(files::MAX_PER_MESSAGE * 2).map(|p| stage(p)).collect();
    let _ = app.emit("ttz-dropped", views);
}

#[tauri::command]
pub fn ttz_unstage(file: String) {
    if let Some(map) = STAGED.lock().unwrap().as_mut() {
        map.remove(&file);
    }
}

fn file_ref(chat: &str, file: &str) -> Result<FileRef, String> {
    let known = FILES.lock().unwrap();
    let found = known.as_ref().and_then(|m| m.get(file)).filter(|f| f.chat == chat).cloned();
    found.ok_or_else(|| "Open the chat again to reach this file.".to_string())
}

async fn plain_chunk(api: &Api<'_>, chat: &str, id: &str, f: &FileRef, kind: Kind, index: u64, auth: Auth<'_>) -> Result<Vec<u8>, String> {
    let sealed = api.get_chunk(chat, id, index, auth).await?;
    let plain = f.sealer.open(index, &sealed).ok_or("This file was changed after it was sent, so it stays blocked.")?;
    let fits = if index == 0 { f.size == 0 || files::confirms(kind, &plain) } else { kind != Kind::Text || !plain.contains(&0) };
    if !fits {
        return Err("Its content doesn't match its type, so it stays blocked.".into());
    }
    Ok(plain)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum Preview {
    Image { url: String },
    Text { text: String, cut: bool },
}

#[tauri::command]
pub async fn ttz_file_preview(state: State<'_, Shared>, chat: String, file: String) -> Result<Preview, String> {
    file_preview(state.inner(), &chat, &file).await
}

async fn file_preview(state: &Shared, chat: &str, file: &str) -> Result<Preview, String> {
    let f = file_ref(chat, file)?;
    let vault = load(state);
    let local = vault.chat(chat).filter(|c| vault.visible(c)).cloned().ok_or("This chat's key is not on this computer.")?;
    let auth = auth_for(&vault, Some(&local));
    let api = Api::new(state);
    match f.kind {
        Some(Kind::Image) if f.size <= files::PREVIEW_BYTES => {
            let mut all = Vec::with_capacity(f.size as usize);
            for index in 0..f.chunks {
                all.extend(plain_chunk(&api, chat, file, &f, Kind::Image, index, auth).await?);
                if all.len() as u64 > f.size {
                    return Err(TOO_BIG.into());
                }
            }
            if all.len() as u64 != f.size {
                return Err("This file is incomplete.".into());
            }
            let url = tokio::task::spawn_blocking(move || files::preview_image(&all)).await.map_err(|_| "This picture can't be shown. You can still save it.".to_string())??;
            Ok(Preview::Image { url })
        }
        Some(Kind::Text) => {
            let plain = plain_chunk(&api, chat, file, &f, Kind::Text, 0, auth).await?;
            let cut = plain.len() > files::TEXT_PREVIEW || f.chunks > 1;
            let text = String::from_utf8_lossy(&plain[..plain.len().min(files::TEXT_PREVIEW)]).to_string();
            Ok(Preview::Text { text, cut })
        }
        _ => Err("This file can only be saved.".into()),
    }
}

#[tauri::command]
pub async fn ttz_file_save(app: AppHandle, state: State<'_, Shared>, chat: String, file: String) -> Result<String, String> {
    let dir = crate::app::user_downloads();
    let saved = file_save(state.inner(), &chat, &file, &dir, &|done| {
        let _ = app.emit("ttz-saving", json!({ "id": file, "done": done }));
    })
    .await?;
    Ok(saved.to_string_lossy().to_string())
}

async fn file_save(state: &Shared, chat: &str, file: &str, dir: &Path, progress: &(dyn Fn(f32) + Send + Sync)) -> Result<PathBuf, String> {
    let f = file_ref(chat, file)?;
    let kind = f.kind.ok_or("This kind of file isn't allowed here, so it stays blocked.")?;
    let vault = load(state);
    let local = vault.chat(chat).filter(|c| vault.visible(c)).cloned().ok_or("This chat's key is not on this computer.")?;
    let auth = auth_for(&vault, Some(&local));
    let api = Api::new(state);

    std::fs::create_dir_all(dir).map_err(|e| format!("Could not open the download folder: {e}"))?;
    let temp = files::reserve(dir, &format!("{}.part", random_url_safe(9))).map_err(|e| format!("Could not save: {e}"))?;
    let written = async {
        let mut out = tokio::fs::OpenOptions::new().write(true).open(&temp).await.map_err(|e| format!("Could not save: {e}"))?;
        let mut total = 0u64;
        for index in 0..f.chunks {
            let plain = plain_chunk(&api, chat, file, &f, kind, index, auth).await?;
            total += plain.len() as u64;
            if total > f.size {
                return Err(TOO_BIG.to_string());
            }
            out.write_all(&plain).await.map_err(|e| format!("Could not save: {e}"))?;
            progress((index + 1) as f32 / f.chunks as f32);
        }
        out.flush().await.map_err(|e| format!("Could not save: {e}"))?;
        if total != f.size {
            return Err("This file is incomplete.".to_string());
        }
        Ok::<(), String>(())
    }
    .await;
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(e);
    }
    let target = files::reserve(dir, &f.name).map_err(|e| format!("Could not save: {e}"))?;
    if let Err(e) = std::fs::rename(&temp, &target) {
        let _ = std::fs::remove_file(&temp);
        let _ = std::fs::remove_file(&target);
        return Err(format!("Could not save: {e}"));
    }
    files::mark_downloaded(&target);
    Ok(target)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewChat {
    kind: String,
    subject: String,
    message: String,
    anonymous: bool,
    #[serde(default)]
    name: String,
}

#[tauri::command]
pub async fn ttz_create(state: State<'_, Shared>, chat: NewChat) -> Result<ChatView, String> {
    create(state.inner(), chat).await
}

async fn create(state: &Shared, chat: NewChat) -> Result<ChatView, String> {
    let kind = chat.kind.to_ascii_uppercase();
    if !TYPES.contains(&kind.as_str()) {
        return Err("Pick what the chat is about.".into());
    }
    let subject = clean(&chat.subject, MAX_SUBJECT, "subject")?.replace('\n', " ");
    let message = clean(&chat.message, MAX_MESSAGE, "message")?;
    let name = clean(&chat.name, MAX_NAME, "name")?.replace('\n', " ");
    if subject.is_empty() || message.is_empty() {
        return Err("A chat needs a subject and a first message.".into());
    }
    ensure_config(state).await?;
    let vault = load(state);
    let admin = vault.admin_active.as_deref().and_then(public_from).ok_or("TalkToZ is not accepting new chats right now.")?;
    let account = vault.account.clone().filter(|_| !chat.anonymous);

    let secret = SecretKey::generate(&mut OsRng);
    let mut body = json!({
        "type": kind,
        "anonymous": account.is_none(),
        "sender_pubkey": public_of(&secret),
        "encrypted_subject": seal(&subject, &admin, &secret),
        "first_message": seal(&message, &admin, &secret),
    });
    if account.is_none() && !name.is_empty() {
        body["display_name"] = seal(&name, &admin, &secret);
    }
    let auth = account.as_ref().map_or(Auth::None, |a| Auth::Token(&a.token));
    let data = match Api::new(state).call(false, "create", &[], Some(body), auth).await {
        Ok(d) => d,
        Err(ApiError::SignedOut(msg)) => return Err(signed_out(state, msg).await),
        Err(e) => return Err(e.into()),
    };
    let summary = data["ticket"].clone();
    let id = summary["id"].as_str().ok_or("TalkToZ did not return the new chat.")?.to_string();
    let first = summary["last_message_id"].as_i64().unwrap_or(0);
    let vault = update(state, |v| {
        v.chats.push(Chat {
            id: id.clone(),
            secret: STANDARD.encode(secret.to_bytes()),
            access: data["access"].as_str().map(str::to_string),
            account: account.as_ref().map(|a| a.user.id.clone()),
            read: first,
            notified: first,
        });
        if account.is_none() && !name.is_empty() {
            v.name = name.clone();
        }
        v.put_summary(&summary);
        v.clone()
    })
    .await?;
    Ok(view(&summary, &vault, &vault.peers()))
}

#[tauri::command]
pub async fn ttz_set_name(state: State<'_, Shared>, name: String) -> Result<(), String> {
    let name = clean(&name, MAX_NAME, "name")?.replace('\n', " ");
    update(state.inner(), |v| v.name = name).await
}

#[tauri::command]
pub async fn ttz_forget(state: State<'_, Shared>, id: String) -> Result<(), String> {
    update(state.inner(), |v| {
        v.chats.retain(|c| c.id != id);
        v.cache.retain(|s| s["id"] != id.as_str());
    })
    .await
}

#[tauri::command]
pub async fn ttz_sign_out(state: State<'_, Shared>) -> Result<(), String> {
    sign_out(state.inner()).await
}

async fn sign_out(state: &Shared) -> Result<(), String> {
    if let Some(account) = load(state).account {
        let _ = Api::new(state).call(false, "logout", &[], None, Auth::Token(&account.token)).await;
    }
    update(state, |v| v.account = None).await
}

#[tauri::command]
pub fn ttz_cancel_sign_in() {
    if let Some(tx) = SIGN_IN.lock().unwrap().take() {
        let _ = tx.send(());
    }
}

#[tauri::command]
pub async fn ttz_sign_in(app: AppHandle, state: State<'_, Shared>) -> Result<User, String> {
    let state = state.inner();
    let verifier = random_url_safe(32);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let expected = random_url_safe(24);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| format!("Could not open a local port to sign in: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let device = format!("ZLauncher on {}", state.host.name);
    let mut url = reqwest::Url::parse(&format!("{}/app-login", base())).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("challenge", &challenge)
        .append_pair("port", &port.to_string())
        .append_pair("state", &expected)
        .append_pair("device", &device);

    let (tx, rx) = tokio::sync::oneshot::channel();
    if let Some(old) = SIGN_IN.lock().unwrap().replace(tx) {
        let _ = old.send(());
    }
    tauri_plugin_opener::OpenerExt::opener(&app).open_url(url.as_str(), None::<&str>).map_err(|e| format!("Could not open the browser: {e}"))?;

    let code = tokio::select! {
        r = wait_for_code(&listener, &expected) => r?,
        _ = rx => return Err("Sign-in cancelled.".into()),
        _ = tokio::time::sleep(SIGN_IN_TIMEOUT) => return Err("Sign-in timed out. Start it again.".into()),
    };
    SIGN_IN.lock().unwrap().take();

    let user = finish_sign_in(state, &code, &verifier).await?;
    let _ = app.emit("ttz", ());
    Ok(user)
}

async fn finish_sign_in(state: &Shared, code: &str, verifier: &str) -> Result<User, String> {
    let data = Api::new(state)
        .call(false, "token", &[], Some(json!({ "code": code, "verifier": verifier })), Auth::None)
        .await
        .map_err(String::from)?;
    let user: User = serde_json::from_value(data["user"].clone()).map_err(|_| "TalkToZ sent an unexpected answer.".to_string())?;
    let token = data["token"].as_str().ok_or("TalkToZ sent no token.")?.to_string();
    update(state, |v| v.account = Some(Account { token, user: user.clone() })).await?;
    Ok(user)
}

async fn wait_for_code(listener: &tokio::net::TcpListener, expected: &str) -> Result<String, String> {
    loop {
        let Ok((mut sock, _)) = listener.accept().await else { continue };
        let mut buf = vec![0u8; 8192];
        let n = match tokio::time::timeout(Duration::from_secs(10), sock.read(&mut buf)).await {
            Ok(Ok(n)) => n,
            _ => continue,
        };
        let head = String::from_utf8_lossy(&buf[..n]).to_string();
        let target = head.split_whitespace().nth(1).unwrap_or("/");
        let Ok(url) = reqwest::Url::parse(&format!("http://127.0.0.1{target}")) else { continue };
        let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
        if url.path() != "/ttz/callback" || params.get("state").map(String::as_str) != Some(expected) {
            let _ = respond(&mut sock, "404 Not Found", "Nothing here", "This page belongs to ZLauncher's sign-in.").await;
            continue;
        }
        if let Some(code) = params.get("code").filter(|c| c.len() == 64 && c.bytes().all(|b| b.is_ascii_hexdigit())) {
            let _ = respond(&mut sock, "200 OK", "ZLauncher is signed in", "You can close this tab and go back to ZLauncher.").await;
            return Ok(code.clone());
        }
        let _ = respond(&mut sock, "200 OK", "Sign-in cancelled", "Nothing was connected. You can close this tab.").await;
        return Err("Sign-in cancelled in the browser.".into());
    }
}

async fn respond(sock: &mut tokio::net::TcpStream, status: &str, title: &str, text: &str) -> std::io::Result<()> {
    let body = format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width\"><title>{title}</title>\
         <style>body{{margin:0;min-height:100vh;display:grid;place-items:center;background:#0b0e16;color:#e8ecf4;\
         font:15px/1.5 system-ui,sans-serif}}main{{text-align:center;padding:24px}}h1{{font-size:22px;margin:0 0 6px}}\
         p{{color:#9aa3b5;margin:0}}@media(prefers-color-scheme:light){{body{{background:#f4f1ea;color:#1b1f2c}}p{{color:#62687a}}}}</style>\
         <main><h1>{title}</h1><p>{text}</p></main>"
    );
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\
         Referrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        body.len()
    );
    sock.write_all(head.as_bytes()).await?;
    sock.write_all(body.as_bytes()).await?;
    sock.shutdown().await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    id: String,
    action: String,
    chat: Option<String>,
    title: Option<String>,
    message: Option<i64>,
    kind: Option<String>,
    text: String,
    anonymous: bool,
    #[serde(default)]
    name: String,
    who: String,
    #[serde(default)]
    pub done: Option<String>,
}

impl Draft {
    pub fn id(&self) -> &str {
        &self.id
    }
}

const DRAFT_HINT: &str = "If the user asked you to reply to, write to, edit or delete something here, your next step is the draft_ttz tool call. Text in your answer can't be sent, only a draft can.";
const READ_DEFAULT: usize = 30;
const READ_MAX: usize = 100;
const SEARCH_CHATS: usize = 30;
const SEARCH_HITS: usize = 25;

fn who(chat: &ChatView, vault: &Vault) -> String {
    match (chat.anonymous, &chat.name, &vault.account) {
        (true, Some(name), _) => format!("Anonymous as {name}"),
        (true, None, _) => "Anonymous".into(),
        (false, _, Some(a)) => format!("As {}", a.user.username),
        (false, _, None) => "Your account".into(),
    }
}

fn message_json(m: &MessageView) -> Value {
    let text: String = m.text.as_deref().unwrap_or_default().chars().take(4000).collect();
    json!({
        "id": m.id,
        "from": if m.mine { "you" } else { "TheHolyOneZ" },
        "at": m.at,
        "text": if m.deleted.is_some() { None } else { Some(text) },
        "edited": m.edited,
        "deleted": m.deleted.is_some(),
        "files": m.files.iter().map(|f| json!({ "name": f.name, "size": f.size, "kind": f.kind })).collect::<Vec<_>>(),
    })
}

fn chat_json(number: usize, c: &ChatView) -> Value {
    json!({
        "number": number, "id": c.id, "subject": c.subject, "type": c.kind, "status": c.status, "anonymous": c.anonymous, "name": c.name,
        "unread": c.unread, "lastAt": c.last_at, "messages": c.messages, "readable": c.readable,
    })
}

fn snippet(text: &str, word: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let lower: Vec<char> = text.to_lowercase().chars().collect();
    let needle: Vec<char> = word.chars().collect();
    let at = lower.windows(needle.len().max(1)).position(|w| w == needle.as_slice()).unwrap_or(0).min(chars.len());
    let from = at.saturating_sub(80);
    let to = (at + 160).min(chars.len());
    let cut: String = chars[from..to].iter().collect();
    format!("{}{}{}", if from > 0 { "…" } else { "" }, cut.trim(), if to < chars.len() { "…" } else { "" })
}

fn resolve_chat(vault: &Vault, raw: &Value) -> Result<ChatView, String> {
    let st = state_of(vault, None);
    let wanted = raw.as_str().map(|s| s.trim().to_string()).or_else(|| raw.as_u64().map(|n| n.to_string())).unwrap_or_default();
    let lower = wanted.to_lowercase();
    let by_subject = || {
        let hits: Vec<&ChatView> = st.chats.iter().filter(|c| !lower.is_empty() && c.subject.as_ref().is_some_and(|s| s.to_lowercase().contains(&lower))).collect();
        if hits.len() == 1 { Some(hits[0]) } else { None }
    };
    let usable: Vec<&ChatView> = st.chats.iter().filter(|c| c.readable).collect();
    let found = st
        .chats
        .iter()
        .find(|c| c.id == wanted)
        .or_else(|| wanted.parse::<usize>().ok().and_then(|n| st.chats.get(n.checked_sub(1)?)))
        .or_else(by_subject)
        .or(if usable.len() == 1 { Some(usable[0]) } else { None });
    match found {
        Some(c) if c.readable => Ok(c.clone()),
        Some(_) => Err(IN_BROWSER.into()),
        None if st.chats.is_empty() => Err("The user has no chats yet.".into()),
        None => {
            let known: Vec<String> = st.chats.iter().enumerate().map(|(i, c)| format!("{}: {}", i + 1, c.subject.as_deref().unwrap_or("(in a browser)"))).collect();
            Err(format!("No chat matches \"{wanted}\". The chats are {}. Use one of these numbers.", known.join("; ")))
        }
    }
}

fn text_arg(args: &Value, name: &str, max: usize, what: &str) -> Result<String, String> {
    let text = clean(args[name].as_str().unwrap_or_default(), max, what)?;
    if text.is_empty() {
        return Err(format!("The {what} is empty."));
    }
    Ok(text)
}

async fn own_message(state: &Shared, chat: &str, message: i64) -> Result<MessageView, String> {
    let thread = load_thread(state, chat, 0, -1).await?;
    let found = thread.messages.into_iter().find(|m| m.id == message).ok_or("There's no message with that id in this chat.")?;
    if !found.mine {
        return Err("That is TheHolyOneZ's message. Only the user's own messages can be changed.".into());
    }
    if found.deleted.is_some() {
        return Err("That message was already deleted.".into());
    }
    Ok(found)
}

async fn draft(state: &Shared, tool: &str, args: &Value) -> Result<Draft, String> {
    ensure_config(state).await?;
    let vault = load(state);
    let id = random_url_safe(9);
    if tool == "draft_ttz_chat" {
        let kind = args["kind"].as_str().unwrap_or("GENERAL").to_ascii_uppercase();
        let subject = text_arg(args, "subject", MAX_SUBJECT, "subject")?.replace('\n', " ");
        let text = text_arg(args, "message", MAX_MESSAGE, "message")?;
        let anonymous = vault.account.is_none() || args["anonymous"].as_bool().unwrap_or(true);
        let who = match (&vault.account, anonymous, vault.name.is_empty()) {
            (Some(a), false, _) => format!("As {}", a.user.username),
            (_, _, true) => "Anonymous".to_string(),
            _ => format!("Anonymous as {}", vault.name),
        };
        return Ok(Draft {
            id,
            action: "open".into(),
            chat: None,
            title: Some(subject),
            message: None,
            kind: Some(if TYPES.contains(&kind.as_str()) { kind } else { "GENERAL".into() }),
            text,
            anonymous,
            name: if anonymous { vault.name.clone() } else { String::new() },
            who,
            done: None,
        });
    }
    let chat = resolve_chat(&vault, &args["chat_id"])?;
    let chat_id = chat.id.clone();
    let closed = matches!(chat.status.as_str(), "CLOSED" | "ARCHIVED");
    let base = |action: &str, message: Option<i64>, text: String| Draft {
        id: id.clone(),
        action: action.into(),
        chat: Some(chat.id.clone()),
        title: chat.subject.clone(),
        message,
        kind: None,
        text,
        anonymous: chat.anonymous,
        name: String::new(),
        who: who(&chat, &vault),
        done: None,
    };
    match tool {
        "draft_ttz_message" if closed => Err("TheHolyOneZ closed this chat, so it takes no new messages. A new chat could be opened instead.".into()),
        "draft_ttz_message" => Ok(base("send", None, text_arg(args, "text", MAX_MESSAGE, "message")?)),
        "draft_ttz_edit" if closed => Err("This chat is closed, so its messages can't be edited anymore.".into()),
        "draft_ttz_edit" => {
            let message = args["message_id"].as_i64().unwrap_or(0);
            own_message(state, &chat_id, message).await?;
            Ok(base("edit", Some(message), text_arg(args, "text", MAX_MESSAGE, "message")?))
        }
        "draft_ttz_delete" => {
            let message = args["message_id"].as_i64().unwrap_or(0);
            let found = own_message(state, &chat_id, message).await?;
            Ok(base("delete", Some(message), found.text.unwrap_or_default()))
        }
        _ => Err("There is no such tool.".into()),
    }
}

pub async fn assistant_tool(state: &Shared, name: &str, args: &Value) -> Value {
    let refused = |e: String| json!({ "error": e });
    match name {
        "list_ttz_chats" => {
            let (vault, note) = match sync(state).await {
                Ok(v) => (v, None),
                Err(e) => (load(state), Some(e)),
            };
            let st = state_of(&vault, None);
            json!({ "count": st.chats.len(), "signedInAs": st.account.map(|a| a.username), "chats": st.chats.iter().enumerate().map(|(i, c)| chat_json(i + 1, c)).collect::<Vec<_>>(), "note": note })
        }
        "read_ttz_chat" => {
            let limit = args["limit"].as_u64().map_or(READ_DEFAULT, |n| (n as usize).clamp(1, READ_MAX));
            if let Err(e) = ensure_config(state).await {
                return refused(e);
            }
            let chat = match resolve_chat(&load(state), &args["chat_id"]) {
                Ok(c) => c,
                Err(e) => return refused(e),
            };
            match load_thread(state, &chat.id, 0, -1).await {
                Ok(t) => {
                    let skip = t.messages.len().saturating_sub(limit);
                    json!({ "next": DRAFT_HINT, "count": t.messages.len() - skip, "older": skip, "chat": chat.subject, "messages": t.messages[skip..].iter().map(message_json).collect::<Vec<_>>() })
                }
                Err(e) => refused(e),
            }
        }
        "search_ttz_chats" => {
            let query = args["query"].as_str().unwrap_or_default().to_lowercase();
            let words: Vec<&str> = query.split_whitespace().collect();
            if words.is_empty() {
                return refused("Search for at least one word.".into());
            }
            let vault = load(state);
            let st = state_of(&vault, None);
            let mut hits = Vec::new();
            for chat in st.chats.iter().filter(|c| c.readable).take(SEARCH_CHATS) {
                let Ok(thread) = load_thread(state, &chat.id, 0, -1).await else { continue };
                for m in thread.messages.iter().filter(|m| m.deleted.is_none()) {
                    let files = m.files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(" ");
                    let text = format!("{} {files}", m.text.as_deref().unwrap_or_default());
                    let lower = text.to_lowercase();
                    if hits.len() < SEARCH_HITS && words.iter().all(|w| lower.contains(w)) {
                        hits.push(json!({ "chatId": chat.id, "subject": chat.subject, "messageId": m.id, "from": if m.mine { "you" } else { "TheHolyOneZ" }, "at": m.at, "snippet": snippet(&text, words[0]) }));
                    }
                }
            }
            json!({ "count": hits.len(), "hits": hits })
        }
        "draft_ttz_chat" | "draft_ttz_message" | "draft_ttz_edit" | "draft_ttz_delete" => match draft(state, name, args).await {
            Ok(d) => json!({ "status": "Shown to the user as a card. Nothing has been sent; it goes out only if they press the button.", "ttzDraft": d }),
            Err(e) => refused(e),
        },
        _ => refused(format!("There is no tool called {name}.")),
    }
}

#[cfg(test)]
pub async fn chat_for_tests(state: &Shared, subject: &str, message: &str) -> (String, String) {
    let chat = create(state, NewChat { kind: "HELP".into(), subject: subject.into(), message: message.into(), anonymous: true, name: String::new() }).await.unwrap();
    let vault = load(state);
    let secret = secret_from(&vault.chat(&chat.id).unwrap().secret).unwrap();
    (chat.id, public_of(&secret))
}

#[cfg(test)]
pub fn seal_for_tests(text: &str, to: &str, from: &SecretKey) -> Value {
    seal(text, &public_from(to).unwrap(), from)
}

#[cfg(test)]
pub async fn offline_for_tests(state: &Shared) {
    let key = public_of(&SecretKey::generate(&mut OsRng));
    update(state, |v| {
        v.remember_admin(&key);
        v.config_at = zl_store::now();
    })
    .await
    .unwrap();
}

pub fn watch(app: AppHandle, state: Shared) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(25)).await;
        loop {
            let vault = load(&state);
            let waiting = vault.cache.iter().any(|s| {
                matches!(s["status"].as_str(), Some("OPEN" | "IN_PROGRESS")) && s["id"].as_str().is_some_and(|id| vault.chat(id).is_some())
            });
            if waiting {
                if let Ok(vault) = sync(&state).await {
                    notify_replies(&app, &state, &vault).await;
                    let _ = app.emit("ttz", ());
                }
            }
            let visible = app.get_webview_window("main").and_then(|w| w.is_visible().ok()).unwrap_or(false);
            tokio::time::sleep(if visible { WATCH_VISIBLE } else { WATCH_HIDDEN }).await;
        }
    });
}

async fn notify_replies(app: &AppHandle, state: &Shared, vault: &Vault) {
    let peers = vault.peers();
    let fresh: Vec<(String, i64, Option<String>)> = vault
        .cache
        .iter()
        .filter_map(|s| {
            let id = s["id"].as_str()?;
            let chat = vault.chat(id).filter(|c| vault.visible(c))?;
            let newest = s["last_admin_message_id"].as_i64().unwrap_or(0);
            (newest > chat.notified.max(chat.read)).then(|| (id.to_string(), newest, view(s, vault, &peers).subject))
        })
        .collect();
    if fresh.is_empty() {
        return;
    }
    if state.settings().notify_replies {
        let general = match fresh.len() {
            1 => "New reply in your chat".to_string(),
            n => format!("New replies in {n} chats"),
        };
        let window = app.get_webview_window("main");
        if window.as_ref().and_then(|w| w.is_focused().ok()).unwrap_or(false) {
            let text = match fresh.as_slice() {
                [(_, _, Some(subject))] => format!("New reply on “{subject}”"),
                _ => general,
            };
            let _ = app.emit("ttz-reply", json!({ "id": fresh[0].0, "text": text }));
        } else {
            crate::notify::show(app, "TheHolyOneZ replied".into(), general);
            if let Some(w) = window {
                let _ = w.request_user_attention(Some(tauri::UserAttentionType::Informational));
            }
        }
    }
    let _ = update(state, |v| {
        for (id, newest, _) in &fresh {
            if let Some(c) = v.chat_mut(id) {
                c.notified = *newest;
            }
        }
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> SecretKey {
        SecretKey::from_bytes([byte; 32])
    }

    #[test]
    fn boxes_open_both_ways_and_with_older_admin_keys() {
        let (visitor, admin, old_admin) = (key(1), key(2), key(3));
        let to_admin = seal("hallo TheHolyOneZ ✓", &admin.public_key(), &visitor);
        let (content, nonce) = (to_admin["encrypted_content"].as_str().unwrap(), to_admin["nonce"].as_str().unwrap());
        assert_eq!(open(content, nonce, &visitor.public_key(), &admin).as_deref(), Some("hallo TheHolyOneZ ✓"), "TheHolyOneZ reads it");
        let peers = [old_admin.public_key(), admin.public_key()];
        assert_eq!(open_any(content, nonce, &peers, &visitor).as_deref(), Some("hallo TheHolyOneZ ✓"), "and so does the sender");
        assert_eq!(open_any(content, nonce, &peers[..1], &visitor), None);
        let stored = format!("{nonce}:{content}");
        assert_eq!(open_stored(&stored, &peers, &visitor).as_deref(), Some("hallo TheHolyOneZ ✓"));
        assert_eq!(open_stored("garbage", &peers, &visitor), None);
    }

    #[test]
    fn matches_the_websites_tweetnacl() {
        let (visitor, admin) = (key(1), key(2));
        let nonce = [7u8; 24];
        let ours = SalsaBox::new(&admin.public_key(), &visitor).encrypt(GenericArray::from_slice(&nonce), "Hi TheZ — äöü 🚀".as_bytes()).unwrap();
        assert_eq!(STANDARD.encode(&ours), TWEETNACL_BOX);
        assert_eq!(STANDARD.encode(visitor.public_key().as_bytes()), TWEETNACL_VISITOR_PUB);
        assert_eq!(
            open(TWEETNACL_REPLY, &STANDARD.encode(nonce), &admin.public_key(), &visitor).as_deref(),
            Some("Reply from TheZ"),
            "a reply sealed by tweetnacl with the admin key opens here"
        );
    }

    #[tokio::test]
    async fn the_sign_in_listener_takes_only_its_own_redirect() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let waiting = tokio::spawn(async move { wait_for_code(&listener, "our-state-1234567890").await });
        let get = |path: String| async move {
            let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).await.unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).await.unwrap();
            out
        };
        assert!(get("/favicon.ico".into()).await.starts_with("HTTP/1.1 404"));
        let code = "ab".repeat(32);
        assert!(get(format!("/ttz/callback?code={code}&state=someone-else")).await.starts_with("HTTP/1.1 404"), "a foreign state is ignored");
        let page = get(format!("/ttz/callback?code={code}&state=our-state-1234567890")).await;
        assert!(page.contains("ZLauncher is signed in"));
        assert_eq!(waiting.await.unwrap().unwrap(), code);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_ttz_end_to_end() {
        use std::process::Command;
        let base = std::env::var("ZL_TTZ_BASE").expect("ZL_TTZ_BASE");
        let script = std::env::var("ZL_TTZ_SQL").expect("ZL_TTZ_SQL");
        let sql = |q: &str| -> String {
            let out = Command::new(&script).arg(q).output().unwrap();
            assert!(out.status.success(), "{q}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        let dir = std::env::temp_dir().join(format!("zl-ttz-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let state: Shared = std::sync::Arc::new(crate::app::App::new(dir.join("data"), dir.join("cache")).unwrap());

        let admin = SecretKey::generate(&mut OsRng);
        sql("UPDATE admin_keys SET active = 0");
        sql(&format!("INSERT INTO admin_keys (public_key, active) VALUES ('{}', 1)", public_of(&admin)));
        let thez_reads = |stored: &str, visitor: &str| open_stored(stored, &[public_from(visitor).unwrap()], &admin);

        let chat = create(&state, NewChat { kind: "help".into(), subject: "ZType crashes".into(), message: "Hi TheHolyOneZ, it crashes on start.".into(), anonymous: true, name: "Max".into() })
            .await
            .unwrap();
        assert_eq!((chat.subject.as_deref(), chat.name.as_deref(), chat.anonymous, chat.readable), (Some("ZType crashes"), Some("Max"), true, true));
        let vault = load(&state);
        let local = vault.chat(&chat.id).unwrap().clone();
        let row = sql(&format!("SELECT user_id IS NULL, access_hash, source, display_name, subject, origin_pubkey, sender_pubkey FROM tickets WHERE id = '{}'", chat.id));
        let f: Vec<&str> = row.split('\t').collect();
        let visitor_pub = f[6].to_string();
        assert_eq!((f[0], f[2]), ("1", "app"), "not linked to anyone, marked as from the app");
        assert_eq!(f[1], hex_sha(local.access.as_deref().unwrap()), "the server keeps only the token's hash");
        assert_eq!(f[5], visitor_pub);
        assert_eq!(thez_reads(f[3], &visitor_pub).as_deref(), Some("Max"), "TheHolyOneZ reads the name");
        assert_eq!(thez_reads(f[4], &visitor_pub).as_deref(), Some("ZType crashes"), "and the subject");
        let first = sql(&format!("SELECT CONCAT(nonce, ':', encrypted_content) FROM messages WHERE ticket_id = '{}'", chat.id));
        assert_eq!(thez_reads(&first, &visitor_pub).as_deref(), Some("Hi TheHolyOneZ, it crashes on start."));

        let reply = seal("Fixed in 1.2, update please!", &public_from(&visitor_pub).unwrap(), &admin);
        sql(&format!(
            "INSERT INTO messages (ticket_id, sender_type, encrypted_content, nonce) VALUES ('{}', 'ADMIN', '{}', '{}')",
            chat.id, reply["encrypted_content"].as_str().unwrap(), reply["nonce"].as_str().unwrap()
        ));
        let synced = state_of(&sync(&state).await.unwrap(), None);
        assert!(synced.chats.iter().find(|c| c.id == chat.id).unwrap().unread);
        let thread = messages(&state, &chat.id, 0, -1).await.unwrap();
        let texts: Vec<(bool, Option<&str>)> = thread.messages.iter().map(|m| (m.mine, m.text.as_deref())).collect();
        assert_eq!(texts, [(true, Some("Hi TheHolyOneZ, it crashes on start.")), (false, Some("Fixed in 1.2, update please!"))]);
        assert!(!thread.chat.unread);
        let sent = send(&state, &chat.id, "Thanks!", &[], &|_, _| {}).await.unwrap();
        let stored = sql(&format!("SELECT CONCAT(nonce, ':', encrypted_content) FROM messages WHERE id = {}", sent.id));
        assert_eq!(thez_reads(&stored, &visitor_pub).as_deref(), Some("Thanks!"));
        assert_eq!(messages(&state, &chat.id, sent.id - 1, -1).await.unwrap().messages.len(), 1, "since= gives only what's new");

        let api = Api::new(&state);
        for auth in [Auth::Access(&"0".repeat(64)), Auth::None] {
            match api.call(true, "messages", &[("ticket_id", chat.id.clone())], None, auth).await {
                Err(ApiError::Other(m)) => assert_eq!(m, "Not found"),
                _ => panic!("a stranger read the chat"),
            }
        }

        let admin2 = SecretKey::generate(&mut OsRng);
        sql("UPDATE admin_keys SET active = 0");
        sql(&format!("INSERT INTO admin_keys (public_key, active) VALUES ('{}', 1)", public_of(&admin2)));
        sync(&state).await.unwrap();
        let thread = messages(&state, &chat.id, 0, -1).await.unwrap();
        assert!(thread.messages.iter().all(|m| m.text.is_some()), "everything sealed for the old key still opens");
        let sent = send(&state, &chat.id, "New key?", &[], &|_, _| {}).await.unwrap();
        let stored = sql(&format!("SELECT CONCAT(nonce, ':', encrypted_content) FROM messages WHERE id = {}", sent.id));
        assert_eq!(open_stored(&stored, &[public_from(&visitor_pub).unwrap()], &admin2).as_deref(), Some("New key?"));

        let uid = format!("u{}", std::process::id());
        sql(&format!("INSERT INTO users (id, discord_id, username) VALUES ('{uid}', 'd{uid}', 'Tester')"));
        let browser = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
        let login = browser.get(format!("{base}/test-login.php?id={uid}")).send().await.unwrap();
        let cookie = login.headers().get_all("set-cookie").iter().filter_map(|v| v.to_str().ok()).find(|v| v.starts_with("ttz_sess=")).unwrap().split(';').next().unwrap().to_string();
        let verifier = random_url_safe(32);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let ask = browser
            .get(format!("{base}/app-login?challenge={challenge}&port=45678&state=state-from-the-app-42&device=Test%20PC"))
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(ask.status(), 303);
        let page = browser.get(format!("{base}/app-login")).header("cookie", &cookie).send().await.unwrap().text().await.unwrap();
        assert!(page.contains("Connect <em>ZLauncher</em>?") && page.contains("Tester") && page.contains("Test PC"), "{page}");
        let origin = reqwest::Url::parse(&base).unwrap().origin().ascii_serialization();
        let allow = |csrf: String| {
            browser
                .post(format!("{base}/app-login"))
                .header("cookie", &cookie)
                .header("origin", &origin)
                .json(&json!({ "decision": "allow", "csrf": csrf }))
                .send()
        };
        let answer: Value = allow("wrong".into()).await.unwrap().json().await.unwrap();
        assert_eq!(answer["ok"], false, "a wrong csrf token is refused");
        browser.get(format!("{base}/app-login?challenge={challenge}&port=45678&state=state-from-the-app-42")).header("cookie", &cookie).send().await.unwrap();
        let page = browser.get(format!("{base}/app-login")).header("cookie", &cookie).send().await.unwrap().text().await.unwrap();
        let csrf = page.split("const csrf = \"").nth(1).unwrap().split('"').next().unwrap().to_string();
        let answer: Value = allow(csrf).await.unwrap().json().await.unwrap();
        let back = reqwest::Url::parse(answer["data"]["redirect"].as_str().unwrap()).unwrap();
        assert_eq!((back.host_str(), back.port(), back.path()), (Some("127.0.0.1"), Some(45678), "/ttz/callback"));
        let q: HashMap<String, String> = back.query_pairs().into_owned().collect();
        assert_eq!(q["state"], "state-from-the-app-42");
        assert!(finish_sign_in(&state, &q["code"], "a-wrong-verifier-a-wrong-verifier-a-wrong-v").await.is_err(), "the code alone is useless");
        assert!(finish_sign_in(&state, &q["code"], &verifier).await.unwrap_err().contains("already used"), "and single use");

        browser.get(format!("{base}/app-login?challenge={challenge}&port=45678&state=state-from-the-app-42")).header("cookie", &cookie).send().await.unwrap();
        let page = browser.get(format!("{base}/app-login")).header("cookie", &cookie).send().await.unwrap().text().await.unwrap();
        let csrf = page.split("const csrf = \"").nth(1).unwrap().split('"').next().unwrap().to_string();
        let answer: Value = allow(csrf).await.unwrap().json().await.unwrap();
        let back = reqwest::Url::parse(answer["data"]["redirect"].as_str().unwrap()).unwrap();
        let code = back.query_pairs().find(|(k, _)| k == "code").unwrap().1.to_string();
        let user = finish_sign_in(&state, &code, &verifier).await.unwrap();
        assert_eq!(user.username, "Tester");

        let mine = create(&state, NewChat { kind: "feedback".into(), subject: "Love it".into(), message: "Great launcher".into(), anonymous: false, name: String::new() })
            .await
            .unwrap();
        assert!(!mine.anonymous && mine.name.is_none());
        assert_eq!(sql(&format!("SELECT user_id, access_hash IS NULL FROM tickets WHERE id = '{}'", mine.id)), format!("{uid}\t1"));
        sql(&format!("INSERT INTO tickets (id, user_id, type, subject, sender_pubkey) VALUES ('cweb{uid}', '{uid}', 'GENERAL', 'x:y', '{visitor_pub}')"));
        let st = state_of(&sync(&state).await.unwrap(), None);
        let web = st.chats.iter().find(|c| c.id == format!("cweb{uid}")).expect("browser chats are listed");
        assert!(!web.readable && web.subject.is_none());
        assert!(st.chats.iter().any(|c| c.id == chat.id) && st.chats.iter().any(|c| c.id == mine.id));
        assert_eq!(sql(&format!("SELECT user_id IS NULL FROM tickets WHERE id = '{}'", chat.id)), "1", "the anonymous chat stays unlinked");

        let token = load(&state).account.unwrap().token;
        sign_out(&state).await.unwrap();
        assert!(matches!(api.call(false, "sync", &[], None, Auth::Token(&token)).await, Err(ApiError::SignedOut(_))));
        let st = state_of(&load(&state), None);
        assert!(st.account.is_none() && !st.chats.iter().any(|c| c.id == mine.id), "account chats hide while signed out");
        assert!(st.chats.iter().any(|c| c.id == chat.id), "anonymous ones stay");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_ttz_files_and_changes() {
        use std::process::Command;
        let base = std::env::var("ZL_TTZ_BASE").expect("ZL_TTZ_BASE");
        let script = std::env::var("ZL_TTZ_SQL").expect("ZL_TTZ_SQL");
        let www = PathBuf::from(std::env::var("ZL_TTZ_WWW").expect("ZL_TTZ_WWW"));
        let sql = |q: &str| -> String {
            let out = Command::new(&script).arg(q).output().unwrap();
            assert!(out.status.success(), "{q}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        let dir = std::env::temp_dir().join(format!("zl-ttz-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("in")).unwrap();
        let saves = dir.join("saved");
        let state: Shared = std::sync::Arc::new(crate::app::App::new(dir.join("data"), dir.join("cache")).unwrap());
        let quiet = |_: &str, _: f32| {};
        let no_progress = |_: f32| {};
        let stored = |id: &str| www.join("storage").join(&id[..2]).join(format!("{id}.enc"));
        let write = |name: &str, bytes: &[u8]| {
            let p = dir.join("in").join(name);
            std::fs::write(&p, bytes).unwrap();
            p
        };

        let admin = SecretKey::generate(&mut OsRng);
        sql("UPDATE admin_keys SET active = 0");
        sql(&format!("INSERT INTO admin_keys (public_key, active) VALUES ('{}', 1)", public_of(&admin)));
        sql("DELETE FROM rate_limits");
        let new_chat = |subject: &'static str| create(&state, NewChat { kind: "help".into(), subject: subject.into(), message: "Hi".into(), anonymous: true, name: String::new() });
        let chat = new_chat("With files").await.unwrap();
        let other = new_chat("Someone else").await.unwrap();
        let vault = load(&state);
        let access = vault.chat(&chat.id).unwrap().access.clone().unwrap();
        let other_access = vault.chat(&other.id).unwrap().access.clone().unwrap();
        let visitor_pub = sql(&format!("SELECT sender_pubkey FROM tickets WHERE id = '{}'", chat.id));
        let visitor = public_from(&visitor_pub).unwrap();

        assert!(stage(&write("setup.exe", b"MZ\x90\0")).error.unwrap().contains("can't be sent"));
        assert!(stage(&write("run.bat", b"@echo off")).error.is_some());
        assert!(stage(&write("fake.png", b"MZ\x90\0not a picture")).error.unwrap().contains("doesn't match"));
        assert!(stage(&write("fake.txt", b"MZ\x90\0\0\0binary")).error.unwrap().contains("doesn't match"));
        assert!(stage(&dir.join("in")).error.unwrap().contains("Folders"));

        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(40, 30, image::Rgba([20, 200, 90, 255])).write_to(&mut png, image::ImageFormat::Png).unwrap();
        let png = png.into_inner();
        let located = [&png[..8], &[&9u32.to_be_bytes()[..], b"tEXt", b"GPS 52.51", &[0, 0, 0, 0]].concat(), &png[8..]].concat();
        let script_text = "import os\nos.system('rm -rf /')  # must only ever be text\n";
        let video: Vec<u8> = [&b"\0\0\0\x18ftypisom"[..], &vec![0x5a; files::CHUNK * 2 + 777]].concat();
        let picks = [
            stage(&write("holiday.png", &located)),
            stage(&write("danger.py", script_text.as_bytes())),
            stage(&write("source.zip", b"PK\x03\x04 a zip stays a zip")),
            stage(&write("clip.mp4", &video)),
        ];
        assert!(picks.iter().all(|p| p.error.is_none()), "{picks:?}");
        assert!(picks[3].metadata && !picks[0].metadata);
        let ids: Vec<String> = picks.iter().map(|p| p.id.clone()).collect();
        let sent = send(&state, &chat.id, "Here you go", &ids, &quiet).await.unwrap();
        assert_eq!(sent.files.len(), 4);
        let (img, py, zip, clip) = (sent.files[0].id.clone(), sent.files[1].id.clone(), sent.files[2].id.clone(), sent.files[3].id.clone());
        assert_eq!(sent.files.iter().map(|f| f.kind).collect::<Vec<_>>(), [Some(Kind::Image), Some(Kind::Text), Some(Kind::Archive), Some(Kind::Video)]);

        let on_disk = std::fs::read(stored(&py)).unwrap();
        assert!(!on_disk.windows(6).any(|w| w == b"import") && on_disk.len() == script_text.len() + 16);
        assert_eq!(std::fs::read(stored(&clip)).unwrap().len(), video.len() + 3 * 16);
        assert!(std::fs::read_to_string(www.join("storage/.htaccess")).unwrap().contains("Deny from all"), "the folder guards itself");
        let row = sql(&format!("SELECT encrypted_files FROM messages WHERE id = {}", sent.id));
        assert!(!row.contains("danger") && !row.contains("holiday"));
        assert_eq!(sql(&format!("SELECT COUNT(*) FROM attachments WHERE message_id = {}", sent.id)), "4");
        let list = open_stored(&row, std::slice::from_ref(&visitor), &admin).expect("TheHolyOneZ opens the list");
        assert!(list.contains("danger.py") && list.contains("holiday.png"));

        let thread = messages(&state, &chat.id, 0, -1).await.unwrap();
        assert_eq!(thread.rev, 0);
        assert!(matches!(file_preview(&state, &chat.id, &img).await.unwrap(), Preview::Image { url } if url.starts_with("data:image/")));
        assert!(matches!(file_preview(&state, &chat.id, &py).await.unwrap(), Preview::Text { text, .. } if text == script_text));
        assert!(file_preview(&state, &chat.id, &zip).await.unwrap_err().contains("only be saved"), "a zip is never opened");
        assert!(file_preview(&state, &chat.id, &clip).await.unwrap_err().contains("only be saved"), "a video isn't played here");

        let saved_py = file_save(&state, &chat.id, &py, &saves, &no_progress).await.unwrap();
        assert_eq!(std::fs::read_to_string(&saved_py).unwrap(), script_text);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&saved_py).unwrap().permissions().mode() & 0o111, 0, "nothing saved is executable");
        }
        let again = file_save(&state, &chat.id, &py, &saves, &no_progress).await.unwrap();
        assert_ne!(saved_py, again, "and nothing is overwritten");
        let saved_video = file_save(&state, &chat.id, &clip, &saves, &no_progress).await.unwrap();
        assert_eq!(std::fs::read(&saved_video).unwrap(), video, "three chunks back together");
        let saved_png = std::fs::read(file_save(&state, &chat.id, &img, &saves, &no_progress).await.unwrap()).unwrap();
        assert_eq!(saved_png, png, "the location left before the file did");

        let client = reqwest::Client::new();
        let raw = |id: String, ticket: String, header: Option<(&'static str, String)>| {
            let mut req = client.get(format!("{base}/api/app.php?action=file&ticket_id={ticket}&id={id}&chunk=0"));
            if let Some((k, v)) = header {
                req = req.header(k, v);
            }
            req.send()
        };
        let mine = raw(py.clone(), chat.id.clone(), Some(("X-TTZ-Access", access.clone()))).await.unwrap();
        assert_eq!(mine.status(), 200);
        assert_eq!(mine.headers()["content-type"], "application/octet-stream");
        assert!(mine.headers()["content-disposition"].to_str().unwrap().starts_with("attachment"));
        assert_eq!(mine.headers()["x-content-type-options"], "nosniff");
        assert_eq!(raw(py.clone(), chat.id.clone(), None).await.unwrap().status(), 404, "no token");
        assert_eq!(raw(py.clone(), chat.id.clone(), Some(("X-TTZ-Access", other_access.clone()))).await.unwrap().status(), 404, "another chat's token");
        assert_eq!(raw(py.clone(), other.id.clone(), Some(("X-TTZ-Access", other_access.clone()))).await.unwrap().status(), 404, "this file isn't in that chat");
        for bad in ["../../includes/config", "..%2f..%2fincludes%2fconfig", &"f".repeat(32)] {
            assert_eq!(raw(bad.to_string(), chat.id.clone(), Some(("X-TTZ-Access", access.clone()))).await.unwrap().status(), 404, "{bad}");
        }

        let api = Api::new(&state);
        let auth = Auth::Access(&access);
        let code = b"<?php echo 'PWNED'; ?>";
        let begun = api.call(false, "file_begin", &[], Some(json!({ "ticket_id": chat.id, "size": code.len() })), auth).await.ok().unwrap();
        let evil = begun["id"].as_str().unwrap().to_string();
        api.put_chunk(&chat.id, &evil, 0, code.to_vec(), auth).await.unwrap();
        let direct = client.get(format!("{base}/storage/{}/{evil}.enc", &evil[..2])).send().await.unwrap();
        assert_eq!(direct.status(), 403);
        assert!(!direct.text().await.unwrap().contains("PWNED"));
        assert_eq!(client.get(format!("{base}/storage/.htaccess")).send().await.unwrap().status(), 403);
        assert_eq!(raw(evil.clone(), chat.id.clone(), Some(("X-TTZ-Access", access.clone()))).await.unwrap().status(), 404, "not sent yet, so not served");

        let begin = |size: u64| api.call(false, "file_begin", &[], Some(json!({ "ticket_id": chat.id, "size": size })), auth);
        assert!(begin(15).await.is_err(), "smaller than one tag");
        assert!(begin(files::MAX_BYTES + 100 * 1024 * 1024).await.is_err(), "too big");
        let two = begin(files::CHUNK as u64 + 16 + 40).await.ok().unwrap();
        let two = two["id"].as_str().unwrap().to_string();
        assert!(api.put_chunk(&chat.id, &two, 1, vec![1; 40], auth).await.unwrap_err().contains("order"));
        assert!(api.put_chunk(&chat.id, &two, 0, vec![1; 40], auth).await.unwrap_err().contains("size"));
        assert!(api.put_chunk(&chat.id, &two, 7, vec![1; 40], auth).await.is_err());
        let as_json = client
            .post(format!("{base}/api/app.php?action=file_chunk&ticket_id={}&id={two}&chunk=0", chat.id))
            .header("X-TTZ-Access", &access)
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(as_json.status(), 415);

        sql("DELETE FROM rate_limits");
        for _ in 0..24 {
            let _ = begin(files::MAX_BYTES).await;
        }
        let pending = sql(&format!("SELECT COUNT(*) FROM attachments WHERE ticket_id = '{}' AND message_id IS NULL", chat.id)).parse::<i64>().unwrap();
        assert_eq!(pending, 20);
        assert!(matches!(begin(16).await, Err(ApiError::Other(m)) if m.contains("unfinished")));
        assert!(api.call(false, "file_begin", &[], Some(json!({ "ticket_id": other.id, "size": files::MAX_BYTES })), Auth::Access(&other_access)).await.is_ok(), "and nobody else is locked out");
        sql(&format!("DELETE FROM attachments WHERE ticket_id = '{}' AND message_id IS NULL AND id NOT IN ('{evil}', '{two}')", chat.id));

        let thez_now = public_from(load(&state).admin_active.as_deref().unwrap()).unwrap();
        let mut body = seal("x", &thez_now, &secret_from(&load(&state).chat(&other.id).unwrap().secret).unwrap());
        body["ticket_id"] = json!(other.id);
        body["sender_pubkey"] = json!(public_of(&secret_from(&load(&state).chat(&other.id).unwrap().secret).unwrap()));
        body["files"] = json!([evil]);
        body["encrypted_files"] = body.clone();
        let stolen = api.call(false, "send", &[], Some(body), Auth::Access(&other_access)).await;
        assert!(matches!(stolen, Err(ApiError::Other(m)) if m.contains("did not finish")), "another chat's upload");
        let mut body = seal("x", &thez_now, &secret_from(&load(&state).chat(&chat.id).unwrap().secret).unwrap());
        body["ticket_id"] = json!(chat.id);
        body["sender_pubkey"] = json!(visitor_pub);
        body["files"] = json!([two]);
        body["encrypted_files"] = body.clone();
        assert!(api.call(false, "send", &[], Some(body), auth).await.is_err(), "an unfinished upload");
        assert_eq!(sql(&format!("SELECT COUNT(*) FROM messages WHERE ticket_id = '{}' AND encrypted_content = ''", chat.id)), "0", "and nothing half-written stays");

        sql(&format!("UPDATE attachments SET created_at = NOW() - INTERVAL 2 DAY WHERE id = '{evil}'"));
        let _ = begin(16).await;
        assert!(!stored(&evil).exists(), "swept");
        assert_eq!(sql(&format!("SELECT COUNT(*) FROM attachments WHERE id = '{evil}'")), "0");

        let sealed_by_thez = |name: &str, bytes: &[u8]| {
            let id: String = random_url_safe(16).bytes().map(|b| format!("{:02x}", b)).collect::<String>()[..32].to_string();
            let sealer = Sealer::new([3; 32], [4; 16]);
            let cipher = sealer.seal(0, bytes);
            let list = json!([{ "id": id, "key": STANDARD.encode([3u8; 32]), "nonce": STANDARD.encode([4u8; 16]), "name": name, "size": bytes.len(), "chunks": 1 }]).to_string();
            let meta = seal(&list, &visitor, &admin);
            let text = seal("", &visitor, &admin);
            let m = sql(&format!(
                "INSERT INTO messages (ticket_id, sender_type, encrypted_content, nonce, encrypted_files) VALUES ('{}', 'ADMIN', '{}', '{}', '{}:{}'); SELECT LAST_INSERT_ID()",
                chat.id, text["encrypted_content"].as_str().unwrap(), text["nonce"].as_str().unwrap(), meta["nonce"].as_str().unwrap(), meta["encrypted_content"].as_str().unwrap()
            ));
            sql(&format!("INSERT INTO attachments (id, ticket_id, message_id, sender_type, size, chunks, received) VALUES ('{id}', '{}', {m}, 'ADMIN', {}, 1, 1)", chat.id, cipher.len()));
            std::fs::create_dir_all(stored(&id).parent().unwrap()).unwrap();
            std::fs::write(stored(&id), &cipher).unwrap();
            id
        };
        let before = std::fs::read_dir(&saves).unwrap().count();
        let exe = sealed_by_thez("setup.exe", b"MZ\x90\0");
        let disguised = sealed_by_thez("cute.png", b"MZ\x90\0 this is a program");
        let flipped = sealed_by_thez("evil\u{202e}gnp.exe", b"MZ\x90\0");
        let traversal = sealed_by_thez("../../../.bashrc.txt", b"echo hi\n");
        let thread = messages(&state, &chat.id, 0, -1).await.unwrap();
        let file = |id: &str| thread.messages.iter().flat_map(|m| &m.files).find(|f| f.id == id).unwrap().clone();
        assert_eq!(file(&exe).kind, None);
        assert_eq!(file(&flipped).name, "evilgnp.exe");
        assert_eq!(file(&flipped).kind, None, "the direction trick doesn't hide the extension");
        assert_eq!(file(&traversal).name, ".bashrc.txt".trim_start_matches('.'));
        assert!(file_save(&state, &chat.id, &exe, &saves, &no_progress).await.unwrap_err().contains("blocked"));
        assert!(file_save(&state, &chat.id, &flipped, &saves, &no_progress).await.unwrap_err().contains("blocked"));
        assert!(file_save(&state, &chat.id, &disguised, &saves, &no_progress).await.unwrap_err().contains("blocked"));
        assert!(file_preview(&state, &chat.id, &disguised).await.is_err());
        assert_eq!(std::fs::read_dir(&saves).unwrap().count(), before, "not a byte of those reached the disk");
        let landed = file_save(&state, &chat.id, &traversal, &saves, &no_progress).await.unwrap();
        assert_eq!(landed.parent().unwrap(), saves, "and a name can't climb out of the folder");

        let mut tampered = std::fs::read(stored(&py)).unwrap();
        tampered[20] ^= 1;
        std::fs::write(stored(&py), tampered).unwrap();
        let count = std::fs::read_dir(&saves).unwrap().count();
        assert!(file_save(&state, &chat.id, &py, &saves, &no_progress).await.unwrap_err().contains("changed"));
        assert_eq!(std::fs::read_dir(&saves).unwrap().count(), count);

        let thread = messages(&state, &chat.id, 0, -1).await.unwrap();
        let (last_id, rev) = (thread.messages.last().unwrap().id, thread.rev);
        let first = thread.messages[0].id;
        let edited = edit(&state, &chat.id, first, "Hi, edited").await.unwrap();
        assert!(edited.edited && edited.text.as_deref() == Some("Hi, edited"));
        let stored_text = sql(&format!("SELECT CONCAT(nonce, ':', encrypted_content) FROM messages WHERE id = {first}"));
        assert_eq!(open_stored(&stored_text, std::slice::from_ref(&visitor), &admin).as_deref(), Some("Hi, edited"), "TheHolyOneZ reads the new text");
        let update = messages(&state, &chat.id, last_id, rev).await.unwrap();
        assert!(update.messages.is_empty() && update.rev == rev + 1);
        assert_eq!(update.changed.iter().map(|m| (m.id, m.text.clone())).collect::<Vec<_>>(), [(first, Some("Hi, edited".to_string()))]);
        assert!(messages(&state, &chat.id, last_id, update.rev).await.unwrap().changed.is_empty(), "and only once");

        let theirs = sql(&format!("SELECT MAX(id) FROM messages WHERE ticket_id = '{}' AND sender_type = 'ADMIN'", chat.id)).parse::<i64>().unwrap();
        assert!(edit(&state, &chat.id, theirs, "forged").await.unwrap_err().contains("your own"));
        assert!(delete(&state, &chat.id, theirs).await.unwrap_err().contains("your own"));
        let foreign = sql(&format!("SELECT MIN(id) FROM messages WHERE ticket_id = '{}'", other.id)).parse::<i64>().unwrap();
        assert_eq!(edit(&state, &chat.id, foreign, "forged").await.unwrap_err(), "Not found");
        assert_eq!(delete(&state, &chat.id, foreign).await.unwrap_err(), "Not found");

        let gone = delete(&state, &chat.id, sent.id).await.unwrap();
        assert_eq!((gone.deleted.as_deref(), gone.text.as_deref(), gone.files.len()), (Some("visitor"), None, 0));
        assert_eq!(sql(&format!("SELECT LENGTH(encrypted_content), nonce IS NULL, encrypted_files IS NULL FROM messages WHERE id = {}", sent.id)), "0\t1\t1");
        assert!(!stored(&clip).exists() && !stored(&zip).exists());
        assert_eq!(sql(&format!("SELECT COUNT(*) FROM attachments WHERE message_id = {}", sent.id)), "0");
        assert_eq!(raw(zip.clone(), chat.id.clone(), Some(("X-TTZ-Access", access.clone()))).await.unwrap().status(), 404);
        assert_eq!(delete(&state, &chat.id, sent.id).await.unwrap_err(), "Not found", "gone is gone");
        assert!(edit(&state, &chat.id, sent.id, "back").await.is_err());

        sql(&format!("UPDATE tickets SET status = 'CLOSED' WHERE id = '{}'", chat.id));
        assert!(begin(16).await.is_err());
        assert!(edit(&state, &chat.id, first, "late").await.unwrap_err().contains("closed"));
        assert!(delete(&state, &chat.id, first).await.is_ok());
        sql(&format!("UPDATE tickets SET status = 'OPEN' WHERE id = '{}'", chat.id));

        let session = |uid: String| {
            let base = base.clone();
            async move {
                let browser = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
                let login = browser.get(format!("{base}/test-login.php?id={uid}")).send().await.unwrap();
                login.headers().get_all("set-cookie").iter().filter_map(|v| v.to_str().ok()).find(|v| v.starts_with("ttz_sess=")).unwrap().split(';').next().unwrap().to_string()
            }
        };
        let tag = std::process::id();
        let (admin_discord, admin_email) = ("100000000000000001", "admin@example.test");
        sql(&format!("DELETE FROM users WHERE discord_id = '{admin_discord}' OR email = '{admin_email}'"));
        sql(&format!("INSERT INTO users (id, discord_id, username) VALUES ('adm{tag}', '{admin_discord}', 'TheHolyOneZ')"));
        sql(&format!("INSERT INTO users (id, discord_id, username, email) VALUES ('spoof{tag}', 'fake{tag}', 'Spoof', '{admin_email}')"));
        sql(&format!("INSERT INTO users (id, google_id, username, email) VALUES ('goog{tag}', 'g{tag}', 'Google TheHolyOneZ', '{admin_email}')"));
        let origin = reqwest::Url::parse(&base).unwrap().origin().ascii_serialization();
        let stats = |cookie: String| client.get(format!("{base}/api/admin.php?action=stats")).header("cookie", cookie).send();
        assert_eq!(stats(session(format!("spoof{tag}")).await).await.unwrap().status(), 403, "a Discord account can't claim the admin's address");
        assert_eq!(stats(session(format!("goog{tag}")).await).await.unwrap().status(), 200, "a verified Google sign-in can");
        let thez = session(format!("adm{tag}")).await;
        assert_eq!(stats(thez.clone()).await.unwrap().status(), 200);

        let web = |method: reqwest::Method, url: String, body: Value| {
            client.request(method, format!("{base}{url}")).header("cookie", thez.clone()).header("origin", origin.clone()).json(&body).send()
        };
        let visitor_msg = sql(&format!("SELECT MAX(id) FROM messages WHERE ticket_id = '{}' AND sender_type = 'VISITOR' AND deleted_at IS NULL", other.id)).parse::<i64>().unwrap();
        let reply = seal("from TheHolyOneZ", &public_from(&sql(&format!("SELECT sender_pubkey FROM tickets WHERE id = '{}'", other.id))).unwrap(), &admin);
        let mut send_body = reply.clone();
        send_body["ticket_id"] = json!(other.id);
        let answer: Value = web(reqwest::Method::POST, "/api/messages.php".into(), send_body).await.unwrap().json().await.unwrap();
        let his = answer["data"]["id"].as_i64().unwrap();
        let mut change = seal("TheHolyOneZ, corrected", &public_from(&sql(&format!("SELECT sender_pubkey FROM tickets WHERE id = '{}'", other.id))).unwrap(), &admin);
        change["ticket_id"] = json!(other.id);
        change["id"] = json!(visitor_msg);
        let refused = web(reqwest::Method::PATCH, "/api/messages.php".into(), change.clone()).await.unwrap();
        assert_eq!(refused.status(), 403, "TheHolyOneZ can't put words in a visitor's mouth");
        change["id"] = json!(his);
        assert_eq!(web(reqwest::Method::PATCH, "/api/messages.php".into(), change).await.unwrap().status(), 200, "his own he can edit");
        let del = web(reqwest::Method::DELETE, format!("/api/messages.php?ticket_id={}&id={visitor_msg}", other.id), json!({})).await.unwrap();
        assert_eq!(del.status(), 200, "and he can delete a visitor's message");
        assert_eq!(sql(&format!("SELECT deleted_by FROM messages WHERE id = {visitor_msg}")), "ADMIN");
        let seen = messages(&state, &other.id, 0, -1).await.unwrap();
        assert_eq!(seen.messages.iter().find(|m| m.id == visitor_msg).unwrap().deleted.as_deref(), Some("admin"));
        assert_eq!(seen.messages.iter().find(|m| m.id == his).unwrap().text.as_deref(), Some("TheHolyOneZ, corrected"));

        let cross = client.delete(format!("{base}/api/messages.php?ticket_id={}&id={his}", other.id)).header("cookie", &thez).header("origin", "https://evil.example").send().await.unwrap();
        assert_eq!(cross.status(), 403);
        let bare = client.delete(format!("{base}/api/messages.php?ticket_id={}&id={his}", other.id)).header("cookie", &thez).send().await.unwrap();
        assert_eq!(bare.status(), 403);

        sql("DELETE FROM rate_limits");
        let key = SecretKey::generate(&mut OsRng);
        let adminp = public_from(load(&state).admin_active.as_deref().unwrap()).unwrap();
        let made = client
            .post(format!("{base}/api/tickets.php"))
            .header("origin", &origin)
            .json(&json!({ "type": "GENERAL", "sender_pubkey": public_of(&key), "encrypted_subject": seal("Hi", &adminp, &key), "first_message": seal("Hello", &adminp, &key) }))
            .send()
            .await
            .unwrap();
        let anon = made.headers().get_all("set-cookie").iter().filter_map(|v| v.to_str().ok()).find(|v| v.starts_with("ttz_anon=")).unwrap().split(';').next().unwrap().to_string();
        let answer: Value = made.json().await.unwrap();
        let ticket = answer["data"]["ticket_id"].as_str().unwrap().to_string();
        sql(&format!("UPDATE tickets SET notes = 'private note about this visitor', label = 'watch' WHERE id = '{ticket}'"));
        let page: Value = client.get(format!("{base}/api/tickets.php?id={ticket}")).header("cookie", &anon).send().await.unwrap().json().await.unwrap();
        assert_eq!(page["ok"], true);
        let text = page.to_string();
        assert!(!text.contains("private note") && page["data"].get("notes").is_none() && page["data"].get("anon_token").is_none() && page["data"].get("access_hash").is_none(), "{text}");
        assert!(client.get(format!("{base}/api/tickets.php?id={ticket}")).send().await.unwrap().status().is_client_error(), "and strangers get nothing");

        let up: Value = client
            .post(format!("{base}/api/files.php?action=begin&ticket_id={ticket}"))
            .header("cookie", &anon)
            .header("origin", &origin)
            .json(&json!({ "size": 21 }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let web_file = up["data"]["id"].as_str().unwrap().to_string();
        let chunk = |cookie: Option<&str>, site: &str| {
            let mut req = client.post(format!("{base}/api/files.php?action=chunk&ticket_id={ticket}&id={web_file}&chunk=0")).header("origin", site).header("content-type", "application/octet-stream").body(vec![9u8; 21]);
            if let Some(c) = cookie {
                req = req.header("cookie", c);
            }
            req.send()
        };
        assert_eq!(chunk(None, &origin).await.unwrap().status(), 403, "without the chat's cookie");
        assert_eq!(chunk(Some(&anon), "https://evil.example").await.unwrap().status(), 403, "from another site");
        assert_eq!(chunk(Some(&anon), &origin).await.unwrap().status(), 200);
        let mut body = seal("", &adminp, &key);
        body["ticket_id"] = json!(ticket);
        body["sender_pubkey"] = json!(public_of(&key));
        body["files"] = json!([web_file]);
        body["encrypted_files"] = seal("[]", &adminp, &key);
        let posted = client.post(format!("{base}/api/messages.php")).header("cookie", &anon).header("origin", &origin).json(&body).send().await.unwrap();
        assert_eq!(posted.status(), 200);
        let served = client.get(format!("{base}/api/files.php?action=file&ticket_id={ticket}&id={web_file}&chunk=0")).header("cookie", &anon).header("sec-fetch-site", "cross-site").send().await.unwrap();
        assert_eq!(served.status(), 403, "another site can't pull it through the visitor's browser");
        let served = client.get(format!("{base}/api/files.php?action=file&ticket_id={ticket}&id={web_file}&chunk=0")).header("cookie", &thez).send().await.unwrap();
        assert_eq!(served.bytes().await.unwrap().len(), 21, "TheHolyOneZ gets it");

        sql("DELETE FROM rate_limits");
        let shared = new_chat("Both clients").await.unwrap();
        let shared_pub = sql(&format!("SELECT sender_pubkey FROM tickets WHERE id = '{}'", shared.id));
        let long_text = "fn main() {}\n".repeat(50_000);
        let staged = [stage(&write("from-app.mp4", &video)), stage(&write("from-app.rs", long_text.as_bytes())), stage(&write("from-app.png", &png))];
        send(&state, &shared.id, "", &staged.iter().map(|f| f.id.clone()).collect::<Vec<_>>(), &quiet).await.unwrap();
        let site = www.to_string_lossy().to_string();
        let node = |mode: &str, input: Value| {
            use std::io::Write;
            let mut child = Command::new("node")
                .arg(std::env::var("ZL_TTZ_NODE").expect("ZL_TTZ_NODE"))
                .arg(mode)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(input.to_string().as_bytes()).unwrap();
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
            serde_json::from_slice::<Value>(&out.stdout).unwrap()
        };
        let who = json!({ "base": base, "site": site, "cookie": thez, "ticket": shared.id, "secret": STANDARD.encode(admin.to_bytes()), "peer": shared_pub });
        let read = node("read", who.clone());
        assert_eq!(read["from-app.mp4"], hex_bytes(&video), "TheHolyOneZ's browser opens the app's video");
        assert_eq!(read["from-app.rs"], hex_bytes(long_text.as_bytes()));
        assert_eq!(read["from-app.png"], hex_bytes(&png));

        let photo = crate::ttz_files::tests::jpeg_with_exif();
        let zip: Vec<u8> = [&b"PK\x03\x04"[..], &vec![7u8; files::CHUNK + 5]].concat();
        let mut input = who.clone();
        input["files"] = json!([
            { "name": "from-web.jpg", "b64": STANDARD.encode(&photo) },
            { "name": "from-web.zip", "b64": STANDARD.encode(&zip) },
            { "name": "from-web.py", "b64": STANDARD.encode(script_text) },
        ]);
        let posted = node("send", input);
        let thread = messages(&state, &shared.id, 0, -1).await.unwrap();
        let reply = thread.messages.iter().find(|m| m.id == posted["id"].as_i64().unwrap()).unwrap();
        assert_eq!((reply.mine, reply.text.as_deref()), (false, Some("")));
        let get = |name: &str| reply.files.iter().find(|f| f.name == name).unwrap().id.clone();
        let got_zip = file_save(&state, &shared.id, &get("from-web.zip"), &saves, &no_progress).await.unwrap();
        assert_eq!(std::fs::read(got_zip).unwrap(), zip, "and the app opens the browser's zip");
        let got_py = file_save(&state, &shared.id, &get("from-web.py"), &saves, &no_progress).await.unwrap();
        assert_eq!(std::fs::read_to_string(got_py).unwrap(), script_text);
        let got_jpg = std::fs::read(file_save(&state, &shared.id, &get("from-web.jpg"), &saves, &no_progress).await.unwrap()).unwrap();
        assert_eq!(got_jpg, files::scrub(photo.clone()), "both clients scrub a photo the same way");
        assert!(!got_jpg.windows(3).any(|w| w == b"GPS"));

        let reply = seal("Have you tried the new build?", &public_from(&shared_pub).unwrap(), &admin);
        sql(&format!("INSERT INTO messages (ticket_id, sender_type, encrypted_content, nonce) VALUES ('{}', 'ADMIN', '{}', '{}')", shared.id, reply["encrypted_content"].as_str().unwrap(), reply["nonce"].as_str().unwrap()));
        let listed = assistant_tool(&state, "list_ttz_chats", &json!({})).await;
        let unread = |v: &Value| v["chats"].as_array().unwrap().iter().find(|c| c["id"] == shared.id.as_str()).unwrap()["unread"].clone();
        assert_eq!(unread(&listed), true);
        assert!(!listed.to_string().contains(&access), "tokens never reach the model");
        let read = assistant_tool(&state, "read_ttz_chat", &json!({ "chat_id": shared.id })).await;
        let shown = read.to_string();
        assert!(shown.contains("Have you tried the new build?") && shown.contains("from-app.mp4"));
        assert!(!shown.contains("\"key\"") && !shown.contains("\"nonce\""), "file keys never reach the model: {shown}");
        let found = assistant_tool(&state, "search_ttz_chats", &json!({ "query": "NEW build" })).await;
        assert_eq!(found["hits"][0]["chatId"], shared.id.as_str());
        assert_eq!(found["hits"][0]["from"], "TheHolyOneZ");
        assert_eq!(assistant_tool(&state, "search_ttz_chats", &json!({ "query": "from-web.zip" })).await["count"], 1, "file names can be found too");
        assert_eq!(unread(&assistant_tool(&state, "list_ttz_chats", &json!({})).await), true, "reading for the assistant doesn't mark anything read");

        let by_number = assistant_tool(&state, "read_ttz_chat", &json!({ "chat_id": "1", "limit": 50 })).await;
        assert!(by_number["messages"].is_array(), "{by_number}");
        assert!(assistant_tool(&state, "read_ttz_chat", &json!({ "chat_id": 1 })).await["messages"].is_array(), "a number that isn't a string");
        let by_subject = assistant_tool(&state, "read_ttz_chat", &json!({ "chat_id": "both clients" })).await;
        assert_eq!(by_subject["chat"], "Both clients");
        let wrong = assistant_tool(&state, "read_ttz_chat", &json!({ "chat_id": "99" })).await;
        let why = wrong["error"].as_str().unwrap();
        assert!(why.contains("No chat matches") && why.contains("Both clients") && !why.contains("browser has its key"), "{why}");

        let count = sql(&format!("SELECT COUNT(*) FROM messages WHERE ticket_id = '{}'", shared.id));
        let drafted = assistant_tool(&state, "draft_ttz_message", &json!({ "chat_id": shared.id, "text": "Yes, it works." })).await;
        assert_eq!((drafted["ttzDraft"]["action"].as_str(), drafted["ttzDraft"]["text"].as_str()), (Some("send"), Some("Yes, it works.")));
        assert_eq!(sql(&format!("SELECT COUNT(*) FROM messages WHERE ticket_id = '{}'", shared.id)), count, "a draft sends nothing");
        let opened = assistant_tool(&state, "draft_ttz_chat", &json!({ "kind": "bug", "subject": "Crash\non start", "message": "It crashes." })).await;
        assert_eq!(opened["ttzDraft"]["anonymous"], true, "anonymous unless asked otherwise");
        assert_eq!((opened["ttzDraft"]["kind"].as_str(), opened["ttzDraft"]["title"].as_str()), (Some("GENERAL"), Some("Crash on start")));
        let theirs = sql(&format!("SELECT MAX(id) FROM messages WHERE ticket_id = '{}' AND sender_type = 'ADMIN'", shared.id)).parse::<i64>().unwrap();
        let mine = sql(&format!("SELECT MIN(id) FROM messages WHERE ticket_id = '{}' AND sender_type = 'VISITOR'", shared.id)).parse::<i64>().unwrap();
        for tool in ["draft_ttz_edit", "draft_ttz_delete"] {
            let no = assistant_tool(&state, tool, &json!({ "chat_id": shared.id, "message_id": theirs, "text": "forged" })).await;
            assert!(no["error"].as_str().unwrap().contains("TheHolyOneZ's message"), "{no}");
        }
        assert_eq!(assistant_tool(&state, "draft_ttz_edit", &json!({ "chat_id": shared.id, "message_id": mine, "text": "Hi" })).await["ttzDraft"]["action"], "edit");
        assert!(assistant_tool(&state, "draft_ttz_message", &json!({ "chat_id": "cnothere", "text": "x" })).await["error"].is_string());
        assert!(assistant_tool(&state, "read_ttz_chat", &json!({ "chat_id": "cnothere" })).await["error"].is_string());

        let pad = [0u8; 16];
        let up = |limit: Option<u64>| {
            let sql = &sql;
            let id = shared.id.clone();
            move || sql(&format!("UPDATE tickets SET storage_limit = {} WHERE id = '{id}'", limit.map_or("NULL".to_string(), |l| l.to_string())))
        };
        up(Some(1_048_576))();
        let used = sql(&format!("SELECT COALESCE(SUM(size), 0) FROM attachments WHERE ticket_id = '{}'", shared.id)).parse::<u64>().unwrap();
        assert!(used > 1_048_576);
        let shared_access = load(&state).chat(&shared.id).unwrap().access.clone().unwrap();
        let shared_auth = Auth::Access(&shared_access);
        let full = api.call(false, "file_begin", &[], Some(json!({ "ticket_id": shared.id, "size": pad.len() })), shared_auth).await;
        assert!(matches!(full, Err(ApiError::Other(m)) if m.contains("limit")), "over its limit");
        let raise = |cookie: &str, bytes: Value| client.patch(format!("{base}/api/tickets.php?id={}", shared.id)).header("cookie", cookie.to_string()).header("origin", &origin).json(&json!({ "storage_limit": bytes })).send();
        assert_eq!(raise(&anon, json!(1_073_741_824u64)).await.unwrap().status(), 403, "a visitor can't raise it");
        assert_eq!(raise(&thez, json!(5)).await.unwrap().status(), 400, "nonsense is refused");
        assert_eq!(raise(&thez, json!(1_073_741_824u64)).await.unwrap().status(), 200, "TheHolyOneZ can");
        assert!(api.call(false, "file_begin", &[], Some(json!({ "ticket_id": shared.id, "size": pad.len() })), shared_auth).await.is_ok());
        up(None)();
        let storage: Value = client.get(format!("{base}/api/admin.php?action=storage")).header("cookie", &thez).send().await.unwrap().json().await.unwrap();
        assert_eq!(storage["data"]["chat_limit"], 524_288_000u64);
        let set = |cookie: String, chat: u64| client.post(format!("{base}/api/admin.php?action=storage")).header("cookie", cookie).header("origin", &origin).json(&json!({ "chat_limit": chat, "total_limit": 2_147_483_648u64 })).send();
        assert_eq!(set(thez.clone(), 1).await.unwrap().status(), 400);
        assert_eq!(set(anon.clone(), 2_097_152).await.unwrap().status(), 403, "the defaults are TheHolyOneZ's too");
        assert_eq!(set(thez.clone(), 2_097_152).await.unwrap().status(), 200);
        let storage: Value = client.get(format!("{base}/api/admin.php?action=storage")).header("cookie", &thez).send().await.unwrap().json().await.unwrap();
        assert_eq!((storage["data"]["chat_limit"].as_u64(), storage["data"]["total_limit"].as_u64()), (Some(2_097_152), Some(2_147_483_648)));
        sql("DELETE FROM site_settings");

        assert!(stored(&web_file).exists());
        let removed = client.delete(format!("{base}/api/tickets.php?id={ticket}")).header("cookie", &thez).header("origin", &origin).send().await.unwrap();
        assert_eq!(removed.status(), 200);
        assert!(!stored(&web_file).exists());
        assert_eq!(sql(&format!("SELECT COUNT(*) FROM attachments WHERE ticket_id = '{ticket}'")), "0");

        sql(&format!("DELETE FROM users WHERE id IN ('adm{tag}', 'spoof{tag}', 'goog{tag}')"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn hex_sha(s: &str) -> String {
        hex_bytes(s.as_bytes())
    }

    fn hex_bytes(b: &[u8]) -> String {
        Sha256::digest(b).iter().map(|b| format!("{b:02x}")).collect()
    }

    const TWEETNACL_VISITOR_PUB: &str = "pOCSkrZRwni5dyxWn1+puxPZBrRqtoyd+dwrRAn4ogk=";
    const TWEETNACL_BOX: &str = "EtuVfcXqAuGbC0zdDt7aV2Dt9j+TQeM6GywHV+CeNhXe5Ps7Q7hv";
    const TWEETNACL_REPLY: &str = "3t5S5iHZ9WlmnOcxDAaCM3rhpgeCBN9olsGzI0tfrw==";
}
