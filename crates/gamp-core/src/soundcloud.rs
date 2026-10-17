//! SoundCloud's stream proxy. Tracks come from the web player's API
//! (`soundcloud_web`), as HLS or one file; the player wants one plain,
//! seekable file, so each HLS segment is measured, its per-segment ID3 tag
//! dropped, and the rest served as one virtual file with byte ranges.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use axum::body::Body;
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
}

/// Segment URLs are signed and expire.
const PLAN_TTL: Duration = Duration::from_secs(240);

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
    let measured: Vec<Result<(String, u64, u64), ApiError>> =
        futures_util::stream::iter(uris.into_iter().map(measure)).buffered(24).collect().await;
    let mut segments = Vec::with_capacity(measured.len());
    let mut offset = 0;
    for m in measured {
        let (url, skip, total) = m?;
        let len = total.saturating_sub(skip);
        segments.push(Segment { url, skip, start: offset, len });
        offset += len;
    }
    Ok(Plan { segments, size: offset, mime, at: Instant::now() })
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
    let stream = futures_util::stream::unfold((urn, plan, pieces.into_iter()), |(urn, mut plan, mut pieces)| async move {
        let (n, from, to) = pieces.next()?;
        let fetch = |url: String| async move {
            client().get(url).header(header::RANGE, format!("bytes={from}-{to}")).send().await
        };
        let mut res = fetch(plan.segments[n].url.clone()).await.ok()?;
        if matches!(res.status(), StatusCode::FORBIDDEN | StatusCode::GONE) {
            // The signed URL expired mid-play: measure again and go on.
            plan = crate::soundcloud::plan(&urn, true).await.ok()?;
            res = fetch(plan.segments[n].url.clone()).await.ok()?;
        }
        if !res.status().is_success() {
            return None;
        }
        let whole = res.status() == StatusCode::OK;
        let mut bytes = res.bytes().await.ok()?;
        if whole {
            bytes = bytes.slice(from as usize..(to as usize + 1).min(bytes.len()));
        }
        Some((Ok::<_, std::io::Error>(bytes), (urn, plan, pieces)))
    });
    Body::from_stream(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

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
