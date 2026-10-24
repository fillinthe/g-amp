//! SoundCloud's stream proxy. Tracks come from the web player's API
//! (`soundcloud_web`), as HLS or one file; the player wants one plain,
//! seekable file, so each HLS segment is measured, its per-segment ID3 tag
//! dropped, and the rest served as one virtual file with byte ranges.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use futures_util::StreamExt;
use tokio::sync::Mutex as AsyncMutex;

use crate::http::{ApiError, client, err, parse_range};

// ---- the stream proxy --------------------------------------------------

#[derive(Clone)]
struct Segment {
    url: String,
    skip: u64,
    start: u64,
    len: u64,
}

#[derive(Clone)]
struct Plan {
    segments: Vec<Segment>,
    size: u64,
    mime: &'static str,
    at: Instant,
    heads: Heads,
}

/// A piece's start (its styp, sidx and moof: under 2 KB on SoundCloud) is
/// where the decoder begins each hop while walking to a far-off place. The
/// starts are fetched a batch at a time and kept, so each hop begins at
/// once instead of after a round trip to SoundCloud: half the time of a
/// far seek in a long mix. Capped at about 6 MB.
const HEAD: u64 = 8192;
const WINDOW: usize = 192;
type Heads = Arc<Mutex<HashMap<usize, Vec<u8>>>>;

async fn head(plan: &Plan, n: usize) -> Option<Vec<u8>> {
    if let Some(h) = plan.heads.lock().unwrap().get(&n) {
        return Some(h.clone());
    }
    let want: Vec<usize> = {
        let c = plan.heads.lock().unwrap();
        (n..(n + WINDOW).min(plan.segments.len())).filter(|i| !c.contains_key(i)).collect()
    };
    let fetched: Vec<Option<(usize, Vec<u8>)>> = futures_util::stream::iter(want.into_iter().map(|i| {
        let url = plan.segments[i].url.clone();
        async move {
            let r = client().get(url).header(header::RANGE, format!("bytes=0-{}", HEAD - 1)).send().await.ok()?;
            r.status().is_success().then_some(())?;
            Some((i, r.bytes().await.ok()?.to_vec()))
        }
    }))
    .buffered(32)
    .collect()
    .await;
    let mut c = plan.heads.lock().unwrap();
    if c.len() > 768 {
        c.clear();
    }
    c.extend(fetched.into_iter().flatten());
    c.get(&n).cloned()
}

/// Segment URLs are signed and expire.
const PLAN_TTL: Duration = Duration::from_secs(240);

/// Measured segment sizes per track, kept on disk: a long mix has over a
/// thousand pieces and they never change, so opening it again (even after
/// a restart) doesn't measure them all over, a request each.
const SIZES: &str = "soundcloud-sizes.json";
type Sizes = HashMap<String, Vec<(u64, u64)>>;

fn plans() -> &'static AsyncMutex<HashMap<String, Plan>> {
    static P: OnceLock<AsyncMutex<HashMap<String, Plan>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

pub fn id3_len(head: &[u8]) -> u64 {
    if head.len() < 10 || &head[..3] != b"ID3" {
        return 0;
    }
    let size = ((head[6] as u64 & 0x7f) << 21) | ((head[7] as u64 & 0x7f) << 14) | ((head[8] as u64 & 0x7f) << 7) | (head[9] as u64 & 0x7f);
    let footer = if head[5] & 0x10 != 0 { 10 } else { 0 };
    10 + size + footer
}

async fn measure(url: String) -> Result<(String, u64, u64), ApiError> {
    let res = client().get(&url).header(header::RANGE, "bytes=0-9").send().await?;
    if !res.status().is_success() {
        return Err(err(StatusCode::BAD_GATEWAY, format!("segment answered {}", res.status())));
    }
    let partial = res.status() == StatusCode::PARTIAL_CONTENT;
    let total_from_range = res
        .headers()
        .get(header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit('/').next())
        .and_then(|v| v.parse::<u64>().ok());
    let head = res.bytes().await?;
    let total = if partial { total_from_range.unwrap_or(head.len() as u64) } else { head.len() as u64 };
    Ok((url, id3_len(&head), total))
}

/// Pulls the segment URIs out of a media playlist. A fragmented-MP4
/// stream's init segment (`#EXT-X-MAP`) comes first, so the pieces join
/// into one playable file.
pub fn segment_uris(m3u8_url: &str, body: &str) -> Vec<String> {
    let base = url::Url::parse(m3u8_url).ok();
    body.lines()
        .map(str::trim)
        .filter_map(|l| match l.strip_prefix("#EXT-X-MAP:") {
            Some(attrs) => attrs.split("URI=\"").nth(1).and_then(|r| r.split('"').next()),
            None => Some(l).filter(|l| !l.is_empty() && !l.starts_with('#')),
        })
        .filter_map(|l| match &base {
            Some(b) => b.join(l).ok().map(String::from),
            None => Some(l.to_owned()),
        })
        .collect()
}

/// The pieces a track's stream comes in (HLS segments, or one file), and
/// their format.
async fn pieces(urn: &str) -> Result<(Vec<String>, &'static str), ApiError> {
    let id = urn.strip_prefix("web:").ok_or_else(|| err(StatusCode::NOT_FOUND, "Not a SoundCloud track"))?;
    let (url, hls, mime) = crate::soundcloud_web::stream_url(id).await?;
    if !hls {
        return Ok((vec![url], mime));
    }
    let list = client().get(&url).send().await?.text().await?;
    Ok((segment_uris(&url, &list), mime))
}

async fn build_plan(urn: &str) -> Result<Plan, ApiError> {
    let (uris, mime) = pieces(urn).await?;
    let mut sizes: Sizes = crate::store::read_json(SIZES).await.unwrap_or_default();
    let measured: Vec<(String, u64, u64)> = match sizes.get(urn).filter(|s| s.len() == uris.len()) {
        Some(known) => uris.into_iter().zip(known).map(|(url, &(skip, total))| (url, skip, total)).collect(),
        None => {
            let m: Vec<Result<(String, u64, u64), ApiError>> =
                futures_util::stream::iter(uris.into_iter().map(measure)).buffered(24).collect().await;
            let m = m.into_iter().collect::<Result<Vec<_>, _>>()?;
            // ponytail: the whole file is rewritten per new track; a few KB each, so it's capped, not pruned.
            if sizes.len() >= 200 {
                sizes.clear();
            }
            sizes.insert(urn.to_owned(), m.iter().map(|(_, skip, total)| (*skip, *total)).collect());
            if let Err(e) = crate::store::write_json(SIZES, &sizes, false).await {
                log::warn!("soundcloud sizes not saved: {e}");
            }
            m
        }
    };
    let mut segments = Vec::with_capacity(measured.len());
    let mut offset = 0;
    for (url, skip, total) in measured {
        let len = total.saturating_sub(skip);
        segments.push(Segment { url, skip, start: offset, len });
        offset += len;
    }
    Ok(Plan { segments, size: offset, mime, at: Instant::now(), heads: Default::default() })
}

async fn plan(urn: &str, fresh: bool) -> Result<Plan, ApiError> {
    if !fresh {
        if let Some(p) = plans().lock().await.get(urn).filter(|p| p.at.elapsed() < PLAN_TTL) {
            return Ok(p.clone());
        }
    }
    let p = build_plan(urn).await?;
    plans().lock().await.insert(urn.to_owned(), p.clone());
    Ok(p)
}

pub async fn stream(urn: String, headers: HeaderMap) -> Result<Response, ApiError> {
    let p = plan(&urn, false).await?;
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let (start, end, partial) = match parse_range(range, p.size) {
        Ok(Some((s, e))) => (s, e, true),
        Ok(None) => (0, p.size.saturating_sub(1), false),
        Err(()) => {
            return Ok(Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{}", p.size))
                .body(Body::empty())?);
        }
    };
    let pieces: Vec<(usize, u64, u64)> = p
        .segments
        .iter()
        .enumerate()
        .filter(|(_, s)| s.len > 0 && s.start + s.len > start && s.start <= end)
        .map(|(n, s)| (n, s.skip + start.saturating_sub(s.start), s.skip + (s.len - 1).min(end - s.start)))
        .collect();
    log::debug!("range {start}-{end} ({} bytes): {} piece(s) from #{}", end - start + 1, pieces.len(), pieces.first().map_or(0, |p| p.0));
    let size = p.size;
    let mime = p.mime;
    let body = async_stream(urn, p, pieces);
    let mut res = Response::builder()
        .status(if partial { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK })
        .header(header::CONTENT_TYPE, mime)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, end - start + 1)
        .header(header::CACHE_CONTROL, "no-store");
    if partial {
        res = res.header(header::CONTENT_RANGE, HeaderValue::from_str(&format!("bytes {start}-{end}/{size}"))?);
    }
    Ok(res.body(body)?)
}

fn async_stream(urn: String, plan: Plan, pieces: Vec<(usize, u64, u64)>) -> Body {
    struct St {
        urn: String,
        plan: Plan,
        pieces: std::vec::IntoIter<(usize, u64, u64)>,
        /// The rest of a piece whose start came from the kept starts.
        carry: Option<(usize, u64, u64)>,
        /// The piece being passed on, chunk by chunk as it comes, not held
        /// whole: a reader that stops early (walking) doesn't cost it all.
        cur: Option<reqwest::Response>,
        /// The request's first piece: the one a walking reader starts in.
        first: bool,
    }
    let st = St { urn, plan, pieces: pieces.into_iter(), carry: None, cur: None, first: true };
    let stream = futures_util::stream::unfold(st, |mut st| async move {
        loop {
            if let Some(res) = st.cur.as_mut() {
                match res.chunk().await {
                    Ok(Some(b)) => return Some((Ok::<_, std::io::Error>(b), st)),
                    Ok(None) => st.cur = None,
                    Err(_) => return None,
                }
                continue;
            }
            let (n, from, to) = match st.carry.take() {
                Some(rest) => rest,
                None => st.pieces.next()?,
            };
            // Starts in a piece's start: that part comes from the kept
            // starts; the rest follows as its own fetch.
            if std::mem::take(&mut st.first) && from < HEAD {
                if let Some(h) = head(&st.plan, n).await {
                    let end = (to + 1).min(HEAD).min(h.len() as u64);
                    if from < end {
                        if end <= to {
                            st.carry = Some((n, end, to));
                        }
                        return Some((Ok(Bytes::copy_from_slice(&h[from as usize..end as usize])), st));
                    }
                }
            }
            log::debug!("fetch #{n} {from}-{to}");
            let fetch = |url: String| async move { client().get(url).header(header::RANGE, format!("bytes={from}-{to}")).send().await };
            let mut res = fetch(st.plan.segments[n].url.clone()).await.ok()?;
            if matches!(res.status(), StatusCode::FORBIDDEN | StatusCode::GONE) {
                // The signed URL expired mid-play: measure again and go on.
                st.plan = crate::soundcloud::plan(&st.urn, true).await.ok()?;
                res = fetch(st.plan.segments[n].url.clone()).await.ok()?;
            }
            if !res.status().is_success() {
                return None;
            }
            if res.status() == StatusCode::OK {
                // No ranges upstream: the whole piece came; its part is cut out.
                let bytes = res.bytes().await.ok()?;
                let bytes = bytes.slice(from as usize..(to as usize + 1).min(bytes.len()));
                return Some((Ok(bytes), st));
            }
            st.cur = Some(res);
        }
    });
    Body::from_stream(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `cargo test -p gamp-core sizes_come_from_disk -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn sizes_come_from_disk_the_second_time() {
        let dir = std::env::temp_dir().join(format!("gamp-sc-sizes-{}", std::process::id()));
        crate::store::init(dir.clone(), None);
        let items = crate::soundcloud_web::resolve("https://soundcloud.com/schirmchendrink/bluebird-migueliste").await.unwrap();
        let urn = items[0].id.clone();
        let t = Instant::now();
        let first = build_plan(&urn).await.unwrap();
        let measured = t.elapsed();
        let t = Instant::now();
        let second = build_plan(&urn).await.unwrap();
        let cached = t.elapsed();
        println!("{} segments: measured in {measured:.1?}, from disk in {cached:.1?}", first.segments.len());
        assert_eq!(first.size, second.size);
        assert!(cached < measured / 2);
        // A walk, as the decoder does to reach a far-off place: a piece's start.
        let t = Instant::now();
        for seg in second.segments.iter().skip(1).take(100) {
            let mut h = HeaderMap::new();
            h.insert(header::RANGE, HeaderValue::from_str(&format!("bytes={}-", seg.start)).unwrap());
            let res = stream(urn.clone(), h).await.unwrap();
            let first = axum::body::to_bytes(Body::from_stream(res.into_body().into_data_stream().take(1)), usize::MAX).await.unwrap();
            assert!(first.len() >= 1900 && &first[4..8] == b"styp", "{} bytes", first.len());
        }
        println!("walking 100 pieces: {:.1?}", t.elapsed());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fmp4_playlists_start_with_the_init_segment() {
        let list = "#EXTM3U\n#EXT-X-MAP:URI=\"https://cdn/x/init.mp4?s=1\"\n#EXTINF:10,\ndata000.m4s\n#EXTINF:10,\ndata001.m4s\n#EXT-X-ENDLIST\n";
        assert_eq!(segment_uris("https://cdn/x/playlist.m3u8", list), ["https://cdn/x/init.mp4?s=1", "https://cdn/x/data000.m4s", "https://cdn/x/data001.m4s"]);
    }

    #[test]
    fn id3_lengths() {
        assert_eq!(id3_len(b"\xff\xfb\x90\x00........"), 0);
        // size 0x000201 synchsafe = 257, plus the 10-byte header
        assert_eq!(id3_len(b"ID3\x04\x00\x00\x00\x00\x02\x01"), 10 + 257);
        assert_eq!(id3_len(b"ID3\x04\x00\x10\x00\x00\x00\x05"), 10 + 5 + 10);
    }

    #[test]
    fn playlist_uris_resolve() {
        let body = "#EXTM3U\n#EXTINF:5,\nseg0.mp3\n#EXTINF:5,\nhttps://cdn.example/x/seg1.mp3\n#EXT-X-ENDLIST\n";
        assert_eq!(
            segment_uris("https://cdn.example/a/list.m3u8?sig=1", body),
            vec!["https://cdn.example/a/seg0.mp3".to_owned(), "https://cdn.example/x/seg1.mp3".to_owned()]
        );
    }
}
