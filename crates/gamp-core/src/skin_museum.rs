//! The Winamp Skin Museum (skins.webamp.org): about 100,000 classic skins,
//! listed and searched through its public API and downloaded when picked.
//! Skins stay their artists'; Gamp fetches them on demand, never ships them.

use std::path::{Path, PathBuf};

use axum::http::StatusCode;
use rand::Rng;
use serde_json::{Value, json};

use crate::http::{ApiError, client, err};
use crate::sources::{Item, s};

const API: &str = "https://api.webamp.org/graphql";
const FIELDS: &str = "md5 filename download_url nsfw";
/// The library rows that are skins (Use skin, not Play).
pub const KIND: &str = "skin";

fn fail(e: impl std::fmt::Display) -> ApiError {
    err(StatusCode::BAD_GATEWAY, format!("Skin Museum: {e}"))
}

async fn query(q: String, vars: Value) -> Result<Value, ApiError> {
    let res = client().post(API).json(&json!({ "query": q, "variables": vars })).send().await.map_err(fail)?;
    let v: Value = res.json().await.map_err(fail)?;
    if let Some(e) = v["errors"][0]["message"].as_str() {
        return Err(fail(e));
    }
    Ok(v["data"].clone())
}

/// "Gold_Metal-v2.wsz" reads "Gold Metal-v2".
pub fn name(file: &str) -> String {
    let stem = file.rsplit_once('.').map_or(file, |(a, _)| a);
    stem.replace('_', " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

fn item(v: &Value) -> Option<Item> {
    // The museum flags some skins as not safe for work; they stay out.
    if v["nsfw"] == true || v["download_url"].as_str().is_none() {
        return None;
    }
    let file = s(&v["filename"]);
    let mut i = Item { kind: KIND.into(), id: s(&v["md5"]), title: name(&file), album: file, ..Default::default() };
    i.url = Some(s(&v["download_url"]));
    i.info = "From the Winamp Skin Museum, skins.webamp.org".into();
    Some(i)
}

fn items(v: &Value) -> Vec<Item> {
    v.as_array().map(|a| a.iter().filter_map(item).collect()).unwrap_or_default()
}

/// The museum's own order: the best-loved first.
pub async fn popular() -> Result<Vec<Item>, ApiError> {
    let d = query(format!("{{ skins(first: 500, sort: MUSEUM) {{ nodes {{ {FIELDS} }} }} }}"), json!({})).await?;
    Ok(items(&d["skins"]["nodes"]))
}

pub async fn search(q: &str) -> Result<Vec<Item>, ApiError> {
    let d = query(format!("query($q: String!) {{ search_classic_skins(query: $q, first: 300, nsfw: false) {{ {FIELDS} }} }}"), json!({ "q": q })).await?;
    Ok(items(&d["search_classic_skins"]))
}

/// One of the 2,000 best-loved skins, at random.
pub async fn random() -> Result<Item, ApiError> {
    for _ in 0..5 {
        let offset = rand::thread_rng().gen_range(0..2000);
        let d = query(format!("query($o: Int) {{ skins(first: 1, offset: $o, sort: MUSEUM) {{ nodes {{ {FIELDS} }} }} }}"), json!({ "o": offset })).await?;
        if let Some(i) = items(&d["skins"]["nodes"]).pop() {
            return Ok(i);
        }
    }
    Err(fail("no skin came back"))
}

/// Downloads a skin into `dir/<md5>/<file>` (once; then it's at hand).
pub async fn fetch(i: &Item, dir: &Path) -> Result<PathBuf, ApiError> {
    let safe = |t: &str| t.chars().filter(|c| c.is_alphanumeric() || " -_.()[]".contains(*c)).collect::<String>();
    let file = match safe(&i.album) {
        f if f.trim_matches('.').is_empty() => "skin.wsz".to_owned(),
        f => f,
    };
    let to = dir.join(safe(&i.id)).join(file);
    if to.exists() {
        return Ok(to);
    }
    let url = i.url.as_deref().ok_or_else(|| fail("no download"))?;
    let res = client().get(url).send().await.map_err(fail)?;
    if !res.status().is_success() {
        return Err(fail(res.status()));
    }
    let bytes = res.bytes().await.map_err(fail)?;
    tokio::fs::create_dir_all(to.parent().unwrap_or(dir)).await.map_err(fail)?;
    tokio::fs::write(&to, &bytes).await.map_err(fail)?;
    Ok(to)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skin_names_read_like_titles() {
        assert_eq!(name("Gold_Metal__v2.wsz"), "Gold Metal v2");
        assert_eq!(name("base-2.91.wsz"), "base-2.91");
        assert!(item(&json!({ "md5": "a", "filename": "x.wsz", "download_url": "u", "nsfw": true })).is_none());
        assert_eq!(item(&json!({ "md5": "a", "filename": "x.wsz", "download_url": "u", "nsfw": false })).unwrap().kind, KIND);
    }
}
