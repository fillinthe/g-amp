//! Jellyfin and Emby media servers.

use serde_json::{Value, json};

use super::*;

pub fn device_id() -> String {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        let path = store::data_dir().join("device-id");
        std::fs::read_to_string(&path).ok().filter(|s| !s.trim().is_empty()).map(|s| s.trim().to_owned()).unwrap_or_else(|| {
            let id = random_hex(8);
            let _ = std::fs::write(&path, &id);
            id
        })
    })
    .clone()
}

fn authorization(token: &str) -> String {
    let mut h = format!(r#"MediaBrowser Client="G-Amp", Device="G-Amp", DeviceId="{}", Version="0.1.0""#, device_id());
    if !token.is_empty() {
        h.push_str(&format!(r#", Token="{token}""#));
    }
    h
}

async fn get(acc: &Account, path: &str, query: &[(&str, &str)]) -> Result<Value, ApiError> {
    let res = acc
        .http()
        .get(format!("{}{path}", acc.base()))
        .header("Authorization", authorization(&acc.token))
        .header("X-Emby-Authorization", authorization(&acc.token))
        .header("X-Emby-Token", &acc.token)
        .query(query)
        .send()
        .await
        .map_err(|e| unreachable(&acc.name, e))?;
    match res.status() {
        s if s.is_success() => res.json().await.map_err(|e| bad_gateway(format!("{}: unexpected answer ({e})", acc.name))),
        StatusCode::UNAUTHORIZED => Err(bad_gateway(format!("{} signed you out. Remove it in Settings and add it again.", acc.name))),
        s => Err(bad_gateway(format!("{} answered {s}", acc.name))),
    }
}

pub async fn connect(acc: &mut Account, use_server_name: bool) -> Result<(), ApiError> {
    let res = acc
        .http()
        .post(format!("{}/Users/AuthenticateByName", acc.base()))
        .header("Authorization", authorization(""))
        .header("X-Emby-Authorization", authorization(""))
        .json(&json!({ "Username": acc.username, "Pw": acc.password }))
        .send()
        .await
        .map_err(|e| unreachable(&acc.name, e))?;
    let status = res.status();
    if status == StatusCode::UNAUTHORIZED {
        return Err(bad_gateway("Wrong user name or password"));
    }
    let body: Value = res
        .json()
        .await
        .map_err(|_| bad_gateway(format!("That address didn't answer like Jellyfin or Emby ({status}). Check it, including the port (usually 8096).")))?;
    acc.token = s(&body["AccessToken"]);
    acc.user_id = s(&body["User"]["Id"]);
    if acc.token.is_empty() || acc.user_id.is_empty() {
        return Err(bad_gateway("The server didn't sign you in"));
    }
    // The token is enough from now on.
    acc.password.clear();
    if use_server_name {
        let name = get(acc, "/System/Info/Public", &[]).await.map(|v| s(&v["ServerName"])).unwrap_or_default();
        if !name.is_empty() {
            acc.name = name;
        }
    }
    Ok(())
}

const FIELDS: &str = "ChildCount,ProductionYear,Genres,Path";

fn ticks(v: &Value) -> Option<f64> {
    f(v).map(|t| t / 10_000_000.0)
}

fn item(v: &Value) -> Item {
    let artist = arr(&v["Artists"]).first().map(s).filter(|a| !a.is_empty()).unwrap_or_else(|| s(&v["AlbumArtist"]));
    match v["Type"].as_str().unwrap_or("") {
        "Audio" => Item::track(s(&v["Id"]), s(&v["Name"]))
            .by(artist)
            .on(s(&v["Album"]))
            .secs(ticks(&v["RunTimeTicks"]))
            .no(u(&v["IndexNumber"]))
            .info(s(&v["Container"])),
        "MusicAlbum" => Item::folder(format!("album:{}", s(&v["Id"])), s(&v["Name"]))
            .by(s(&v["AlbumArtist"]))
            .items(u(&v["ChildCount"]))
            .info(s(&v["ProductionYear"])),
        "Playlist" => Item::folder(format!("playlist:{}", s(&v["Id"])), s(&v["Name"])).items(u(&v["ChildCount"])),
        _ => Item::folder(format!("artist:{}", s(&v["Id"])), s(&v["Name"])),
    }
}

async fn items(acc: &Account, query: &[(&str, &str)]) -> Result<Listing, ApiError> {
    let mut q = vec![("Fields", FIELDS), ("EnableImages", "false")];
    q.extend_from_slice(query);
    let r = get(acc, &format!("/Users/{}/Items", acc.user_id), &q).await?;
    Ok(Listing::of(arr(&r["Items"]).iter().map(item).collect()))
}

pub async fn browse(acc: &Account, node: &str) -> Result<Listing, ApiError> {
    let (kind, arg) = node.split_once(':').unwrap_or((node, ""));
    match kind {
        "favorites" => Ok(items(
            acc,
            &[("IncludeItemTypes", "Audio,MusicAlbum"), ("Recursive", "true"), ("Filters", "IsFavorite"), ("SortBy", "AlbumArtist,Album,ParentIndexNumber,IndexNumber")],
        )
        .await?
        .hint("Mark songs or albums as favourites on your server to see them here.")),
        "albums" => items(acc, &[("IncludeItemTypes", "MusicAlbum"), ("Recursive", "true"), ("SortBy", "DateCreated"), ("SortOrder", "Descending"), ("Limit", "500")]).await,
        "album" => items(acc, &[("ParentId", arg), ("IncludeItemTypes", "Audio"), ("SortBy", "ParentIndexNumber,IndexNumber,SortName")]).await,
        "artists" => {
            let r = get(acc, "/Artists/AlbumArtists", &[("UserId", acc.user_id.as_str()), ("Limit", "5000"), ("EnableImages", "false")]).await?;
            Ok(Listing::of(arr(&r["Items"]).iter().map(|a| Item::folder(format!("artist:{}", s(&a["Id"])), s(&a["Name"]))).collect()))
        }
        "artist" => items(acc, &[("IncludeItemTypes", "MusicAlbum"), ("Recursive", "true"), ("AlbumArtistIds", arg), ("SortBy", "ProductionYear,SortName")]).await,
        "playlists" => items(acc, &[("IncludeItemTypes", "Playlist"), ("Recursive", "true"), ("SortBy", "SortName")]).await,
        "playlist" => {
            let r = get(acc, &format!("/Playlists/{arg}/Items"), &[("UserId", acc.user_id.as_str()), ("Fields", FIELDS), ("EnableImages", "false")]).await?;
            Ok(Listing::of(arr(&r["Items"]).iter().map(item).collect()))
        }
        _ => Err(not_found("That list")),
    }
}

pub async fn search(acc: &Account, q: &str) -> Result<Listing, ApiError> {
    let mut l = items(acc, &[("SearchTerm", q), ("IncludeItemTypes", "MusicAlbum,Audio"), ("Recursive", "true"), ("Limit", "300")]).await?;
    // Albums first, then songs.
    l.items.sort_by_key(|i| i.kind == "track");
    Ok(l)
}

pub fn media(acc: &Account, id: &str) -> Upstream {
    // The original file, untouched: seekable, and FLAC stays FLAC.
    Upstream::get(format!("{}/Audio/{}/stream?static=true&api_key={}", acc.base(), urlencoding::encode(id), acc.token))
        .header("X-Emby-Token", acc.token.clone())
}
