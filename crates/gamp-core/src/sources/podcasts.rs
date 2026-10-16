//! Podcasts: Apple's directory to find them (no account needed), any RSS
//! feed address, and your subscriptions.

use serde_json::Value;

use super::*;

const SUBSCRIPTIONS: &str = "podcasts.json";

async fn subscriptions() -> Vec<Item> {
    store::read_json::<Vec<Item>>(SUBSCRIPTIONS).await.unwrap_or_default()
}

async fn get_json(url: &str) -> Result<Value, ApiError> {
    let res = client().get(url).send().await.map_err(|e| unreachable("the podcast directory", e))?;
    res.json().await.map_err(|e| bad_gateway(format!("Podcast directory: {e}")))
}

fn show(v: &Value) -> Option<Item> {
    let feed = s(&v["feedUrl"]);
    (!feed.is_empty()).then(|| Item::folder(format!("feed:{feed}"), s(&v["collectionName"])).by(s(&v["artistName"])).items(u(&v["trackCount"])).info(s(&v["primaryGenreName"])))
}

pub struct Feed {
    pub title: String,
    pub author: String,
    pub episodes: Vec<Item>,
}

pub fn parse_feed(xml: &str) -> Result<Feed, String> {
    let doc = roxmltree::Document::parse_with_options(xml, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }).map_err(|e| e.to_string())?;
    let channel = doc.descendants().find(|n| n.tag_name().name() == "channel").ok_or("not an RSS feed")?;
    let text = |node: roxmltree::Node, name: &str| node.children().find(|c| c.tag_name().name() == name).and_then(|c| c.text()).unwrap_or("").trim().to_owned();
    let title = text(channel, "title");
    let author = text(channel, "author");
    let mut episodes = Vec::new();
    for item in channel.children().filter(|n| n.tag_name().name() == "item") {
        let Some(enc) = item.children().find(|c| c.tag_name().name() == "enclosure") else { continue };
        let url = enc.attribute("url").unwrap_or("").trim().to_owned();
        let kind = enc.attribute("type").unwrap_or("audio/mpeg");
        if !url.starts_with("http") || kind.starts_with("video") {
            continue;
        }
        let date = text(item, "pubDate");
        // "Tue, 14 Oct 2026 08:00:00 GMT" → "14 Oct 2026"
        let short: String = date.split_whitespace().skip(1).take(3).collect::<Vec<_>>().join(" ");
        episodes.push(Item::track(url, text(item, "title")).by(title.clone()).on(short).secs(clock(&text(item, "duration"))).info(date));
        if episodes.len() >= 500 {
            break;
        }
    }
    Ok(Feed { title, author, episodes })
}

async fn feed(url: &str) -> Result<Feed, ApiError> {
    let res = client().get(url).send().await.map_err(|e| unreachable("that podcast", e))?;
    if !res.status().is_success() {
        return Err(bad_gateway(format!("The podcast's feed answered {}", res.status())));
    }
    let xml = res.text().await.map_err(|e| unreachable("that podcast", e))?;
    parse_feed(&xml).map_err(|e| bad_gateway(format!("That isn't a podcast feed ({e})")))
}

pub async fn browse(node: &str) -> Result<Listing, ApiError> {
    let (kind, arg) = node.split_once(':').unwrap_or((node, ""));
    match kind {
        "subscriptions" => Ok(Listing::of(subscriptions().await).with("unsubscribe", "Unsubscribe").hint("Search for a podcast (or paste its feed address), select it and press Subscribe.")),
        "top" => {
            let chart = get_json("https://rss.applemarketingtools.com/api/v2/us/podcasts/top/100/podcasts.json").await?;
            let ids: Vec<String> = arr(&chart["feed"]["results"]).iter().map(|r| s(&r["id"])).filter(|i| !i.is_empty()).collect();
            let found = get_json(&format!("https://itunes.apple.com/lookup?id={}&entity=podcast", ids.join(","))).await?;
            let by_id: std::collections::HashMap<String, Item> = arr(&found["results"]).iter().filter_map(|r| Some((s(&r["collectionId"]), show(r)?))).collect();
            let items = ids.iter().filter_map(|i| by_id.get(i).cloned()).collect();
            Ok(Listing::of(items).with("subscribe", "Subscribe"))
        }
        "feed" => {
            let f = feed(arg).await?;
            Ok(Listing::of(f.episodes).hint("This podcast has no episodes to play."))
        }
        _ => Err(not_found("That list")),
    }
}

pub async fn search(q: &str) -> Result<Listing, ApiError> {
    if q.starts_with("http://") || q.starts_with("https://") {
        let f = feed(q).await?;
        let item = Item::folder(format!("feed:{q}"), f.title).by(f.author).items(Some(f.episodes.len() as u64));
        return Ok(Listing::of(vec![item]).with("subscribe", "Subscribe"));
    }
    let r = get_json(&format!("https://itunes.apple.com/search?media=podcast&limit=100&term={}", urlencoding::encode(q))).await?;
    Ok(Listing::of(arr(&r["results"]).iter().filter_map(show).collect()).with("subscribe", "Subscribe"))
}

pub async fn action(action: &str, items: Vec<Item>) -> Result<String, ApiError> {
    let mut subs = subscriptions().await;
    let n = items.len();
    match action {
        "subscribe" => {
            for i in items.into_iter().filter(|i| i.id.starts_with("feed:")) {
                if !subs.iter().any(|f| f.id == i.id) {
                    subs.push(i);
                }
            }
            subs.sort_by_key(|i| i.title.to_lowercase());
        }
        "unsubscribe" => subs.retain(|f| !items.iter().any(|i| i.id == f.id)),
        _ => return Err(not_found("That action")),
    }
    store::write_json(SUBSCRIPTIONS, &subs, false).await?;
    Ok(if action == "subscribe" { format!("Subscribed to {n}") } else { format!("Unsubscribed from {n}") })
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_feeds() {
        let xml = r#"<?xml version="1.0"?><rss xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd"><channel><title>Show</title><itunes:author>Host</itunes:author>
<item><title>Ep 2</title><pubDate>Tue, 14 Oct 2026 08:00:00 GMT</pubDate><itunes:duration>1:02:03</itunes:duration><enclosure url="https://cdn.example/2.mp3" type="audio/mpeg"/></item>
<item><title>Trailer</title><enclosure url="https://cdn.example/t.mp4" type="video/mp4"/></item></channel></rss>"#;
        let f = super::parse_feed(xml).unwrap();
        assert_eq!((f.title.as_str(), f.author.as_str()), ("Show", "Host"));
        assert_eq!(f.episodes.len(), 1);
        assert_eq!(f.episodes[0].duration, Some(3723.0));
        assert_eq!(f.episodes[0].album, "14 Oct 2026");
    }
}
