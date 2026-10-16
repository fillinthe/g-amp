//! The Internet Archive's free audio: the Live Music Archive (taper-
//! friendly bands' concerts), digitised 78s, netlabels, old radio.

use serde_json::Value;

use super::*;

async fn get_json(url: &str) -> Result<Value, ApiError> {
    let res = client().get(url).send().await.map_err(|e| unreachable("archive.org", e))?;
    if !res.status().is_success() {
        return Err(bad_gateway(format!("archive.org answered {}", res.status())));
    }
    res.json().await.map_err(|e| bad_gateway(format!("archive.org: {e}")))
}

async fn find_items(query: &str) -> Result<Listing, ApiError> {
    let mut url = url::Url::parse("https://archive.org/advancedsearch.php").expect("static");
    url.query_pairs_mut()
        .append_pair("q", &format!("({query}) AND mediatype:(audio OR etree)"))
        .append_pair("fl[]", "identifier")
        .append_pair("fl[]", "title")
        .append_pair("fl[]", "creator")
        .append_pair("fl[]", "date")
        .append_pair("sort[]", "downloads desc")
        .append_pair("rows", "300")
        .append_pair("page", "1")
        .append_pair("output", "json");
    let r = get_json(url.as_str()).await?;
    let first = |v: &Value| v.as_array().and_then(|a| a.first()).map(s).unwrap_or_else(|| s(v));
    let items = arr(&r["response"]["docs"])
        .iter()
        .map(|d| Item::folder(format!("item:{}", s(&d["identifier"])), first(&d["title"])).by(first(&d["creator"])).info(first(&d["date"]).chars().take(10).collect::<String>()))
        .collect();
    Ok(Listing::of(items))
}

/// One recording's tracks, in the best format the browser plays.
pub fn pick_files(meta: &Value) -> Vec<Item> {
    let id = s(&meta["metadata"]["identifier"]);
    let album = s(&meta["metadata"]["title"]);
    let creator = meta["metadata"]["creator"].as_array().and_then(|a| a.first()).map(s).unwrap_or_else(|| s(&meta["metadata"]["creator"]));
    let files = arr(&meta["files"]);
    let order = ["VBR MP3", "MP3", "128Kbps MP3", "64Kbps MP3", "Ogg Vorbis", "Flac", "24bit Flac"];
    let Some(best) = order.iter().find(|fmt| files.iter().any(|f| f["format"] == **fmt)) else { return Vec::new() };
    let mut picked: Vec<&Value> = files.iter().filter(|f| f["format"] == *best).collect();
    picked.sort_by(|a, b| {
        let n = |f: &Value| u(&f["track"]).unwrap_or(u64::MAX);
        n(a).cmp(&n(b)).then_with(|| s(&a["name"]).cmp(&s(&b["name"])))
    });
    picked
        .into_iter()
        .map(|f| {
            let name = s(&f["name"]);
            let url = format!("https://archive.org/download/{}/{}", urlencoding::encode(&id), name.split('/').map(|p| urlencoding::encode(p).into_owned()).collect::<Vec<_>>().join("/"));
            let title = Some(s(&f["title"])).filter(|t| !t.is_empty()).unwrap_or_else(|| stem(name.rsplit('/').next().unwrap_or(&name)));
            let artist = [s(&f["creator"]), s(&f["artist"])].into_iter().find(|a| !a.is_empty()).unwrap_or_else(|| creator.clone());
            Item::track(url, title).by(artist).on(album.clone()).secs(clock(&s(&f["length"]))).no(u(&f["track"])).info(s(&f["format"]))
        })
        .collect()
}

pub async fn browse(node: &str) -> Result<Listing, ApiError> {
    let (kind, arg) = node.split_once(':').unwrap_or((node, ""));
    match kind {
        "c" => find_items(&format!("collection:({arg})")).await,
        "item" => {
            let meta = get_json(&format!("https://archive.org/metadata/{}", urlencoding::encode(arg))).await?;
            Ok(Listing::of(pick_files(&meta)).hint("No playable audio in this item."))
        }
        _ => Err(not_found("That list")),
    }
}

pub async fn search(q: &str) -> Result<Listing, ApiError> {
    let clean: String = q.chars().filter(|c| c.is_alphanumeric() || c.is_whitespace() || "'-.".contains(*c)).collect();
    // Every word must match (the Archive's default is any word).
    let words: Vec<&str> = clean.split_whitespace().collect();
    if words.is_empty() {
        return Ok(Listing::default());
    }
    let each: Vec<String> = words.iter().map(|w| format!("(title:({w}) OR creator:({w}))")).collect();
    find_items(&each.join(" AND ")).await
}

#[cfg(test)]
mod tests {
    #[test]
    fn prefers_vbr_mp3() {
        let meta = serde_json::json!({
            "metadata": {"identifier": "gd1977", "title": "Live 1977", "creator": "Band"},
            "files": [
                {"name": "t02.flac", "format": "Flac", "track": "2"},
                {"name": "t02.mp3", "format": "VBR MP3", "track": "2", "title": "Two", "length": "05:10"},
                {"name": "t01.mp3", "format": "VBR MP3", "track": "1", "length": "312.5"},
            ]
        });
        let items = super::pick_files(&meta);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, "https://archive.org/download/gd1977/t01.mp3");
        assert_eq!(items[0].title, "t01");
        assert_eq!(items[1].duration, Some(310.0));
        assert_eq!(items[1].artist, "Band");
    }
}
