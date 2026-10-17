//! Dropbox: sign in once (OAuth with PKCE), then browse and search your
//! files. Gamp's own Dropbox app is built in, so people just click Connect;
//! a key pasted into Settings (your own app) takes its place.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};

use super::*;

const API: &str = "https://api.dropboxapi.com/2";

struct Pending {
    verifier: String,
    redirect: String,
    at: Instant,
}

fn pending() -> &'static Mutex<HashMap<String, Pending>> {
    static P: OnceLock<Mutex<HashMap<String, Pending>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

/// Short-lived access tokens and temporary file links, kept in memory.
fn tokens() -> &'static AsyncMutex<HashMap<String, (String, Instant)>> {
    static T: OnceLock<AsyncMutex<HashMap<String, (String, Instant)>>> = OnceLock::new();
    T.get_or_init(Default::default)
}

fn links() -> &'static Mutex<HashMap<String, (String, Instant)>> {
    static L: OnceLock<Mutex<HashMap<String, (String, Instant)>>> = OnceLock::new();
    L.get_or_init(Default::default)
}

/// Gamp's Dropbox app (dropbox.com/developers/apps: Scoped access, Full
/// Dropbox, files.metadata.read + files.content.read, redirect URI
/// http://127.0.0.1:8742/callback/dropbox). PKCE, so there's no secret.
/// Empty until that app is registered: Settings then asks for a key.
pub const APP_KEY: &str = "";

async fn app_key() -> Result<String, ApiError> {
    let key = Some(store::config().await.dropbox_key).filter(|k| !k.is_empty()).unwrap_or_else(|| APP_KEY.to_owned());
    if key.is_empty() {
        return Err(bad_gateway("Paste your Dropbox app key in Settings first"));
    }
    Ok(key)
}

pub async fn login_url(redirect: &str) -> Result<String, ApiError> {
    let key = app_key().await?;
    let verifier = random_hex(32);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_hex(8);
    pending().lock().unwrap().insert(state.clone(), Pending { verifier, redirect: redirect.to_owned(), at: Instant::now() });
    let mut url = url::Url::parse("https://www.dropbox.com/oauth2/authorize").expect("static");
    url.query_pairs_mut()
        .append_pair("client_id", &key)
        .append_pair("response_type", "code")
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("token_access_type", "offline")
        .append_pair("redirect_uri", redirect)
        .append_pair("state", &state);
    Ok(url.into())
}

async fn token(params: &[(&str, &str)]) -> Result<Value, ApiError> {
    let res = client().post("https://api.dropboxapi.com/oauth2/token").form(params).send().await.map_err(|e| unreachable("Dropbox", e))?;
    let status = res.status();
    let body: Value = res.json().await.unwrap_or_default();
    if !status.is_success() {
        let why = body["error_description"].as_str().or(body["error"].as_str()).unwrap_or("unknown error");
        return Err(bad_gateway(format!("Dropbox sign-in failed: {why}")));
    }
    Ok(body)
}

pub async fn finish_login(code: &str, state: &str) -> Result<(), ApiError> {
    let p = pending().lock().unwrap().remove(state);
    let Some(p) = p.filter(|p| p.at.elapsed() < Duration::from_secs(900)) else {
        return Err(bad_gateway("This Dropbox sign-in expired. Try again."));
    };
    let key = app_key().await?;
    let body = token(&[("code", code), ("grant_type", "authorization_code"), ("code_verifier", &p.verifier), ("client_id", &key), ("redirect_uri", &p.redirect)]).await?;
    let access = s(&body["access_token"]);
    let refresh = s(&body["refresh_token"]);
    let account_id = s(&body["account_id"]);
    let me: Value = client()
        .post(format!("{API}/users/get_current_account"))
        .bearer_auth(&access)
        .send()
        .await
        .map_err(|e| unreachable("Dropbox", e))?
        .json()
        .await
        .unwrap_or_default();
    let existing = accounts().await;
    let id = existing.iter().find(|a| a.kind == Kind::Dropbox && a.user_id == account_id).map(|a| a.id.clone()).unwrap_or_else(|| format!("dropbox-{}", random_hex(4)));
    let name = Some(s(&me["name"]["display_name"])).filter(|n| !n.is_empty()).map(|n| format!("Dropbox ({n})")).unwrap_or_else(|| "Dropbox".into());
    let acc = Account { id: id.clone(), kind: Kind::Dropbox, name, username: s(&me["email"]), token: refresh, user_id: account_id, ..Default::default() };
    tokens().lock().await.insert(id, (access, Instant::now() + Duration::from_secs(body["expires_in"].as_u64().unwrap_or(14400).saturating_sub(300))));
    save_account(acc).await
}

async fn access(acc: &Account) -> Result<String, ApiError> {
    let mut all = tokens().lock().await;
    if let Some((t, until)) = all.get(&acc.id) {
        if Instant::now() < *until {
            return Ok(t.clone());
        }
    }
    let key = app_key().await?;
    let body = token(&[("grant_type", "refresh_token"), ("refresh_token", &acc.token), ("client_id", &key)]).await?;
    let t = s(&body["access_token"]);
    all.insert(acc.id.clone(), (t.clone(), Instant::now() + Duration::from_secs(body["expires_in"].as_u64().unwrap_or(14400).saturating_sub(300))));
    Ok(t)
}

async fn rpc(acc: &Account, endpoint: &str, arg: Value) -> Result<Value, ApiError> {
    let res = client()
        .post(format!("{API}/{endpoint}"))
        .bearer_auth(access(acc).await?)
        .json(&arg)
        .send()
        .await
        .map_err(|e| unreachable("Dropbox", e))?;
    let status = res.status();
    let body: Value = res.json().await.unwrap_or_default();
    if status.is_success() {
        return Ok(body);
    }
    let summary = s(&body["error_summary"]);
    Err(if summary.contains("not_found") {
        not_found("That folder")
    } else if summary.contains("missing_scope") {
        bad_gateway("Your Dropbox app needs the files.metadata.read and files.content.read permissions (Permissions tab), then connect again")
    } else {
        bad_gateway(format!("Dropbox: {}", if summary.is_empty() { status.to_string() } else { summary }))
    })
}

fn entry(e: &Value) -> Option<Item> {
    let name = s(&e["name"]);
    let path = s(&e["path_lower"]);
    match e[".tag"].as_str()? {
        "folder" => Some(Item::folder(format!("dir:{path}"), name)),
        "file" if is_audio(&name) => {
            let display = s(&e["path_display"]);
            let folder = display.rsplit('/').nth(1).unwrap_or("").to_owned();
            Some(Item::track(path, stem(&name)).on(folder).info(display))
        }
        _ => None,
    }
}

pub async fn browse(acc: &Account, node: &str) -> Result<Listing, ApiError> {
    let path = match node.strip_prefix("dir:") {
        Some(p) => p.to_owned(),
        None if node == "files" => String::new(),
        None => return Err(not_found("That list")),
    };
    let mut r = rpc(acc, "files/list_folder", json!({ "path": path, "limit": 2000 })).await?;
    let mut items: Vec<Item> = arr(&r["entries"]).iter().filter_map(entry).collect();
    while r["has_more"] == true && items.len() < 20_000 {
        r = rpc(acc, "files/list_folder/continue", json!({ "cursor": r["cursor"] })).await?;
        items.extend(arr(&r["entries"]).iter().filter_map(entry));
    }
    items.sort_by(|a, b| (a.kind != "folder").cmp(&(b.kind != "folder")).then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase())));
    Ok(Listing::of(items).hint("No folders or music files here."))
}

pub async fn search(acc: &Account, q: &str) -> Result<Listing, ApiError> {
    let r = rpc(acc, "files/search_v2", json!({ "query": q, "options": { "max_results": 500, "file_categories": ["audio", "folder"] } })).await?;
    Ok(Listing::of(arr(&r["matches"]).iter().filter_map(|m| entry(&m["metadata"]["metadata"])).collect()))
}

pub async fn media(acc: &Account, path: &str) -> Result<Upstream, ApiError> {
    let cached = links().lock().unwrap().get(path).filter(|(_, at)| at.elapsed() < Duration::from_secs(3 * 3600)).map(|(l, _)| l.clone());
    let link = match cached {
        Some(l) => l,
        None => {
            let r = rpc(acc, "files/get_temporary_link", json!({ "path": path })).await?;
            let l = s(&r["link"]);
            links().lock().unwrap().insert(path.to_owned(), (l.clone(), Instant::now()));
            l
        }
    };
    Ok(Upstream::get(link))
}
