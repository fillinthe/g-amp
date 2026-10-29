//! Internet radio, as in Winamp's SHOUTcast days: about 50,000 stations
//! from the community directory at radio-browser.info, your favourites,
//! and any stream address you paste (.pls and .m3u playlists included).

use serde_json::Value;

use super::*;

const FAVORITES: &str = "radio.json";
/// The directory's servers; any of them will do.
const SERVERS: &[&str] = &["https://de1.api.radio-browser.info", "https://fi1.api.radio-browser.info", "https://de2.api.radio-browser.info", "https://all.api.radio-browser.info"];

async fn directory(path: &str) -> Result<Value, ApiError> {
    let mut last = None;
    for base in SERVERS {
        match client().get(format!("{base}{path}")).timeout(std::time::Duration::from_secs(10)).send().await {
            Ok(r) if r.status().is_success() => return r.json().await.map_err(|e| bad_gateway(format!("Radio directory: {e}"))),
            Ok(r) => last = Some(bad_gateway(format!("The radio directory answered {}", r.status()))),
            Err(e) => last = Some(unreachable("the radio directory", e)),
        }
    }
    Err(last.unwrap_or_else(|| bad_gateway("No radio directory")))
}

fn station(v: &Value) -> Option<Item> {
    // HLS needs a different player; the rest plays as one endless file.
    if v["hls"].as_u64() == Some(1) || v["lastcheckok"].as_u64() == Some(0) {
        return None;
    }
    let url = Some(s(&v["url_resolved"])).filter(|u| u.starts_with("http")).unwrap_or_else(|| s(&v["url"]));
    if !url.starts_with("http") || url.contains(".m3u8") {
        return None;
    }
    let tags: Vec<&str> = v["tags"].as_str().unwrap_or("").split(',').filter(|t| !t.is_empty()).take(3).collect();
    let codec = s(&v["codec"]);
    let kbps = u(&v["bitrate"]).filter(|b| *b > 0).map(|b| format!(" {b} kbps")).unwrap_or_default();
    let mut item = Item::track(url, s(&v["name"]).trim().to_owned()).by(tags.join(", ")).on(format!("{codec}{kbps}").trim().to_owned()).info(s(&v["country"]));
    item.live = true;
    Some(item)
}

fn stations(v: &Value) -> Vec<Item> {
    arr(v).iter().filter_map(station).collect()
}

async fn favorites() -> Vec<Item> {
    store::read_json::<Vec<Item>>(FAVORITES).await.unwrap_or_default()
}

pub async fn browse(node: &str) -> Result<Listing, ApiError> {
    let (kind, arg) = node.split_once(':').unwrap_or((node, ""));
    let arg = urlencoding::encode(arg);
    let q = "hidebroken=true&order=clickcount&reverse=true&limit=300";
    Ok(match kind {
        "favorites" => {
            let mut items = favorites().await;
            for i in &mut items {
                i.live = true;
            }
            return Ok(Listing::of(items).with("unfavorite", "Remove").hint("Select stations anywhere in Internet Radio and press Add to Favorites. You can also paste a stream address into Search."));
        }
        "top" => Listing::of(stations(&directory(&format!("/json/stations/topclick/300?hidebroken=true")).await?)),
        "genres" => {
            let v = directory("/json/tags?order=stationcount&reverse=true&limit=150&hidebroken=true").await?;
            let items = arr(&v).iter().map(|t| Item::folder(format!("tag:{}", s(&t["name"])), s(&t["name"])).items(u(&t["stationcount"]))).collect();
            return Ok(Listing::of(items));
        }
        "tag" => Listing::of(stations(&directory(&format!("/json/stations/bytagexact/{arg}?{q}")).await?)),
        "countries" => {
            let v = directory("/json/countries?order=stationcount&reverse=true&hidebroken=true").await?;
            let items = arr(&v)
                .iter()
                .filter(|c| !s(&c["iso_3166_1"]).is_empty())
                .map(|c| Item::folder(format!("country:{}", s(&c["iso_3166_1"])), s(&c["name"])).items(u(&c["stationcount"])))
                .collect();
            return Ok(Listing::of(items));
        }
        "country" => Listing::of(stations(&directory(&format!("/json/stations/bycountrycodeexact/{arg}?{q}")).await?)),
        _ => return Err(not_found("That list")),
    }
    .with("fav:add", "Add to Favorites"))
}

/// The stream addresses inside a .pls or .m3u playlist.
pub fn playlist_entries(body: &str) -> Vec<String> {
    body.lines()
        .map(str::trim)
        .filter_map(|l| if l.to_ascii_lowercase().starts_with("file") { l.split_once('=').map(|(_, v)| v.trim()) } else { Some(l) })
        .filter(|l| l.starts_with("http://") || l.starts_with("https://"))
        .map(str::to_owned)
        .collect()
}

/// A pasted address: a stream, or a playlist pointing at streams.
async fn from_url(url: &str) -> Result<Listing, ApiError> {
    let lower = url.to_ascii_lowercase();
    let path = lower.split(['?', '#']).next().unwrap_or("");
    let mut urls = vec![url.to_owned()];
    if path.ends_with(".pls") || path.ends_with(".m3u") {
        let body = client().get(url).send().await.map_err(|e| unreachable("that address", e))?.text().await.map_err(|e| unreachable("that address", e))?;
        urls = playlist_entries(&body);
        if urls.is_empty() {
            return Err(bad_gateway("That playlist lists no streams"));
        }
    }
    let host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default();
    let items = urls
        .into_iter()
        .map(|u| {
            let mut i = Item::track(u, host.clone()).on("Stream");
            i.live = true;
            i
        })
        .collect();
    Ok(Listing::of(items).with("fav:add", "Add to Favorites"))
}

pub async fn search(q: &str) -> Result<Listing, ApiError> {
    if q.starts_with("http://") || q.starts_with("https://") {
        return from_url(q).await;
    }
    let v = directory(&format!("/json/stations/search?name={}&hidebroken=true&order=clickcount&reverse=true&limit=300", urlencoding::encode(q))).await?;
    let mut items = stations(&v);
    // "soma groove" finds "SomaFM Groove Salad": search on the longest
    // word, keep stations whose name and tags hold every word.
    let words: Vec<String> = q.to_lowercase().split_whitespace().map(str::to_owned).collect();
    if items.is_empty() && words.len() > 1 {
        let longest = words.iter().max_by_key(|w| w.len()).cloned().unwrap_or_default();
        let v = directory(&format!("/json/stations/search?name={}&hidebroken=true&order=clickcount&reverse=true&limit=1000", urlencoding::encode(&longest))).await?;
        items = stations(&v)
            .into_iter()
            .filter(|i| {
                let hay = format!("{} {}", i.title, i.artist).to_lowercase();
                words.iter().all(|w| hay.contains(w.as_str()))
            })
            .collect();
    }
    if items.is_empty() {
        let v = directory(&format!("/json/stations/bytag/{}?hidebroken=true&order=clickcount&reverse=true&limit=300", urlencoding::encode(q))).await?;
        items = stations(&v);
    }
    Ok(Listing::of(items).with("fav:add", "Add to Favorites"))
}

pub async fn action(action: &str, items: Vec<Item>) -> Result<String, ApiError> {
    let mut favs = favorites().await;
    let n = items.len();
    match action {
        "favorite" => {
            for mut i in items.into_iter().filter(|i| i.kind == "track") {
                if !favs.iter().any(|f| f.id == i.id) {
                    i.url = None;
                    favs.push(i);
                }
            }
        }
        "unfavorite" => favs.retain(|f| !items.iter().any(|i| i.id == f.id)),
        _ => return Err(not_found("That action")),
    }
    store::write_json(FAVORITES, &favs, false).await?;
    Ok(if action == "favorite" { format!("Added {n} to Favorites") } else { format!("Removed {n}") })
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_playlists() {
        let pls = "[playlist]\nNumberOfEntries=2\nFile1=http://a.example:8000/stream\nTitle1=A\nFile2=https://b.example/live\n";
        assert_eq!(super::playlist_entries(pls), vec!["http://a.example:8000/stream", "https://b.example/live"]);
        let m3u = "#EXTM3U\n#EXTINF:-1,Station\nhttp://c.example/x.mp3\n";
        assert_eq!(super::playlist_entries(m3u), vec!["http://c.example/x.mp3"]);
    }
}
