//! Every place music comes from, as one tree for the native library
//! window: the folder library, SoundCloud and Spotify alongside the
//! sources in `sources` (servers, cloud, radio, podcasts, Archive). Each
//! has top-level lists (`roots`), folders that open further lists, and
//! tracks with a `url` on the local server to play them from.

use serde_json::{Value, json};

use crate::http::{ApiError, client, err};
use crate::sources::{self, Item, Listing, arr, f, s, u};
use crate::{library, soundcloud, spotify};
use axum::http::StatusCode;

/// The tree: (id, name, group, roots as (id, title), searchable).
#[derive(Clone, Debug)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub group: String,
    pub roots: Vec<(String, String)>,
    pub search: bool,
    /// Needs connecting in Settings first.
    pub needs_setup: bool,
}

pub async fn sources() -> Vec<Source> {
    let mut out = vec![Source { id: "local".into(), name: "Local Media".into(), group: "local".into(), roots: vec![("audio".into(), "Audio".into())], search: false, needs_setup: false }];
    for v in sources::list().await.as_array().cloned().unwrap_or_default() {
        out.push(Source {
            id: s(&v["id"]),
            name: s(&v["name"]),
            group: s(&v["group"]),
            roots: arr(&v["roots"]).iter().map(|r| (s(&r["id"]), s(&r["title"]))).collect(),
            search: v["search"] == true,
            needs_setup: false,
        });
    }
    let (_, sc) = soundcloud::status().await;
    out.push(Source {
        id: "soundcloud".into(),
        name: "SoundCloud".into(),
        group: "soundcloud".into(),
        roots: if sc { vec![("likes".into(), "Likes".into()), ("playlists".into(), "Playlists".into())] } else { vec![] },
        // Links play without an account, so Search is always there.
        search: true,
        needs_setup: !sc,
    });
    let sp = spotify::connected().await;
    out.push(Source {
        id: "spotify".into(),
        name: "Spotify".into(),
        group: "spotify".into(),
        roots: if sp { vec![("liked".into(), "Liked Songs".into()), ("playlists".into(), "Playlists".into())] } else { vec![] },
        search: sp,
        needs_setup: !sp,
    });
    out
}

fn local_items() -> impl std::future::Future<Output = Vec<Item>> {
    async {
        let mut tracks = library::list().await;
        tracks.sort_by(|a, b| a.artist.to_lowercase().cmp(&b.artist.to_lowercase()).then(a.album.cmp(&b.album)).then(a.track_no.cmp(&b.track_no)));
        tracks
            .into_iter()
            .map(|t| {
                let mut i = Item::track(t.id.clone(), t.title).by(t.artist).on(t.album).secs(t.duration.map(f64::from)).no(t.track_no.map(u64::from));
                i.info = t.path.to_string_lossy().into_owned();
                i.url = Some(format!("/media/local/{}", t.id));
                i
            })
            .collect()
    }
}

fn sc_track(v: &Value) -> Item {
    let id = s(&v["id"]);
    let mut i = Item::track(id.clone(), s(&v["title"])).by(s(&v["artist"])).on(s(&v["album"])).secs(f(&v["duration"]));
    if v["preview"] == true {
        i.info = "SoundCloud only allows a 30-second preview of this track".into();
    }
    i.url = Some(format!("/media/soundcloud/{}.mp3", urlencoding::encode(&id)));
    i
}

// ---- Spotify's library (Web API) ------------------------------------------

async fn sp_api(path: &str) -> Result<Value, ApiError> {
    let url = if path.starts_with("http") { path.to_owned() } else { format!("https://api.spotify.com/v1{path}") };
    let (token, _, own) = spotify::api_token().await?;
    let mut tries = 0;
    let res = loop {
        let res = client().get(&url).bearer_auth(&token).send().await.map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Spotify: {e}")))?;
        let wait = res.headers().get("retry-after").and_then(|v| v.to_str().ok()?.parse::<u64>().ok()).unwrap_or(1);
        if res.status() != StatusCode::TOO_MANY_REQUESTS {
            break res;
        }
        if !own {
            return Err(err(
                StatusCode::TOO_MANY_REQUESTS,
                "Spotify is turning away the shared sign-in. For your Liked Songs, playlists and search, add your own Spotify Client ID in Settings › Spotify.",
            ));
        }
        tries += 1;
        if tries > 2 || wait > 10 {
            return Err(err(StatusCode::TOO_MANY_REQUESTS, format!("Spotify asks to wait {wait} seconds. Try again then.")));
        }
        tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
    };
    let status = res.status();
    let body: Value = res.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = body["error"]["message"].as_str().unwrap_or("error").to_owned();
        return Err(err(status, format!("Spotify: {msg}")));
    }
    Ok(body)
}

async fn sp_pages(path: &str, max: usize) -> Result<Vec<Value>, ApiError> {
    let mut out = Vec::new();
    let mut next = Some(path.to_owned());
    while let Some(p) = next.take() {
        let page = sp_api(&p).await?;
        out.extend(arr(&page["items"]).iter().cloned());
        if out.len() >= max {
            break;
        }
        next = page["next"].as_str().map(str::to_owned);
    }
    Ok(out)
}

fn sp_track(t: &Value) -> Option<Item> {
    let id = t["id"].as_str()?;
    if t["is_local"] == true || t["type"].as_str().is_some_and(|ty| ty != "track") {
        return None;
    }
    let artists: Vec<String> = arr(&t["artists"]).iter().map(|a| s(&a["name"])).collect();
    let mut i = Item::track(id, s(&t["name"])).by(artists.join(", ")).on(s(&t["album"]["name"])).secs(f(&t["duration_ms"]).map(|ms| (ms / 1000.0).round()));
    i.url = Some(format!("/media/spotify/{id}.wav"));
    Some(i)
}

// ---- the common interface ---------------------------------------------------

pub async fn browse(sid: &str, node: &str) -> Result<Listing, ApiError> {
    match sid {
        "local" => Ok(Listing::of(local_items().await).hint("Add folders in Settings › Music folders.")),
        "soundcloud" => {
            let (kind, arg) = node.split_once(':').unwrap_or((node, ""));
            let rows = match kind {
                "likes" => soundcloud::likes().await?.iter().map(sc_track).collect(),
                "playlists" => soundcloud::playlists()
                    .await?
                    .iter()
                    .map(|p| Item::folder(format!("playlist:{}", s(&p["id"])), s(&p["title"])).items(u(&p["count"])))
                    .collect(),
                "playlist" => soundcloud::playlist_tracks(arg).await?.iter().map(sc_track).collect(),
                _ => return Err(sources::not_found("That list")),
            };
            Ok(Listing::of(rows))
        }
        "spotify" => {
            let (kind, arg) = node.split_once(':').unwrap_or((node, ""));
            let rows: Vec<Item> = match kind {
                "liked" => sp_pages("/me/tracks?limit=50", 5000).await?.iter().filter_map(|i| sp_track(&i["track"])).collect(),
                "playlists" => sp_pages("/me/playlists?limit=50", 2000)
                    .await?
                    .iter()
                    .filter(|p| !p.is_null())
                    .map(|p| {
                        let count = u(&p["items"]["total"]).or(u(&p["tracks"]["total"]));
                        Item::folder(format!("playlist:{}", s(&p["id"])), s(&p["name"])).by(s(&p["owner"]["display_name"])).items(count)
                    })
                    .collect(),
                "playlist" => {
                    let items = match sp_pages(&format!("/playlists/{arg}/items?limit=50"), 10000).await {
                        Ok(i) => i,
                        Err(e) if e.0 == StatusCode::NOT_FOUND => sp_pages(&format!("/playlists/{arg}/tracks?limit=50"), 10000).await?,
                        Err(e) => return Err(e),
                    };
                    items.iter().filter_map(|i| sp_track(if i["item"].is_null() { &i["track"] } else { &i["item"] })).collect()
                }
                _ => return Err(sources::not_found("That list")),
            };
            let hint = (kind == "playlist" && rows.is_empty()).then_some("Spotify only shares the songs of playlists you own or collaborate on.");
            let mut l = Listing::of(rows);
            if let Some(h) = hint {
                l = l.hint(h);
            }
            Ok(l)
        }
        _ => sources::browse(sid, node).await,
    }
}

pub async fn search(sid: &str, q: &str) -> Result<Listing, ApiError> {
    // A SoundCloud link plays from any search box, no account needed.
    if crate::soundcloud_web::is_link(q) {
        return Ok(Listing::of(crate::soundcloud_web::resolve(q).await?));
    }
    match sid {
        "soundcloud" => {
            if !soundcloud::status().await.1 {
                return Err(err(StatusCode::BAD_REQUEST, "Paste a SoundCloud link to play it. Searching SoundCloud needs Connect (Settings › SoundCloud)."));
            }
            Ok(Listing::of(soundcloud::search(q).await?.iter().map(sc_track).collect()))
        }
        "spotify" => {
            // Your own playlists whose names match come first.
            let ql = q.to_lowercase();
            let mut rows: Vec<Item> = match sp_pages("/me/playlists?limit=50", 500).await {
                Ok(lists) => lists
                    .iter()
                    .filter(|p| !p.is_null() && s(&p["name"]).to_lowercase().contains(&ql))
                    .map(|p| {
                        let count = u(&p["items"]["total"]).or(u(&p["tracks"]["total"]));
                        Item::folder(format!("playlist:{}", s(&p["id"])), s(&p["name"])).by(s(&p["owner"]["display_name"])).items(count)
                    })
                    .collect(),
                Err(e) if e.0 == StatusCode::TOO_MANY_REQUESTS => return Err(e),
                Err(_) => Vec::new(),
            };
            for offset in (0..50).step_by(10) {
                let r = sp_api(&format!("/search?type=track&limit=10&offset={offset}&q={}", urlencoding::encode(q))).await?;
                rows.extend(arr(&r["tracks"]["items"]).iter().filter_map(sp_track));
                if r["tracks"]["next"].is_null() {
                    break;
                }
            }
            Ok(Listing::of(rows))
        }
        "local" => {
            let q = q.to_lowercase();
            Ok(Listing::of(local_items().await.into_iter().filter(|i| format!("{} {} {}", i.artist, i.title, i.album).to_lowercase().contains(&q)).collect()))
        }
        _ => sources::search(sid, q).await,
    }
}

/// For tests and the page: the same as JSON.
pub fn to_json(l: &Listing) -> Value {
    json!(l.items)
}
