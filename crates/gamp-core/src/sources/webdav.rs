//! WebDAV: Synology's WebDAV Server package, Nextcloud and ownCloud,
//! pCloud, Koofr, QNAP, Apache, rclone... Browsed folder by folder, so
//! nothing is downloaded until you play it.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use reqwest::Method;

use super::*;

const PROPFIND: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:getcontentlength/><d:displayname/></d:prop></d:propfind>"#;

fn basic(acc: &Account) -> Option<String> {
    (!acc.username.is_empty()).then(|| format!("Basic {}", STANDARD.encode(format!("{}:{}", acc.username, acc.password))))
}

/// The server's origin (scheme, host, port) and the folder path you gave.
fn split(acc: &Account) -> (String, String) {
    let u = url::Url::parse(&acc.url).expect("checked when added");
    let origin = u.origin().ascii_serialization();
    let path = u.path().trim_end_matches('/').to_owned();
    (origin, path)
}

struct Entry {
    href: String,
    name: String,
    dir: bool,
    size: Option<u64>,
}

fn parse_multistatus(xml: &str, origin: &str) -> Result<Vec<Entry>, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in doc.descendants().filter(|n| n.tag_name().name() == "response") {
        let Some(href) = r.descendants().find(|n| n.tag_name().name() == "href").and_then(|n| n.text()) else { continue };
        // Some servers give the full URL, most the path.
        let href = href.trim().strip_prefix(origin).unwrap_or(href.trim()).to_owned();
        let dir = r.descendants().any(|n| n.tag_name().name() == "collection");
        let size = r.descendants().find(|n| n.tag_name().name() == "getcontentlength").and_then(|n| n.text()).and_then(|t| t.trim().parse().ok());
        let raw_name = href.trim_end_matches('/').rsplit('/').next().unwrap_or("").to_owned();
        let name = urlencoding::decode(&raw_name).map(|c| c.into_owned()).unwrap_or(raw_name);
        out.push(Entry { href, name, dir, size });
    }
    Ok(out)
}

async fn list(acc: &Account, path: &str) -> Result<Vec<Entry>, ApiError> {
    let (origin, _) = split(acc);
    let mut req = acc
        .http()
        .request(Method::from_bytes(b"PROPFIND").expect("static"), format!("{origin}{path}"))
        .header("Depth", "1")
        .header("Content-Type", "application/xml; charset=utf-8")
        .body(PROPFIND);
    if let Some(b) = basic(acc) {
        req = req.header("Authorization", b);
    }
    let res = req.send().await.map_err(|e| unreachable(&acc.name, e))?;
    match res.status().as_u16() {
        207 => {}
        401 | 403 => return Err(bad_gateway(format!("{}: wrong user name or password", acc.name))),
        404 => return Err(not_found("That folder")),
        405 => return Err(bad_gateway("That address isn't a WebDAV folder. For Nextcloud use https://your.cloud/remote.php/dav/files/USERNAME")),
        s => return Err(bad_gateway(format!("{} answered {s}", acc.name))),
    }
    let xml = res.text().await.map_err(|e| unreachable(&acc.name, e))?;
    let mut entries = parse_multistatus(&xml, &origin).map_err(|e| bad_gateway(format!("{}: unreadable folder listing ({e})", acc.name)))?;
    // The folder itself comes back first.
    let me = path.trim_end_matches('/');
    entries.retain(|e| {
        let h = e.href.trim_end_matches('/');
        h != me && urlencoding::decode(h).map(|d| d != urlencoding::decode(me).unwrap_or_default()).unwrap_or(true)
    });
    Ok(entries)
}

pub async fn connect(acc: &mut Account) -> Result<(), ApiError> {
    let (_, path) = split(acc);
    list(acc, &format!("{path}/")).await?;
    Ok(())
}

pub async fn browse(acc: &Account, node: &str) -> Result<Listing, ApiError> {
    let path = match node.strip_prefix("dir:") {
        Some(p) => p.to_owned(),
        None if node == "files" => format!("{}/", split(acc).1),
        None => return Err(not_found("That list")),
    };
    let mut entries = list(acc, &path).await?;
    entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    let folder = urlencoding::decode(path.trim_end_matches('/').rsplit('/').next().unwrap_or("")).map(|c| c.into_owned()).unwrap_or_default();
    let items = entries
        .into_iter()
        .filter(|e| !e.name.starts_with('.') && !e.name.starts_with('@') && !e.name.starts_with('#'))
        .filter_map(|e| {
            if e.dir {
                let href = if e.href.ends_with('/') { e.href } else { format!("{}/", e.href) };
                Some(Item::folder(format!("dir:{href}"), e.name))
            } else if is_audio(&e.name) {
                let mb = e.size.map(|s| format!("{:.1} MB", s as f64 / 1e6)).unwrap_or_default();
                Some(Item::track(e.href, stem(&e.name)).on(folder.clone()).info(mb))
            } else {
                None
            }
        })
        .collect();
    Ok(Listing::of(items).hint("No folders or music files here."))
}

pub fn media(acc: &Account, href: &str) -> Upstream {
    let (origin, _) = split(acc);
    let mut up = Upstream::get(format!("{origin}{href}"));
    if let Some(b) = basic(acc) {
        up = up.header("Authorization", b);
    }
    up
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_listings() {
        let xml = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:">
 <d:response><d:href>/music/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>
 <d:response><d:href>http://nas:5005/music/Blue%20Album/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>
 <d:response><d:href>/music/Say%20It.mp3</d:href><d:propstat><d:prop><d:resourcetype/><d:getcontentlength>4000000</d:getcontentlength></d:prop></d:propstat></d:response>
</d:multistatus>"#;
        let e = parse_multistatus(xml, "http://nas:5005").unwrap();
        assert_eq!(e.len(), 3);
        assert_eq!(e[1].href, "/music/Blue%20Album/");
        assert_eq!(e[1].name, "Blue Album");
        assert!(e[1].dir && !e[2].dir);
        assert_eq!(e[2].name, "Say It.mp3");
        assert_eq!(e[2].size, Some(4_000_000));
    }
}
