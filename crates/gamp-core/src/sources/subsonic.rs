//! Subsonic-API servers: Navidrome, Airsonic, Gonic, Ampache, LMS,
//! Funkwhale and Subsonic itself. A favourite on a Synology (in Docker).

use md5::{Digest, Md5};
use serde_json::Value;

use super::*;

/// Token auth: the password never crosses the network.
fn auth(acc: &Account) -> Vec<(&'static str, String)> {
    let salt = random_hex(6);
    let token = hex::encode(Md5::digest(format!("{}{salt}", acc.password).as_bytes()));
    vec![
        ("u", acc.username.clone()),
        ("t", token),
        ("s", salt),
        ("v", "1.16.1".into()),
        ("c", "Gamp".into()),
        ("f", "json".into()),
    ]
}

async fn call(acc: &Account, method: &str, params: &[(&str, &str)]) -> Result<Value, ApiError> {
    let res = acc
        .http()
        .get(format!("{}/rest/{method}", acc.base()))
        .query(&auth(acc))
        .query(params)
        .send()
        .await
        .map_err(|e| unreachable(&acc.name, e))?;
    let status = res.status();
    let body: Value = res
        .json()
        .await
        .map_err(|_| bad_gateway(format!("{} didn't answer like a Subsonic server ({status}). Check the address.", acc.name)))?;
    let r = &body["subsonic-response"];
    if r["status"] == "ok" {
        Ok(r.clone())
    } else {
        let msg = r["error"]["message"].as_str().unwrap_or("it refused");
        Err(bad_gateway(format!("{}: {msg}", acc.name)))
    }
}

pub async fn connect(acc: &mut Account) -> Result<(), ApiError> {
    if acc.username.is_empty() {
        return Err(bad_gateway("Type your user name"));
    }
    call(acc, "ping", &[]).await?;
    Ok(())
}

fn song(v: &Value) -> Item {
    Item::track(s(&v["id"]), s(&v["title"]))
        .by(s(&v["artist"]))
        .on(s(&v["album"]))
        .secs(f(&v["duration"]))
        .no(u(&v["track"]))
        .info([s(&v["suffix"]), s(&v["genre"])].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join(" · "))
}

fn album(v: &Value) -> Item {
    Item::folder(format!("album:{}", s(&v["id"])), s(&v["name"]))
        .by(s(&v["artist"]))
        .items(u(&v["songCount"]))
        .info(s(&v["year"]))
}

fn songs(v: &Value) -> Vec<Item> {
    arr(v).iter().map(song).collect()
}

pub async fn browse(acc: &Account, node: &str) -> Result<Listing, ApiError> {
    let (kind, arg) = node.split_once(':').unwrap_or((node, ""));
    Ok(match kind {
        "starred" => {
            let r = call(acc, "getStarred2", &[]).await?;
            let mut items: Vec<Item> = arr(&r["starred2"]["album"]).iter().map(album).collect();
            items.extend(songs(&r["starred2"]["song"]));
            Listing::of(items).hint("Star songs or albums on your server to see them here.")
        }
        "albums" => {
            let r = call(acc, "getAlbumList2", &[("type", "newest"), ("size", "500")]).await?;
            Listing::of(arr(&r["albumList2"]["album"]).iter().map(album).collect())
        }
        "album" => Listing::of(songs(&call(acc, "getAlbum", &[("id", arg)]).await?["album"]["song"])),
        "artists" => {
            let r = call(acc, "getArtists", &[]).await?;
            let items = arr(&r["artists"]["index"])
                .iter()
                .flat_map(|i| arr(&i["artist"]).iter())
                .map(|a| Item::folder(format!("artist:{}", s(&a["id"])), s(&a["name"])).items(u(&a["albumCount"])))
                .collect();
            Listing::of(items)
        }
        "artist" => Listing::of(arr(&call(acc, "getArtist", &[("id", arg)]).await?["artist"]["album"]).iter().map(album).collect()),
        "playlists" => {
            let r = call(acc, "getPlaylists", &[]).await?;
            let items = arr(&r["playlists"]["playlist"])
                .iter()
                .map(|p| Item::folder(format!("playlist:{}", s(&p["id"])), s(&p["name"])).by(s(&p["owner"])).items(u(&p["songCount"])))
                .collect();
            Listing::of(items)
        }
        "playlist" => Listing::of(songs(&call(acc, "getPlaylist", &[("id", arg)]).await?["playlist"]["entry"])),
        _ => return Err(not_found("That list")),
    })
}

pub async fn search(acc: &Account, q: &str) -> Result<Listing, ApiError> {
    let r = call(acc, "search3", &[("query", q), ("songCount", "200"), ("albumCount", "40"), ("artistCount", "0")]).await?;
    let mut items: Vec<Item> = arr(&r["searchResult3"]["album"]).iter().map(album).collect();
    items.extend(songs(&r["searchResult3"]["song"]));
    Ok(Listing::of(items))
}

pub fn media(acc: &Account, id: &str) -> Upstream {
    let mut url = url::Url::parse(&format!("{}/rest/stream", acc.base())).expect("checked when added");
    url.query_pairs_mut().extend_pairs(auth(acc)).append_pair("id", id);
    Upstream::get(url.to_string())
}
