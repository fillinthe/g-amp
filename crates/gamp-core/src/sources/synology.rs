//! Synology Audio Station, over the DSM Web API: the same library the
//! DS audio app plays, from home (http://nas:5000) or away (your DDNS
//! address, https://you.synology.me:5001).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::*;

const APIS: &[(&str, u64)] = &[
    ("SYNO.API.Auth", 6),
    ("SYNO.AudioStation.Song", 3),
    ("SYNO.AudioStation.Album", 3),
    ("SYNO.AudioStation.Folder", 3),
    ("SYNO.AudioStation.Playlist", 3),
    ("SYNO.AudioStation.Search", 1),
    ("SYNO.AudioStation.Stream", 2),
];

#[derive(Clone)]
struct Api {
    path: String,
    version: u64,
}

#[derive(Clone)]
struct Session {
    sid: String,
    apis: HashMap<String, Api>,
    at: Instant,
}

fn sessions() -> &'static AsyncMutex<HashMap<String, Session>> {
    static S: OnceLock<AsyncMutex<HashMap<String, Session>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

fn why(code: u64) -> &'static str {
    match code {
        400 => "wrong user name or password",
        401 => "this account is disabled",
        402 => "this account can't use Audio Station",
        403 | 406 => "type your two-step code too (it's asked for once)",
        404 => "that two-step code didn't work",
        407 => "this address is blocked by the NAS after too many tries",
        102 => "Audio Station isn't installed on this NAS",
        105 => "this account can't use Audio Station",
        _ => "it refused",
    }
}

async fn query_apis(acc: &Account) -> Result<HashMap<String, Api>, ApiError> {
    let names: Vec<&str> = APIS.iter().map(|(n, _)| *n).collect();
    let res = acc
        .http()
        .get(format!("{}/webapi/query.cgi", acc.base()))
        .query(&[("api", "SYNO.API.Info"), ("version", "1"), ("method", "query"), ("query", &names.join(","))])
        .send()
        .await
        .map_err(|e| unreachable(&acc.name, e))?;
    let body: Value = res
        .json()
        .await
        .map_err(|_| bad_gateway("That address didn't answer like a Synology NAS. Use the DSM address, e.g. http://nas:5000 or https://you.synology.me:5001"))?;
    let mut out = HashMap::new();
    for (name, cap) in APIS {
        let a = &body["data"][name];
        if let (Some(path), Some(max)) = (a["path"].as_str(), a["maxVersion"].as_u64()) {
            let min = a["minVersion"].as_u64().unwrap_or(1);
            out.insert(name.to_string(), Api { path: path.to_owned(), version: (*cap).min(max).max(min) });
        }
    }
    if !out.contains_key("SYNO.AudioStation.Song") {
        return Err(bad_gateway("Audio Station isn't installed on this NAS (Package Center › Audio Station)"));
    }
    Ok(out)
}

async fn login(acc: &Account, otp: &str) -> Result<(Session, String), ApiError> {
    let apis = query_apis(acc).await?;
    let auth = apis.get("SYNO.API.Auth").cloned().unwrap_or(Api { path: "auth.cgi".into(), version: 3 });
    let version = auth.version.to_string();
    let mut q: Vec<(&str, &str)> = vec![
        ("api", "SYNO.API.Auth"),
        ("version", &version),
        ("method", "login"),
        ("account", &acc.username),
        ("passwd", &acc.password),
        ("session", "AudioStation"),
        ("format", "sid"),
    ];
    if !otp.is_empty() {
        q.extend([("otp_code", otp), ("enable_device_token", "yes"), ("device_name", "Gamp")]);
    } else if !acc.token.is_empty() {
        // The trusted-device id from the first two-step sign-in.
        q.push(("device_id", &acc.token));
    }
    let res = acc
        .http()
        .get(format!("{}/webapi/{}", acc.base(), auth.path))
        .query(&q)
        .send()
        .await
        .map_err(|e| unreachable(&acc.name, e))?;
    let body: Value = res.json().await.map_err(|e| bad_gateway(format!("{}: {e}", acc.name)))?;
    if body["success"] != true {
        let code = body["error"]["code"].as_u64().unwrap_or(0);
        return Err(bad_gateway(format!("{}: {}", if acc.name.is_empty() { "Synology" } else { &acc.name }, why(code))));
    }
    let sid = s(&body["data"]["sid"]);
    let did = s(&body["data"]["did"]);
    Ok((Session { sid, apis, at: Instant::now() }, did))
}

pub async fn connect(acc: &mut Account, otp: &str) -> Result<(), ApiError> {
    if acc.username.is_empty() {
        return Err(bad_gateway("Type your DSM user name"));
    }
    let (session, did) = login(acc, otp).await?;
    if !did.is_empty() {
        acc.token = did;
    }
    sessions().lock().await.insert(acc.id.clone(), session);
    Ok(())
}

async fn session(acc: &Account, fresh: bool) -> Result<Session, ApiError> {
    let mut all = sessions().lock().await;
    if !fresh {
        if let Some(s) = all.get(&acc.id) {
            return Ok(s.clone());
        }
    }
    let (s, _) = login(acc, "").await?;
    all.insert(acc.id.clone(), s.clone());
    Ok(s)
}

async fn call(acc: &Account, api: &str, method: &str, params: &[(&str, &str)]) -> Result<Value, ApiError> {
    for attempt in 0..2 {
        let sess = session(acc, attempt > 0).await?;
        let Some(a) = sess.apis.get(api) else {
            return Err(bad_gateway(format!("This NAS's Audio Station has no {api}")));
        };
        let version = a.version.to_string();
        let res = acc
            .http()
            .get(format!("{}/webapi/{}", acc.base(), a.path))
            .query(&[("api", api), ("version", &version), ("method", method), ("_sid", &sess.sid)])
            .query(params)
            .send()
            .await
            .map_err(|e| unreachable(&acc.name, e))?;
        let body: Value = res.json().await.map_err(|e| bad_gateway(format!("{}: {e}", acc.name)))?;
        if body["success"] == true {
            return Ok(body["data"].clone());
        }
        let code = body["error"]["code"].as_u64().unwrap_or(0);
        // 106, 107, 119: the session ended. Sign in again, once.
        if attempt == 0 && matches!(code, 105 | 106 | 107 | 119) {
            continue;
        }
        return Err(bad_gateway(format!("{}: Audio Station error {code}", acc.name)));
    }
    unreachable!()
}

const EXTRA: (&str, &str) = ("additional", "song_tag,song_audio");

fn song(v: &Value) -> Item {
    let tag = &v["additional"]["song_tag"];
    let audio = &v["additional"]["song_audio"];
    let title = Some(s(&v["title"])).filter(|t| !t.is_empty()).unwrap_or_else(|| stem(&s(&v["path"]).rsplit('/').next().unwrap_or("").to_owned()));
    Item::track(s(&v["id"]), title)
        .by(s(&tag["artist"]))
        .on(s(&tag["album"]))
        .secs(f(&audio["duration"]))
        .no(u(&tag["track"]))
        .info(s(&v["path"]))
}

fn songs(v: &Value) -> Vec<Item> {
    arr(v).iter().map(song).collect()
}

pub async fn browse(acc: &Account, node: &str) -> Result<Listing, ApiError> {
    let (kind, arg) = node.split_once(':').unwrap_or((node, ""));
    Ok(match kind {
        "songs" => Listing::of(songs(
            &call(acc, "SYNO.AudioStation.Song", "list", &[("library", "all"), ("limit", "20000"), ("offset", "0"), ("sort_by", "artist"), EXTRA]).await?["songs"],
        )),
        "albums" => {
            let r = call(acc, "SYNO.AudioStation.Album", "list", &[("library", "all"), ("limit", "5000"), ("offset", "0"), ("sort_by", "display_artist")]).await?;
            let items = arr(&r["albums"])
                .iter()
                .map(|a| {
                    let artist = s(&a["album_artist"]);
                    let id = format!("album:{}\u{1f}{}", s(&a["name"]), artist);
                    Item::folder(id, s(&a["name"])).by(Some(s(&a["display_artist"])).filter(|d| !d.is_empty()).unwrap_or(artist)).info(s(&a["year"]))
                })
                .collect();
            Listing::of(items)
        }
        "album" => {
            let (name, artist) = arg.split_once('\u{1f}').unwrap_or((arg, ""));
            let mut l = Listing::of(songs(
                &call(acc, "SYNO.AudioStation.Song", "list", &[("library", "all"), ("album", name), ("album_artist", artist), ("limit", "1000"), EXTRA]).await?["songs"],
            ));
            l.items.sort_by_key(|i| i.track_no.unwrap_or(u32::MAX));
            l
        }
        "folders" | "folder" => {
            let mut q = vec![("library", "all"), ("limit", "5000"), ("offset", "0"), ("sort_by", "title"), EXTRA];
            if kind == "folder" {
                q.push(("id", arg));
            }
            let r = call(acc, "SYNO.AudioStation.Folder", "list", &q).await?;
            let items = arr(&r["items"])
                .iter()
                .map(|i| if i["type"] == "folder" { Item::folder(format!("folder:{}", s(&i["id"])), s(&i["title"])).info(s(&i["path"])) } else { song(i) })
                .collect();
            Listing::of(items)
        }
        "playlists" => {
            let r = call(acc, "SYNO.AudioStation.Playlist", "list", &[("library", "all"), ("limit", "1000"), ("offset", "0")]).await?;
            Listing::of(arr(&r["playlists"]).iter().map(|p| Item::folder(format!("playlist:{}", s(&p["id"])), s(&p["name"]))).collect())
        }
        "playlist" => {
            let r = call(acc, "SYNO.AudioStation.Playlist", "getinfo", &[("id", arg), ("library", "all"), ("additional", "songs_song_tag,songs_song_audio")]).await?;
            Listing::of(songs(&r["playlists"][0]["additional"]["songs"]))
        }
        _ => return Err(not_found("That list")),
    })
}

pub async fn search(acc: &Account, q: &str) -> Result<Listing, ApiError> {
    let r = call(acc, "SYNO.AudioStation.Search", "list", &[("keyword", q), ("library", "all"), ("limit", "500"), ("offset", "0"), ("sort_by", "title"), EXTRA]).await?;
    Ok(Listing::of(songs(&r["songs"])))
}

pub async fn media(acc: &Account, id: &str) -> Result<Upstream, ApiError> {
    // The stream URL carries the session; check it is still good first.
    let stale = sessions().lock().await.get(&acc.id).is_none_or(|s| s.at.elapsed() > Duration::from_secs(600));
    if stale {
        call(acc, "SYNO.AudioStation.Playlist", "list", &[("library", "all"), ("limit", "1")]).await?;
        if let Some(s) = sessions().lock().await.get_mut(&acc.id) {
            s.at = Instant::now();
        }
    }
    let sess = session(acc, false).await?;
    let a = sess.apis.get("SYNO.AudioStation.Stream").cloned().unwrap_or(Api { path: "AudioStation/stream.cgi".into(), version: 2 });
    let mut url = url::Url::parse(&format!("{}/webapi/{}/0.mp3", acc.base(), a.path))?;
    url.query_pairs_mut()
        .append_pair("api", "SYNO.AudioStation.Stream")
        .append_pair("version", &a.version.to_string())
        .append_pair("method", "stream")
        .append_pair("id", id)
        .append_pair("_sid", &sess.sid);
    Ok(Upstream::get(url.to_string()))
}
