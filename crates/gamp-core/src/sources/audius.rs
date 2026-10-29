//! Audius: free full-length tracks from independent artists, through its
//! open API (no account).

use serde_json::Value;

use super::*;

const API: &str = "https://api.audius.co/v1";
const APP: &str = "app_name=Gamp";

async fn get(path: &str) -> Result<Value, ApiError> {
    let sep = if path.contains('?') { '&' } else { '?' };
    let res = client().get(format!("{API}{path}{sep}{APP}")).send().await.map_err(|e| unreachable("Audius", e))?;
    if !res.status().is_success() {
        return Err(bad_gateway(format!("Audius answered {}", res.status())));
    }
    let v: Value = res.json().await.map_err(|e| bad_gateway(format!("Audius: {e}")))?;
    Ok(v["data"].clone())
}

/// Tracks that play for anyone (not gated or deleted).
pub fn tracks(v: &Value) -> Vec<Item> {
    arr(v)
        .iter()
        .filter(|t| t["is_streamable"] == true && t["access"]["stream"] != false)
        .map(|t| {
            let id = s(&t["id"]);
            let mut i = Item::track(format!("{API}/tracks/{id}/stream?{APP}"), s(&t["title"])).by(s(&t["user"]["name"])).secs(f(&t["duration"])).info(s(&t["genre"]));
            i.date = s(&t["release_date"]).chars().take(10).collect();
            i.plays = u(&t["play_count"]);
            i.likes = u(&t["favorite_count"]);
            i
        })
        .collect()
}

fn playlists(v: &Value) -> Vec<Item> {
    arr(v).iter().map(|p| Item::folder(format!("pl:{}", s(&p["id"])), s(&p["playlist_name"])).by(s(&p["user"]["name"])).items(u(&p["track_count"]))).collect()
}

fn artists(v: &Value) -> Vec<Item> {
    arr(v).iter().map(|a| Item::folder(format!("user:{}", s(&a["id"])), s(&a["name"])).by(format!("@{}", s(&a["handle"]))).items(u(&a["track_count"]))).collect()
}

pub async fn browse(node: &str) -> Result<Listing, ApiError> {
    let (kind, arg) = node.split_once(':').unwrap_or((node, ""));
    let arg = urlencoding::encode(arg);
    let items = match kind {
        "trending" => tracks(&get("/tracks/trending?time=week").await?),
        "underground" => tracks(&get("/tracks/trending/underground").await?),
        "pl" => tracks(&get(&format!("/playlists/{arg}/tracks")).await?),
        "user" => tracks(&get(&format!("/users/{arg}/tracks?limit=100")).await?),
        _ => return Err(not_found("That list")),
    };
    Ok(Listing::of(items).hint("Nothing here plays for free."))
}

/// Tracks first, then playlists and artists that match.
pub async fn search(q: &str) -> Result<Listing, ApiError> {
    let q = urlencoding::encode(q.trim());
    let [tp, pp, ap] = ["tracks", "playlists", "users"].map(|k| format!("/{k}/search?query={q}"));
    let (t, p, a) = tokio::join!(get(&tp), get(&pp), get(&ap));
    let mut items = tracks(&t?);
    items.extend(playlists(&p.unwrap_or_default()));
    items.extend(artists(&a.unwrap_or_default()));
    Ok(Listing::of(items))
}

#[cfg(test)]
mod tests {
    #[test]
    fn keeps_tracks_that_play() {
        let v = serde_json::json!([
            {"id": "a1", "title": "Free", "is_streamable": true, "access": {"stream": true}, "duration": 71, "user": {"name": "Me"}, "release_date": "2021-04-20T08:00:10Z", "play_count": 5},
            {"id": "b2", "title": "Gated", "is_streamable": true, "access": {"stream": false}, "user": {"name": "Me"}},
            {"id": "c3", "title": "Gone", "is_streamable": false, "user": {"name": "Me"}},
        ]);
        let t = super::tracks(&v);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].id, "https://api.audius.co/v1/tracks/a1/stream?app_name=Gamp");
        assert_eq!((t[0].date.as_str(), t[0].plays, t[0].duration), ("2021-04-20", Some(5), Some(71.0)));
    }
}
