//! SoundCloud: OAuth 2.1 with PKCE, the account's likes, playlists and
//! search, and a stream proxy.
//!
//! The API hands out HLS only. Webamp plays an <audio> element wired into
//! Web Audio, so the equalizer and every visualizer see the signal, and that
//! wants one plain, seekable file. The MP3 rendition's segments are MP3
//! frames behind a per-segment ID3 tag, so the proxy measures each segment,
//! drops the tag, and serves the rest as one virtual MP3 with byte ranges.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures_util::StreamExt;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as AsyncMutex;

use crate::http::{ApiError, client, err, parse_range};
use crate::store;

const API_DEFAULT: &str = "https://api.soundcloud.com";

/// Debug builds can point at a stand-in API for tests.
fn api_base() -> String {
    if cfg!(debug_assertions) {
        if let Ok(base) = std::env::var("WINAMP_SOUNDCLOUD_API") {
            return base;
        }
    }
    API_DEFAULT.to_owned()
}
const AUTH: &str = "https://secure.soundcloud.com";
const TOKENS: &str = "soundcloud-tokens.json";

#[derive(Clone, Default, Serialize, Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: String,
    expires_at: u64,
}

struct Pending {
    verifier: String,
    redirect_uri: String,
    at: Instant,
}

fn pending() -> &'static Mutex<HashMap<String, Pending>> {
    static P: OnceLock<Mutex<HashMap<String, Pending>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

fn token_lock() -> &'static AsyncMutex<()> {
    static L: OnceLock<AsyncMutex<()>> = OnceLock::new();
    L.get_or_init(|| AsyncMutex::new(()))
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn random(n: usize) -> String {
    let mut buf = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

pub async fn status() -> (bool, bool) {
    let keys = store::config().await.soundcloud;
    let tokens = store::read_json::<Tokens>(TOKENS).await.unwrap_or_default();
    (!keys.client_id.is_empty() && !keys.client_secret.is_empty(), !tokens.refresh_token.is_empty())
}

pub async fn login_url(redirect_uri: &str) -> Result<String, ApiError> {
    let keys = store::config().await.soundcloud;
    if keys.client_id.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "Add your SoundCloud client ID and secret in Settings first."));
    }
    let verifier = random(48);
    let state = random(16);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    pending()
        .lock()
        .unwrap()
        .insert(state.clone(), Pending { verifier, redirect_uri: redirect_uri.to_owned(), at: Instant::now() });
    let mut url = url::Url::parse(&format!("{AUTH}/authorize"))?;
    url.query_pairs_mut()
        .append_pair("client_id", &keys.client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state);
    Ok(url.into())
}

async fn token_request(params: &[(&str, &str)]) -> Result<(), ApiError> {
    let res = client()
        .post(format!("{AUTH}/oauth/token"))
        .header(header::ACCEPT, "application/json; charset=utf-8")
        .form(params)
        .send()
        .await?;
    let status = res.status();
    let body: Value = res.json().await.unwrap_or_default();
    if !status.is_success() {
        let why = body["error_description"].as_str().or(body["error"].as_str()).unwrap_or("unknown error");
        return Err(err(StatusCode::UNAUTHORIZED, format!("SoundCloud sign-in failed ({status}): {why}")));
    }
    let tokens = Tokens {
        access_token: body["access_token"].as_str().unwrap_or_default().to_owned(),
        refresh_token: body["refresh_token"].as_str().unwrap_or_default().to_owned(),
        expires_at: now_ms() + body["expires_in"].as_u64().unwrap_or(3600) * 1000,
    };
    store::write_json(TOKENS, &tokens, true).await?;
    Ok(())
}

pub async fn finish_login(code: &str, state: &str) -> Result<(), ApiError> {
    let p = pending().lock().unwrap().remove(state);
    let Some(p) = p.filter(|p| p.at.elapsed() < Duration::from_secs(600)) else {
        return Err(err(StatusCode::BAD_REQUEST, "This SoundCloud sign-in expired. Try again."));
    };
    let keys = store::config().await.soundcloud;
    token_request(&[
        ("grant_type", "authorization_code"),
        ("client_id", &keys.client_id),
        ("client_secret", &keys.client_secret),
        ("redirect_uri", &p.redirect_uri),
        ("code_verifier", &p.verifier),
        ("code", code),
    ])
    .await
}

pub async fn logout() -> anyhow::Result<()> {
    store::write_json(TOKENS, &Tokens::default(), true).await
}

/// Refresh tokens are single use, so refreshes are serialised.
async fn access_token(force: bool) -> Result<String, ApiError> {
    let _guard = token_lock().lock().await;
    let t = store::read_json::<Tokens>(TOKENS).await.unwrap_or_default();
    if t.refresh_token.is_empty() {
        return Err(err(StatusCode::UNAUTHORIZED, "Connect SoundCloud in Settings."));
    }
    if !force && !t.access_token.is_empty() && now_ms() + 60_000 < t.expires_at {
        return Ok(t.access_token);
    }
    let keys = store::config().await.soundcloud;
    token_request(&[
        ("grant_type", "refresh_token"),
        ("client_id", &keys.client_id),
        ("client_secret", &keys.client_secret),
        ("refresh_token", &t.refresh_token),
    ])
    .await?;
    Ok(store::read_json::<Tokens>(TOKENS).await.unwrap_or_default().access_token)
}

async fn api(path_or_url: &str) -> Result<Value, ApiError> {
    let url = if path_or_url.starts_with("http") { path_or_url.to_owned() } else { format!("{}{path_or_url}", api_base()) };
    for attempt in 0..2 {
        let token = access_token(attempt == 1).await?;
        let res = client()
            .get(&url)
            .header(header::AUTHORIZATION, format!("OAuth {token}"))
            .header(header::ACCEPT, "application/json; charset=utf-8")
            .send()
            .await?;
        if res.status() == StatusCode::UNAUTHORIZED && attempt == 0 {
            continue;
        }
        if !res.status().is_success() {
            let path = url::Url::parse(&url).map(|u| u.path().to_owned()).unwrap_or_default();
            return Err(err(res.status(), format!("SoundCloud answered {} for {path}", res.status())));
        }
        return Ok(res.json().await?);
    }
    Err(err(StatusCode::UNAUTHORIZED, "SoundCloud rejected the sign-in. Connect it again in Settings."))
}

/// Follows linked_partitioning pages up to `max` items.
async fn collect(path: &str, max: usize) -> Result<Vec<Value>, ApiError> {
    let sep = if path.contains('?') { '&' } else { '?' };
    let mut next = Some(format!("{path}{sep}linked_partitioning=true&limit=200"));
    let mut out = Vec::new();
    while let Some(url) = next.take() {
        if out.len() >= max {
            break;
        }
        let page = api(&url).await?;
        if let Some(items) = page["collection"].as_array() {
            out.extend(items.iter().cloned());
        }
        next = page["next_href"].as_str().map(str::to_owned);
    }
    out.truncate(max);
    Ok(out)
}

fn to_track(t: &Value) -> Option<Value> {
    if t["kind"] == "playlist" || t["access"] == "blocked" {
        return None;
    }
    let urn = t["urn"].as_str()?;
    let artist = t["metadata_artist"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or(t["user"]["username"].as_str())
        .unwrap_or("");
    Some(serde_json::json!({
        "source": "soundcloud",
        "id": urn,
        "title": t["title"],
        "artist": artist,
        "album": t["release"].as_str().unwrap_or(""),
        "duration": t["duration"].as_u64().map(|ms| (ms + 500) / 1000),
        "artwork": t["artwork_url"],
        "preview": t["access"] == "preview",
    }))
}

fn tracks(items: Vec<Value>) -> Vec<Value> {
    items.iter().filter_map(to_track).collect()
}

pub async fn likes() -> Result<Vec<Value>, ApiError> {
    Ok(tracks(collect("/me/likes/tracks?access=playable,preview", 2000).await?))
}

pub async fn playlists() -> Result<Vec<Value>, ApiError> {
    Ok(collect("/me/playlists?show_tracks=false", 2000)
        .await?
        .iter()
        .map(|p| serde_json::json!({ "id": p["urn"], "title": p["title"], "count": p["track_count"] }))
        .collect())
}

pub async fn playlist_tracks(urn: &str) -> Result<Vec<Value>, ApiError> {
    let path = format!("/playlists/{}/tracks?access=playable,preview", urlencoding::encode(urn));
    Ok(tracks(collect(&path, 5000).await?))
}

pub async fn search(q: &str) -> Result<Vec<Value>, ApiError> {
    let path = format!("/tracks?q={}&access=playable,preview", urlencoding::encode(q));
    Ok(tracks(collect(&path, 200).await?))
}

// ---- the stream proxy --------------------------------------------------

#[derive(Clone)]
struct Segment {
    url: String,
    skip: u64,
    start: u64,
    len: u64,
}

#[derive(Clone)]
struct Plan {
    segments: Vec<Segment>,
    size: u64,
    at: Instant,
}

/// Segment URLs are signed and expire.
const PLAN_TTL: Duration = Duration::from_secs(240);

fn plans() -> &'static AsyncMutex<HashMap<String, Plan>> {
    static P: OnceLock<AsyncMutex<HashMap<String, Plan>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

pub fn id3_len(head: &[u8]) -> u64 {
    if head.len() < 10 || &head[..3] != b"ID3" {
        return 0;
    }
    let size = ((head[6] as u64 & 0x7f) << 21) | ((head[7] as u64 & 0x7f) << 14) | ((head[8] as u64 & 0x7f) << 7) | (head[9] as u64 & 0x7f);
    let footer = if head[5] & 0x10 != 0 { 10 } else { 0 };
    10 + size + footer
}

async fn measure(url: String) -> Result<(String, u64, u64), ApiError> {
    let res = client().get(&url).header(header::RANGE, "bytes=0-9").send().await?;
    if !res.status().is_success() {
        return Err(err(StatusCode::BAD_GATEWAY, format!("segment answered {}", res.status())));
    }
    let partial = res.status() == StatusCode::PARTIAL_CONTENT;
    let total_from_range = res
        .headers()
        .get(header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit('/').next())
        .and_then(|v| v.parse::<u64>().ok());
    let head = res.bytes().await?;
    let total = if partial { total_from_range.unwrap_or(head.len() as u64) } else { head.len() as u64 };
    Ok((url, id3_len(&head), total))
}

/// Pulls the segment URIs out of a media playlist.
pub fn segment_uris(m3u8_url: &str, body: &str) -> Vec<String> {
    let base = url::Url::parse(m3u8_url).ok();
    body.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| match &base {
            Some(b) => b.join(l).ok().map(String::from),
            None => Some(l.to_owned()),
        })
        .collect()
}

async fn build_plan(urn: &str) -> Result<Plan, ApiError> {
    let streams = api(&format!("/tracks/{}/streams", urlencoding::encode(urn))).await?;
    let m3u8 = streams["hls_mp3_128_url"]
        .as_str()
        .or(streams["hls_mp3_standard_url"].as_str())
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "SoundCloud has no MP3 stream for this track."))?
        .to_owned();
    let list = client().get(&m3u8).send().await?.text().await?;
    let uris = segment_uris(&m3u8, &list);
    let measured: Vec<Result<(String, u64, u64), ApiError>> =
        futures_util::stream::iter(uris.into_iter().map(measure)).buffered(24).collect().await;
    let mut segments = Vec::with_capacity(measured.len());
    let mut offset = 0;
    for m in measured {
        let (url, skip, total) = m?;
        let len = total.saturating_sub(skip);
        segments.push(Segment { url, skip, start: offset, len });
        offset += len;
    }
    Ok(Plan { segments, size: offset, at: Instant::now() })
}

async fn plan(urn: &str, fresh: bool) -> Result<Plan, ApiError> {
    if !fresh {
        if let Some(p) = plans().lock().await.get(urn).filter(|p| p.at.elapsed() < PLAN_TTL) {
            return Ok(p.clone());
        }
    }
    let p = build_plan(urn).await?;
    plans().lock().await.insert(urn.to_owned(), p.clone());
    Ok(p)
}

pub async fn stream(urn: String, headers: HeaderMap) -> Result<Response, ApiError> {
    let p = plan(&urn, false).await?;
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let (start, end, partial) = match parse_range(range, p.size) {
        Ok(Some((s, e))) => (s, e, true),
        Ok(None) => (0, p.size.saturating_sub(1), false),
        Err(()) => {
            return Ok(Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{}", p.size))
                .body(Body::empty())?);
        }
    };
    let pieces: Vec<(usize, u64, u64)> = p
        .segments
        .iter()
        .enumerate()
        .filter(|(_, s)| s.len > 0 && s.start + s.len > start && s.start <= end)
        .map(|(n, s)| (n, s.skip + start.saturating_sub(s.start), s.skip + (s.len - 1).min(end - s.start)))
        .collect();
    let size = p.size;
    let body = async_stream(urn, p, pieces);
    let mut res = Response::builder()
        .status(if partial { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK })
        .header(header::CONTENT_TYPE, "audio/mpeg")
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, end - start + 1)
        .header(header::CACHE_CONTROL, "no-store");
    if partial {
        res = res.header(header::CONTENT_RANGE, HeaderValue::from_str(&format!("bytes {start}-{end}/{size}"))?);
    }
    Ok(res.body(body)?)
}

fn async_stream(urn: String, plan: Plan, pieces: Vec<(usize, u64, u64)>) -> Body {
    let stream = futures_util::stream::unfold((urn, plan, pieces.into_iter()), |(urn, mut plan, mut pieces)| async move {
        let (n, from, to) = pieces.next()?;
        let fetch = |url: String| async move {
            client().get(url).header(header::RANGE, format!("bytes={from}-{to}")).send().await
        };
        let mut res = fetch(plan.segments[n].url.clone()).await.ok()?;
        if matches!(res.status(), StatusCode::FORBIDDEN | StatusCode::GONE) {
            // The signed URL expired mid-play: measure again and go on.
            plan = crate::soundcloud::plan(&urn, true).await.ok()?;
            res = fetch(plan.segments[n].url.clone()).await.ok()?;
        }
        if !res.status().is_success() {
            return None;
        }
        let whole = res.status() == StatusCode::OK;
        let mut bytes = res.bytes().await.ok()?;
        if whole {
            bytes = bytes.slice(from as usize..(to as usize + 1).min(bytes.len()));
        }
        Some((Ok::<_, std::io::Error>(bytes), (urn, plan, pieces)))
    });
    Body::from_stream(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id3_lengths() {
        assert_eq!(id3_len(b"\xff\xfb\x90\x00........"), 0);
        // size 0x000201 synchsafe = 257, plus the 10-byte header
        assert_eq!(id3_len(b"ID3\x04\x00\x00\x00\x00\x02\x01"), 10 + 257);
        assert_eq!(id3_len(b"ID3\x04\x00\x10\x00\x00\x00\x05"), 10 + 5 + 10);
    }

    #[test]
    fn playlist_uris_resolve() {
        let body = "#EXTM3U\n#EXTINF:5,\nseg0.mp3\n#EXTINF:5,\nhttps://cdn.example/x/seg1.mp3\n#EXT-X-ENDLIST\n";
        assert_eq!(
            segment_uris("https://cdn.example/a/list.m3u8?sig=1", body),
            vec!["https://cdn.example/a/seg0.mp3".to_owned(), "https://cdn.example/x/seg1.mp3".to_owned()]
        );
    }
}
