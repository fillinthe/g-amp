//! SoundCloud by link, no account or API keys: a pasted soundcloud.com
//! address (track, playlist/set, or artist) is resolved the way the
//! SoundCloud web player does it, and played through the same seekable
//! MP3 stream as connected SoundCloud. Unofficial: if SoundCloud changes
//! its web player, this needs updating.

use std::sync::OnceLock;

use axum::http::StatusCode;
use serde_json::Value;
use tokio::sync::Mutex as AsyncMutex;

use crate::http::{ApiError, client, err};
use crate::sources::{Item, arr, f, s};

const API: &str = "https://api-v2.soundcloud.com";
const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";

fn id_cell() -> &'static AsyncMutex<Option<String>> {
    static C: OnceLock<AsyncMutex<Option<String>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

pub fn is_link(q: &str) -> bool {
    let q = q.trim().to_ascii_lowercase();
    (q.starts_with("http://") || q.starts_with("https://")) && (q.contains("soundcloud.com/") || q.contains("snd.sc/"))
}

/// The web player's public client id, read from its scripts.
async fn find_client_id() -> Result<String, ApiError> {
    let fail = |e: String| err(StatusCode::BAD_GATEWAY, format!("Can't reach SoundCloud: {e}"));
    let page = client().get("https://soundcloud.com/").header("User-Agent", UA).send().await.map_err(|e| fail(e.to_string()))?.text().await.map_err(|e| fail(e.to_string()))?;
    let mut scripts: Vec<String> = page
        .split("<script")
        .filter_map(|tag| {
            let src = tag.split("src=\"").nth(1)?.split('"').next()?;
            src.contains("sndcdn.com").then(|| src.to_owned())
        })
        .collect();
    // The id lives in one of the last scripts.
    scripts.reverse();
    for src in scripts.iter().take(8) {
        let Ok(r) = client().get(src).header("User-Agent", UA).send().await else { continue };
        let Ok(js) = r.text().await else { continue };
        if let Some(id) = extract_client_id(&js) {
            return Ok(id);
        }
    }
    Err(err(StatusCode::BAD_GATEWAY, "SoundCloud changed its web player; Gamp needs an update to play links"))
}

pub fn extract_client_id(js: &str) -> Option<String> {
    for key in ["client_id:\"", "client_id=\"", "client_id=", "\"client_id\":\""] {
        let mut rest = js;
        while let Some(i) = rest.find(key) {
            let tail = &rest[i + key.len()..];
            let id: String = tail.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
            if id.len() == 32 {
                return Some(id);
            }
            rest = &tail[1.min(tail.len())..];
        }
    }
    None
}

async fn client_id(fresh: bool) -> Result<String, ApiError> {
    let mut cell = id_cell().lock().await;
    if !fresh {
        if let Some(id) = cell.as_ref() {
            return Ok(id.clone());
        }
    }
    let id = find_client_id().await?;
    *cell = Some(id.clone());
    Ok(id)
}

/// GET on the web player's API; a stale client id is refreshed once.
async fn get(url: &str) -> Result<Value, ApiError> {
    for attempt in 0..2 {
        let id = client_id(attempt > 0).await?;
        let sep = if url.contains('?') { '&' } else { '?' };
        let full = if url.starts_with("http") { format!("{url}{sep}client_id={id}") } else { format!("{API}{url}{sep}client_id={id}") };
        let res = client().get(&full).header("User-Agent", UA).send().await.map_err(|e| err(StatusCode::BAD_GATEWAY, format!("SoundCloud: {e}")))?;
        match res.status() {
            s if s.is_success() => return res.json().await.map_err(|e| err(StatusCode::BAD_GATEWAY, format!("SoundCloud: {e}"))),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN if attempt == 0 => continue,
            StatusCode::NOT_FOUND => return Err(err(StatusCode::NOT_FOUND, "SoundCloud doesn't have that (or it's private)")),
            s => return Err(err(StatusCode::BAD_GATEWAY, format!("SoundCloud answered {s}"))),
        }
    }
    Err(err(StatusCode::BAD_GATEWAY, "SoundCloud refused"))
}

pub fn track_item(t: &Value) -> Option<Item> {
    let id = t["id"].as_u64()?;
    let title = s(&t["title"]);
    if title.is_empty() {
        return None;
    }
    let artist = [s(&t["publisher_metadata"]["artist"]), s(&t["user"]["username"])].into_iter().find(|a| !a.is_empty()).unwrap_or_default();
    let mut i = Item::track(format!("web:{id}"), title).by(artist).secs(f(&t["full_duration"]).or(f(&t["duration"])).map(|ms| (ms / 1000.0).round()));
    if t["policy"] == "SNIP" {
        i.info = "SoundCloud only allows a 30-second preview of this track".into();
    }
    i.url = Some(format!("/media/soundcloud/{}.mp3", urlencoding::encode(&i.id)));
    Some(i)
}

/// Fills in playlist entries SoundCloud lists by id only.
async fn complete(tracks: &[Value]) -> Result<Vec<Value>, ApiError> {
    let mut full: Vec<Value> = Vec::new();
    let missing: Vec<u64> = tracks.iter().filter(|t| t["title"].is_null()).filter_map(|t| t["id"].as_u64()).collect();
    let mut fetched = std::collections::HashMap::new();
    for chunk in missing.chunks(50) {
        let ids: Vec<String> = chunk.iter().map(u64::to_string).collect();
        if let Ok(v) = get(&format!("/tracks?ids={}", ids.join(","))).await {
            for t in arr(&v) {
                if let Some(id) = t["id"].as_u64() {
                    fetched.insert(id, t.clone());
                }
            }
        }
    }
    for t in tracks {
        let id = t["id"].as_u64().unwrap_or(0);
        full.push(fetched.remove(&id).unwrap_or_else(|| t.clone()));
    }
    Ok(full)
}

/// A pasted link: its track(s).
pub async fn resolve(link: &str) -> Result<Vec<Item>, ApiError> {
    let mut link = link.trim().to_owned();
    // Short links (on.soundcloud.com) redirect to the real page.
    if link.contains("on.soundcloud.com") || link.contains("snd.sc") {
        if let Ok(r) = client().get(&link).header("User-Agent", UA).send().await {
            link = r.url().to_string();
        }
    }
    let v = get(&format!("/resolve?url={}", urlencoding::encode(link.split('?').next().unwrap_or(&link)))).await?;
    let tracks: Vec<Value> = match v["kind"].as_str() {
        Some("track") => vec![v.clone()],
        Some("playlist" | "system-playlist") => complete(arr(&v["tracks"])).await?,
        Some("user") => {
            let id = v["id"].as_u64().unwrap_or(0);
            arr(&get(&format!("/users/{id}/tracks?limit=200")).await?["collection"]).to_vec()
        }
        _ => return Err(err(StatusCode::NOT_FOUND, "That link isn't a SoundCloud track, playlist or artist")),
    };
    Ok(tracks.iter().filter_map(track_item).collect())
}

/// Up to `max` entries of a paged list, following `next_href`.
async fn pages(first: String, max: usize) -> Result<Vec<Value>, ApiError> {
    let mut out = Vec::new();
    let mut next = Some(first);
    while let Some(url) = next.take() {
        let v = get(&url).await?;
        out.extend(arr(&v["collection"]).iter().cloned());
        if out.len() < max {
            next = v["next_href"].as_str().map(str::to_owned);
        }
    }
    Ok(out)
}

/// A profile link or bare user name ("soundcloud.com/you", "you") → its user id.
pub async fn user_id(profile: &str) -> Result<u64, ApiError> {
    let p = profile.trim().trim_end_matches('/');
    let link = if p.contains("soundcloud.com/") { p.to_owned() } else { format!("https://soundcloud.com/{p}") };
    let v = get(&format!("/resolve?url={}", urlencoding::encode(&link))).await?;
    match (v["kind"].as_str(), v["id"].as_u64()) {
        (Some("user"), Some(id)) => Ok(id),
        _ => Err(err(StatusCode::NOT_FOUND, "That isn't a SoundCloud profile. Use your profile page's address, like soundcloud.com/you")),
    }
}

/// A profile's public likes.
pub async fn likes(user: u64) -> Result<Vec<Item>, ApiError> {
    let likes = pages(format!("/users/{user}/track_likes?limit=200"), 2000).await?;
    Ok(likes.iter().filter_map(|l| track_item(&l["track"])).collect())
}

/// A profile's public playlists and albums, as folders `playlist:web:<id>`.
pub async fn playlists(user: u64) -> Result<Vec<Item>, ApiError> {
    let lists = pages(format!("/users/{user}/playlists?limit=200"), 2000).await?;
    Ok(lists.iter().filter_map(|p| Some(Item::folder(format!("playlist:web:{}", p["id"].as_u64()?), s(&p["title"])).items(p["track_count"].as_u64()))).collect())
}

pub async fn playlist(id: &str) -> Result<Vec<Item>, ApiError> {
    let v = get(&format!("/playlists/{id}")).await?;
    Ok(complete(arr(&v["tracks"])).await?.iter().filter_map(track_item).collect())
}

pub async fn search(q: &str) -> Result<Vec<Item>, ApiError> {
    let found = pages(format!("/search/tracks?q={}&limit=50", urlencoding::encode(q)), 100).await?;
    Ok(found.iter().filter_map(track_item).collect())
}

/// The MP3 stream for track `id`: its HLS playlist (or a plain MP3).
/// Returns (url, is_hls).
/// A track's stream: `(url, is_hls, mime)`. MP3 when SoundCloud still
/// serves it, otherwise AAC (HLS of fragmented MP4).
pub async fn stream_url(id: &str) -> Result<(String, bool, &'static str), ApiError> {
    let t = get(&format!("/tracks/{id}")).await?;
    let codings = arr(&t["media"]["transcodings"]);
    let kinds: [(&str, &str, &'static str); 3] = [("progressive", "mpeg", "audio/mpeg"), ("hls", "mpeg", "audio/mpeg"), ("hls", "mp4", "audio/mp4")];
    let mut last = err(StatusCode::NOT_FOUND, "SoundCloud has no stream for this track");
    for (proto, kind, mime) in kinds {
        let found = codings.iter().filter(|c| {
            c["format"]["protocol"] == proto && c["format"]["mime_type"].as_str().is_some_and(|m| m.contains(kind))
        });
        for coding in found {
            let mut url = s(&coding["url"]);
            if let Some(auth) = t["track_authorization"].as_str() {
                url = format!("{url}?track_authorization={auth}");
            }
            // Some advertised streams are gone (404): try the next.
            match get(&url).await {
                Ok(v) if !s(&v["url"]).is_empty() => return Ok((s(&v["url"]), proto == "hls", mime)),
                Ok(_) => {}
                Err(e) => last = e,
            }
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds_ids_and_links() {
        assert_eq!(super::extract_client_id(r#"x={client_id:"abcdefghijABCDEFGHIJ0123456789xy",y:1}"#).as_deref(), Some("abcdefghijABCDEFGHIJ0123456789xy"));
        assert_eq!(super::extract_client_id("client_id=short"), None);
        assert!(super::is_link("https://soundcloud.com/artist/track"));
        assert!(super::is_link("https://on.soundcloud.com/AbC12"));
        assert!(!super::is_link("https://example.com/soundcloud"));
    }
}

#[cfg(test)]
mod live {
    /// `cargo test -p gamp-core live_link -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_link() {
        let items = super::resolve("https://soundcloud.com/forss/flickermood").await.unwrap();
        println!("{} - {} ({:?}s) {}", items[0].artist, items[0].title, items[0].duration, items[0].info);
        let id = items[0].id.trim_start_matches("web:");
        let (url, hls, mime) = super::stream_url(id).await.unwrap();
        println!("hls {hls} {mime}: {}", &url[..url.len().min(90)]);
        let set = super::resolve("https://soundcloud.com/forss/sets/soulhack").await.unwrap();
        println!("set: {} tracks, first {}", set.len(), set.first().map(|t| t.title.clone()).unwrap_or_default());
    }
}
