//! Jamendo: free Creative Commons music, through its API. Jamendo gives
//! every app its own client ID, free, so you paste yours in Settings.

use serde_json::Value;

use super::*;

const API: &str = "https://api.jamendo.com/v3.0";

async fn get(path: &str, query: &[(&str, &str)]) -> Result<Value, ApiError> {
    let id = store::config().await.jamendo_client;
    if id.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "Jamendo needs a free client ID: add one in Settings › Jamendo."));
    }
    let mut url = url::Url::parse(&format!("{API}{path}")).expect("static");
    url.query_pairs_mut().append_pair("client_id", &id).append_pair("format", "json").append_pair("limit", "200").extend_pairs(query);
    let res = client().get(url).send().await.map_err(|e| unreachable("Jamendo", e))?;
    if !res.status().is_success() {
        return Err(bad_gateway(format!("Jamendo answered {}", res.status())));
    }
    let v: Value = res.json().await.map_err(|e| bad_gateway(format!("Jamendo: {e}")))?;
    if v["headers"]["status"] != "success" {
        return Err(bad_gateway(format!("Jamendo: {}", s(&v["headers"]["error_message"]))));
    }
    Ok(v["results"].clone())
}

pub fn tracks(v: &Value) -> Vec<Item> {
    let mut t: Vec<&Value> = arr(v).iter().filter(|t| !s(&t["audio"]).is_empty()).collect();
    // An album's tracks come in its order.
    t.sort_by_key(|t| u(&t["position"]).unwrap_or(0));
    t.into_iter()
        .map(|t| {
            let mut i = Item::track(s(&t["audio"]), s(&t["name"])).by(s(&t["artist_name"])).on(s(&t["album_name"])).secs(f(&t["duration"])).no(u(&t["position"]));
            i.info = s(&t["license_ccurl"]);
            i
        })
        .collect()
}

fn folders(v: &Value, kind: &str) -> Vec<Item> {
    arr(v).iter().map(|a| Item::folder(format!("{kind}:{}", s(&a["id"])), s(&a["name"])).by(s(&a["artist_name"])).info(s(&a["releasedate"]))).collect()
}

pub async fn browse(node: &str) -> Result<Listing, ApiError> {
    let (kind, arg) = node.split_once(':').unwrap_or((node, ""));
    let items = match kind {
        "popular" => tracks(&get("/tracks/", &[("order", "popularity_week")]).await?),
        "new" => tracks(&get("/tracks/", &[("order", "releasedate_desc")]).await?),
        "album" => tracks(&get("/tracks/", &[("album_id", arg)]).await?),
        "artist" => tracks(&get("/tracks/", &[("artist_id", arg), ("order", "popularity_total")]).await?),
        _ => return Err(not_found("That list")),
    };
    Ok(Listing::of(items))
}

/// Tracks first, then albums and artists that match.
pub async fn search(q: &str) -> Result<Listing, ApiError> {
    let q = q.trim();
    let (tq, nq) = ([("search", q)], [("namesearch", q)]);
    let (t, al, ar) = tokio::join!(get("/tracks/", &tq), get("/albums/", &nq), get("/artists/", &nq));
    let mut items = tracks(&t?);
    items.extend(folders(&al.unwrap_or_default(), "album"));
    items.extend(folders(&ar.unwrap_or_default(), "artist"));
    Ok(Listing::of(items))
}

#[cfg(test)]
mod tests {
    #[test]
    fn album_tracks_in_order() {
        let v = serde_json::json!([
            {"name": "Two", "position": 2, "audio": "https://x/2.mp3", "duration": 200, "artist_name": "A"},
            {"name": "One", "position": 1, "audio": "https://x/1.mp3"},
            {"name": "No audio", "position": 3, "audio": ""},
        ]);
        let t = super::tracks(&v);
        assert_eq!(t.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["One", "Two"]);
        assert_eq!(t[1].id, "https://x/2.mp3");
    }
}
