//! Favorites: tracks, stations, albums, playlists and skins from any
//! source, in one list. Each is kept with the source it came from (its id
//! is `source\u{1f}id`), so a favorite folder opens from that source.

use crate::catalog;
use crate::http::ApiError;
use crate::sources::{Item, Listing, not_found};
use crate::store;

const FILE: &str = "favorites.json";
const SEP: char = '\u{1f}';

async fn list() -> Vec<Item> {
    store::read_json::<Vec<Item>>(FILE).await.unwrap_or_default()
}

/// The list itself (`favorites`), or a favorite folder opened from its source.
pub async fn browse(node: &str) -> Result<Listing, ApiError> {
    if node == "favorites" {
        return Ok(Listing::of(list().await)
            .with("fav:remove", "Remove")
            .hint("Right-click anything in the library and choose Add to Favorites."));
    }
    let (sid, id) = node.split_once(SEP).ok_or_else(|| not_found("That favorite"))?;
    let mut l = Box::pin(catalog::browse(sid, id)).await?;
    // What's inside opens from the same source.
    for i in &mut l.items {
        i.id = format!("{sid}{SEP}{}", i.id);
    }
    l.actions.clear();
    Ok(l)
}

/// `fav:add` the items, listed by `sid`, or `fav:remove` them.
pub async fn action(sid: &str, action: &str, items: Vec<Item>) -> Result<String, ApiError> {
    let mut favs = list().await;
    let n = items.len();
    match action {
        "fav:add" => {
            for mut i in items {
                if !i.id.contains(SEP) {
                    i.id = format!("{sid}{SEP}{}", i.id);
                }
                if !favs.iter().any(|f| f.id == i.id) {
                    favs.push(i);
                }
            }
        }
        "fav:remove" => favs.retain(|f| !items.iter().any(|i| i.id == f.id)),
        _ => return Err(not_found("That action")),
    }
    store::write_json(FILE, &favs, false).await?;
    Ok(if action == "fav:add" { format!("Added {n} to Favorites") } else { format!("Removed {n}") })
}
