//! Plex Media Server. You sign in on plex.tv (in your browser); every
//! server your account can reach is then added, with each of its addresses
//! so it keeps working at home and away.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::*;

const PRODUCT: &str = "G-Amp";

fn client_id() -> String {
    format!("g-amp-{}", super::jellyfin::device_id())
}

fn plex_headers(req: reqwest::RequestBuilder, token: &str) -> reqwest::RequestBuilder {
    let req = req
        .header("Accept", "application/json")
        .header("X-Plex-Product", PRODUCT)
        .header("X-Plex-Version", "0.1.0")
        .header("X-Plex-Client-Identifier", client_id())
        .header("X-Plex-Device-Name", PRODUCT);
    if token.is_empty() { req } else { req.header("X-Plex-Token", token) }
}

// ---- sign-in ------------------------------------------------------------

struct Pin {
    id: u64,
    at: Instant,
}

fn pins() -> &'static Mutex<HashMap<String, Pin>> {
    static P: OnceLock<Mutex<HashMap<String, Pin>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

/// Where to send you to sign in; plex.tv sends you back to `forward`.
pub async fn login_url(forward: &str) -> Result<String, ApiError> {
    let res = plex_headers(client().post("https://plex.tv/api/v2/pins?strong=true"), "")
        .send()
        .await
        .map_err(|e| unreachable("plex.tv", e))?;
    let body: Value = res.json().await.map_err(|e| bad_gateway(format!("plex.tv: {e}")))?;
    let (id, code) = (body["id"].as_u64().unwrap_or(0), s(&body["code"]));
    if code.is_empty() {
        return Err(bad_gateway("plex.tv didn't start a sign-in"));
    }
    let state = random_hex(8);
    pins().lock().unwrap().insert(state.clone(), Pin { id, at: Instant::now() });
    let forward = format!("{forward}?state={state}");
    let mut url = url::Url::parse("https://app.plex.tv/auth").expect("static");
    let mut frag = url::form_urlencoded::Serializer::new(String::new());
    frag.append_pair("clientID", &client_id())
        .append_pair("code", &code)
        .append_pair("context[device][product]", PRODUCT)
        .append_pair("forwardUrl", &forward);
    url.set_fragment(Some(&format!("?{}", frag.finish())));
    Ok(url.into())
}

/// After plex.tv: picks up the token and adds every music server.
pub async fn finish_login(state: &str) -> Result<usize, ApiError> {
    let pin = pins().lock().unwrap().remove(state);
    let Some(pin) = pin.filter(|p| p.at.elapsed() < Duration::from_secs(900)) else {
        return Err(bad_gateway("This Plex sign-in expired. Try again."));
    };
    let mut token = String::new();
    // plex.tv can take a moment to mark the code as used.
    for _ in 0..10 {
        let res = plex_headers(client().get(format!("https://plex.tv/api/v2/pins/{}", pin.id)), "")
            .send()
            .await
            .map_err(|e| unreachable("plex.tv", e))?;
        let body: Value = res.json().await.unwrap_or_default();
        token = s(&body["authToken"]);
        if !token.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    if token.is_empty() {
        return Err(bad_gateway("Plex didn't finish the sign-in"));
    }
    let res = plex_headers(client().get("https://plex.tv/api/v2/resources?includeHttps=1&includeRelay=1"), &token)
        .send()
        .await
        .map_err(|e| unreachable("plex.tv", e))?;
    let resources: Value = res.json().await.map_err(|e| bad_gateway(format!("plex.tv: {e}")))?;
    let existing = accounts().await;
    let mut added = 0;
    for r in arr(&resources).iter().filter(|r| s(&r["provides"]).split(',').any(|p| p == "server")) {
        // Local addresses first, the relay last.
        let mut conns: Vec<&Value> = arr(&r["connections"]).iter().collect();
        conns.sort_by_key(|c| (c["relay"].as_bool().unwrap_or(false), !c["local"].as_bool().unwrap_or(false)));
        let urls: Vec<String> = conns.iter().map(|c| s(&c["uri"]).trim_end_matches('/').to_owned()).filter(|u| !u.is_empty()).collect();
        let Some(first) = urls.first().cloned() else { continue };
        let server_id = s(&r["clientIdentifier"]);
        let id = existing.iter().find(|a| a.kind == Kind::Plex && a.user_id == server_id).map(|a| a.id.clone()).unwrap_or_else(|| format!("plex-{}", random_hex(4)));
        let mut acc = Account {
            id,
            kind: Kind::Plex,
            name: s(&r["name"]),
            url: first,
            alt_urls: urls,
            token: { let t = s(&r["accessToken"]); if t.is_empty() { token.clone() } else { t } },
            user_id: server_id,
            ..Default::default()
        };
        let _ = pick(&mut acc).await;
        save_account(acc).await?;
        added += 1;
    }
    if added == 0 {
        return Err(bad_gateway("Signed in, but this Plex account has no servers"));
    }
    Ok(added)
}

/// Settings' manual form: an address and an X-Plex-Token in the password
/// field.
pub async fn connect_manual(acc: &mut Account) -> Result<(), ApiError> {
    acc.token = std::mem::take(&mut acc.password);
    if acc.token.is_empty() {
        return Err(bad_gateway("Use Sign in with Plex, or put your X-Plex-Token in the password box"));
    }
    let r = get(acc, "/identity").await?;
    acc.user_id = s(&r["MediaContainer"]["machineIdentifier"]);
    acc.alt_urls = vec![acc.url.clone()];
    Ok(())
}

/// Finds an address that answers right now.
async fn pick(acc: &mut Account) -> Result<(), ApiError> {
    for url in acc.alt_urls.clone() {
        let req = plex_headers(insecure_client().get(format!("{url}/identity")), &acc.token).timeout(Duration::from_secs(4));
        if matches!(req.send().await, Ok(r) if r.status().is_success()) {
            acc.url = url;
            return Ok(());
        }
    }
    Err(bad_gateway(format!("Can't reach {} at any of its addresses", acc.name)))
}

async fn get(acc: &Account, path: &str) -> Result<Value, ApiError> {
    let send = |base: String| plex_headers(acc.http().get(format!("{base}{path}")), &acc.token).send();
    let res = match send(acc.base().to_owned()).await {
        Ok(r) => r,
        // Moved between home and away: find the address that works now.
        Err(e) if acc.alt_urls.len() > 1 => {
            let mut fresh = acc.clone();
            if pick(&mut fresh).await.is_err() {
                return Err(unreachable(&acc.name, e));
            }
            let res = send(fresh.base().to_owned()).await.map_err(|e| unreachable(&acc.name, e))?;
            let _ = save_account(fresh).await;
            res
        }
        Err(e) => return Err(unreachable(&acc.name, e)),
    };
    match res.status() {
        s if s.is_success() => res.json().await.map_err(|e| bad_gateway(format!("{}: unexpected answer ({e})", acc.name))),
        StatusCode::UNAUTHORIZED => Err(bad_gateway(format!("{} refused the sign-in. Sign in with Plex again in Settings.", acc.name))),
        s => Err(bad_gateway(format!("{} answered {s}", acc.name))),
    }
}

fn meta(v: &Value) -> Vec<Item> {
    arr(&v["MediaContainer"]["Metadata"]).iter().filter_map(item).collect()
}

fn item(v: &Value) -> Option<Item> {
    let key = s(&v["ratingKey"]);
    Some(match v["type"].as_str().unwrap_or("") {
        "track" => {
            let part = s(&v["Media"][0]["Part"][0]["key"]);
            if part.is_empty() {
                return None;
            }
            let artist = Some(s(&v["originalTitle"])).filter(|a| !a.is_empty()).unwrap_or_else(|| s(&v["grandparentTitle"]));
            Item::track(part, s(&v["title"]))
                .by(artist)
                .on(s(&v["parentTitle"]))
                .secs(f(&v["duration"]).map(|ms| ms / 1000.0))
                .no(u(&v["index"]))
                .info(s(&v["Media"][0]["audioCodec"]))
        }
        "album" => Item::folder(format!("album:{key}"), s(&v["title"])).by(s(&v["parentTitle"])).items(u(&v["leafCount"])).info(s(&v["year"])),
        "artist" => Item::folder(format!("artist:{key}"), s(&v["title"])).items(u(&v["childCount"])),
        "playlist" => Item::folder(format!("playlist:{key}"), s(&v["title"])).items(u(&v["leafCount"])),
        _ => return None,
    })
}

async fn music_sections(acc: &Account) -> Result<Vec<String>, ApiError> {
    let r = get(acc, "/library/sections").await?;
    Ok(arr(&r["MediaContainer"]["Directory"]).iter().filter(|d| d["type"] == "artist").map(|d| s(&d["key"])).collect())
}

async fn across_sections(acc: &Account, query: &str) -> Result<Listing, ApiError> {
    let mut items = Vec::new();
    for key in music_sections(acc).await? {
        items.extend(meta(&get(acc, &format!("/library/sections/{key}/all?{query}")).await?));
    }
    Ok(Listing::of(items))
}

pub async fn browse(acc: &Account, node: &str) -> Result<Listing, ApiError> {
    let (kind, arg) = node.split_once(':').unwrap_or((node, ""));
    let arg = urlencoding::encode(arg);
    match kind {
        "albums" => across_sections(acc, "type=9&sort=addedAt:desc&X-Plex-Container-Start=0&X-Plex-Container-Size=500").await,
        "artists" => across_sections(acc, "type=8&sort=titleSort").await,
        "album" | "artist" => Ok(Listing::of(meta(&get(acc, &format!("/library/metadata/{arg}/children")).await?))),
        "playlists" => Ok(Listing::of(meta(&get(acc, "/playlists?playlistType=audio").await?))),
        "playlist" => Ok(Listing::of(meta(&get(acc, &format!("/playlists/{arg}/items")).await?))),
        _ => Err(not_found("That list")),
    }
}

pub async fn search(acc: &Account, q: &str) -> Result<Listing, ApiError> {
    let r = get(acc, &format!("/hubs/search?query={}&limit=100", urlencoding::encode(q))).await?;
    let mut items: Vec<Item> = arr(&r["MediaContainer"]["Hub"])
        .iter()
        .filter(|h| matches!(h["type"].as_str(), Some("track" | "album" | "artist")))
        .flat_map(|h| arr(&h["Metadata"]).iter().filter_map(item))
        .collect();
    items.sort_by_key(|i| i.kind == "track");
    Ok(Listing::of(items))
}

pub fn media(acc: &Account, part: &str) -> Upstream {
    Upstream::get(format!("{}{part}?X-Plex-Token={}", acc.base(), acc.token)).header("X-Plex-Client-Identifier", client_id())
}
