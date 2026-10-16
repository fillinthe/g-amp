//! DLNA / UPnP media servers, found on the local network without any
//! setup: Synology Media Server, minidlna, Plex's and Jellyfin's DLNA,
//! Windows Media Player sharing, many routers with a USB disk.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use sha1::{Digest as _, Sha1};

use super::*;

#[derive(Default)]
struct Found {
    servers: Vec<Account>,
    searched: Option<Instant>,
    searching: bool,
}

fn found() -> &'static Mutex<Found> {
    static F: OnceLock<Mutex<Found>> = OnceLock::new();
    F.get_or_init(Default::default)
}

pub fn servers() -> Vec<Account> {
    found().lock().unwrap().servers.clone()
}

pub fn find(id: &str) -> Option<Account> {
    found().lock().unwrap().servers.iter().find(|a| a.id == id).cloned()
}

/// Looks again in the background, at most every few minutes.
pub fn refresh_soon() {
    refresh(false);
}

pub fn refresh(force: bool) {
    {
        let mut f = found().lock().unwrap();
        if f.searching || (!force && f.searched.is_some_and(|t| t.elapsed() < Duration::from_secs(180))) {
            return;
        }
        f.searching = true;
    }
    tokio::spawn(async {
        let servers = discover().await;
        let mut f = found().lock().unwrap();
        // A server that missed one search keeps its place.
        for s in servers {
            f.servers.retain(|o| o.id != s.id);
            f.servers.push(s);
        }
        f.servers.sort_by_key(|a| a.name.to_lowercase());
        f.searched = Some(Instant::now());
        f.searching = false;
    });
}

pub fn searching() -> bool {
    found().lock().unwrap().searching
}

fn ssdp_socket(addr: Option<std::net::Ipv4Addr>) -> std::io::Result<tokio::net::UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    let ip = addr.unwrap_or(std::net::Ipv4Addr::UNSPECIFIED);
    if let Some(a) = addr {
        sock.set_multicast_if_v4(&a)?;
    }
    sock.set_multicast_ttl_v4(2)?;
    sock.bind(&std::net::SocketAddr::from((ip, 0)).into())?;
    sock.set_nonblocking(true)?;
    tokio::net::UdpSocket::from_std(sock.into())
}

/// SSDP: ask the network who serves media, then read each description.
async fn discover() -> Vec<Account> {
    // iOS needs a special Apple entitlement for multicast.
    if cfg!(target_os = "ios") {
        return Vec::new();
    }
    let msg = "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: urn:schemas-upnp-org:device:MediaServer:1\r\n\r\n";
    // Ask on every network this machine is on (Wi-Fi, Ethernet, a VPN to
    // home), not just the default one.
    let mut sockets = Vec::new();
    let addrs: Vec<std::net::Ipv4Addr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .filter_map(|i| match i.ip() {
            std::net::IpAddr::V4(a) => Some(a),
            _ => None,
        })
        .collect();
    for addr in addrs.iter().copied().map(Some).chain([None]) {
        match ssdp_socket(addr) {
            Ok(sock) => {
                for _ in 0..2 {
                    let _ = sock.send_to(msg.as_bytes(), "239.255.255.250:1900").await;
                }
                sockets.push(sock);
            }
            Err(e) => log::debug!("DLNA: no search on {addr:?}: {e}"),
        }
    }
    let mut locations: Vec<String> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(3);
    let listen = sockets.iter().map(|sock| async move {
        let mut found = Vec::new();
        let mut buf = [0u8; 2048];
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            let Ok(Ok((n, _))) = tokio::time::timeout(left, sock.recv_from(&mut buf)).await else { break };
            let text = String::from_utf8_lossy(&buf[..n]);
            if let Some(loc) = text.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case("location")).map(|(_, v)| v.trim().to_owned())) {
                found.push(loc);
            }
        }
        found
    });
    for loc in futures_util::future::join_all(listen).await.into_iter().flatten() {
        if !locations.contains(&loc) {
            locations.push(loc);
        }
    }
    log::info!("DLNA: {} answered", locations.len());
    let mut out = Vec::new();
    for loc in locations {
        let d = describe(&loc).await;
        if let Err(e) = &d {
            log::info!("DLNA: {loc}: {e}");
        }
        if let Ok(Some(acc)) = d {
            if !out.iter().any(|a: &Account| a.id == acc.id) {
                out.push(acc);
            }
        }
    }
    out
}

pub fn parse_description(xml: &str, location: &str) -> Option<(String, String, String)> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let text = |node: roxmltree::Node, name: &str| node.children().find(|c| c.tag_name().name() == name).and_then(|c| c.text()).map(|t| t.trim().to_owned());
    let root = doc.root_element();
    let base = text(root, "URLBase").unwrap_or_else(|| location.to_owned());
    let device = root.descendants().find(|n| n.tag_name().name() == "device")?;
    let name = text(device, "friendlyName").unwrap_or_else(|| "Media server".into());
    let udn = text(device, "UDN").unwrap_or_else(|| location.to_owned());
    let service = root
        .descendants()
        .filter(|n| n.tag_name().name() == "service")
        .find(|s| text(*s, "serviceType").is_some_and(|t| t.contains("ContentDirectory")))?;
    let control = text(service, "controlURL")?;
    let control = url::Url::parse(&base).ok()?.join(&control).ok()?.to_string();
    Some((name, udn, control))
}

async fn describe(location: &str) -> anyhow::Result<Option<Account>> {
    let xml = client().get(location).timeout(Duration::from_secs(4)).send().await?.text().await?;
    let Some((name, udn, control)) = parse_description(&xml, location) else { return Ok(None) };
    let id = format!("dlna-{}", hex::encode(&Sha1::digest(udn.as_bytes())[..4]));
    Ok(Some(Account { id, kind: Kind::Dlna, name, url: control, ..Default::default() }))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

async fn browse_page(acc: &Account, object: &str, start: usize) -> Result<(String, usize, usize), ApiError> {
    let body = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/"><s:Body>
<u:Browse xmlns:u="urn:schemas-upnp-org:service:ContentDirectory:1"><ObjectID>{}</ObjectID><BrowseFlag>BrowseDirectChildren</BrowseFlag><Filter>*</Filter><StartingIndex>{start}</StartingIndex><RequestedCount>500</RequestedCount><SortCriteria></SortCriteria></u:Browse>
</s:Body></s:Envelope>"#,
        xml_escape(object)
    );
    let res = client()
        .post(&acc.url)
        .header("Content-Type", r#"text/xml; charset="utf-8""#)
        .header("SOAPAction", r#""urn:schemas-upnp-org:service:ContentDirectory:1#Browse""#)
        .body(body)
        .send()
        .await
        .map_err(|e| unreachable(&acc.name, e))?;
    if !res.status().is_success() {
        return Err(bad_gateway(format!("{} answered {}", acc.name, res.status())));
    }
    let xml = res.text().await.map_err(|e| unreachable(&acc.name, e))?;
    let doc = roxmltree::Document::parse(&xml).map_err(|e| bad_gateway(format!("{}: {e}", acc.name)))?;
    let field = |name: &str| doc.descendants().find(|n| n.tag_name().name() == name).and_then(|n| n.text()).unwrap_or("").to_owned();
    Ok((field("Result"), field("NumberReturned").parse().unwrap_or(0), field("TotalMatches").parse().unwrap_or(0)))
}

/// DIDL-Lite: the listing format inside a Browse answer.
pub fn parse_didl(didl: &str) -> Vec<Item> {
    let Ok(doc) = roxmltree::Document::parse(didl) else { return Vec::new() };
    let text = |node: roxmltree::Node, name: &str| node.children().find(|c| c.tag_name().name() == name).and_then(|c| c.text()).unwrap_or("").trim().to_owned();
    let mut out = Vec::new();
    for n in doc.root_element().children().filter(|n| n.is_element()) {
        let id = n.attribute("id").unwrap_or("").to_owned();
        let title = text(n, "title");
        match n.tag_name().name() {
            "container" => out.push(Item::folder(format!("c:{id}"), title).items(n.attribute("childCount").and_then(|c| c.parse().ok()))),
            "item" => {
                let class = text(n, "class");
                let res = n
                    .children()
                    .filter(|c| c.tag_name().name() == "res")
                    .find(|r| r.attribute("protocolInfo").is_some_and(|p| p.contains("audio/")) || class.contains("audioItem"));
                let Some(res) = res else { continue };
                let Some(url) = res.text().map(|t| t.trim().to_owned()).filter(|t| t.starts_with("http")) else { continue };
                let artist = Some(text(n, "artist")).filter(|a| !a.is_empty()).unwrap_or_else(|| text(n, "creator"));
                let mime = res.attribute("protocolInfo").and_then(|p| p.split(':').nth(2)).unwrap_or("").to_owned();
                out.push(
                    Item::track(url, title)
                        .by(artist)
                        .on(text(n, "album"))
                        .secs(res.attribute("duration").and_then(clock))
                        .no(text(n, "originalTrackNumber").parse().ok())
                        .info(mime),
                );
            }
            _ => {}
        }
    }
    out
}

pub async fn browse(acc: &Account, node: &str) -> Result<Listing, ApiError> {
    let object = node.strip_prefix("c:").ok_or_else(|| not_found("That list"))?;
    let mut items = Vec::new();
    let mut start = 0;
    loop {
        let (didl, returned, total) = browse_page(acc, object, start).await?;
        items.extend(parse_didl(&didl));
        start += returned;
        if returned == 0 || start >= total || start >= 10_000 {
            break;
        }
    }
    Ok(Listing::of(items).hint("Nothing here."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_device_and_listing() {
        let desc = r#"<root xmlns="urn:schemas-upnp-org:device-1-0"><device><friendlyName>DiskStation</friendlyName><UDN>uuid:1</UDN>
<serviceList><service><serviceType>urn:schemas-upnp-org:service:ConnectionManager:1</serviceType><controlURL>/cm</controlURL></service>
<service><serviceType>urn:schemas-upnp-org:service:ContentDirectory:1</serviceType><controlURL>/ctl/ContentDir</controlURL></service></serviceList></device></root>"#;
        let (name, udn, control) = parse_description(desc, "http://192.168.1.5:50001/desc.xml").unwrap();
        assert_eq!((name.as_str(), udn.as_str()), ("DiskStation", "uuid:1"));
        assert_eq!(control, "http://192.168.1.5:50001/ctl/ContentDir");
        let didl = r#"<DIDL-Lite xmlns="urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:upnp="urn:schemas-upnp-org:metadata-1-0/upnp/">
<container id="64$1" parentID="64" childCount="12"><dc:title>Albums</dc:title><upnp:class>object.container</upnp:class></container>
<item id="t1" parentID="64"><dc:title>Song</dc:title><upnp:artist>Band</upnp:artist><upnp:album>LP</upnp:album><upnp:class>object.item.audioItem.musicTrack</upnp:class>
<res protocolInfo="http-get:*:audio/mpeg:*" duration="0:03:25.000">http://192.168.1.5:50002/m/1.mp3</res></item>
<item id="v1"><dc:title>Clip</dc:title><upnp:class>object.item.videoItem</upnp:class><res protocolInfo="http-get:*:video/mp4:*">http://x/v.mp4</res></item>
</DIDL-Lite>"#;
        let items = parse_didl(didl);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, "c:64$1");
        assert_eq!(items[0].count, Some(12));
        assert_eq!(items[1].id, "http://192.168.1.5:50002/m/1.mp3");
        assert_eq!(items[1].duration, Some(205.0));
        assert_eq!(items[1].artist, "Band");
    }
}

#[cfg(test)]
mod live {
    #[tokio::test]
    #[ignore]
    async fn finds_servers() {
        let _ = env_logger::builder().is_test(true).filter_level(log::LevelFilter::Debug).try_init();
        let found = super::discover().await;
        for a in &found {
            println!("{} {} {}", a.id, a.name, a.url);
        }
    }
}
