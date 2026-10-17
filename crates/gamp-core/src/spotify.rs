//! Spotify: one browser approval, then librespot plays here.
//!
//! The approval uses librespot's client ID with PKCE. Spotify only accepts
//! that client's fixed callback, http://127.0.0.1:8898/login, so a tiny
//! listener opens on that port for the sign-in and closes after. The same
//! grant reads the library through the Web API (the page asks this server
//! for a current token) and logs librespot in; librespot then keeps its own
//! reusable credential in the data folder.
//!
//! Playback: `/media/spotify/<id>.wav` is served as a seekable WAV file of
//! 16-bit stereo PCM at 44.1 kHz, decoded by a librespot player whose sink
//! writes into the HTTP response. To the Webamp page a Spotify song is just
//! another file, so the equalizer and every visualizer see the real music.
//! A seek arrives as a byte range and starts a player at that position.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::Query;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Redirect, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use bytes::Bytes;
use librespot_core::authentication::Credentials;
use librespot_core::cache::Cache;
use librespot_core::config::SessionConfig;
use librespot_core::session::Session;
use librespot_core::spotify_uri::SpotifyUri;
use librespot_metadata::Metadata;
use librespot_playback::audio_backend::{Sink, SinkError, SinkResult};
use librespot_playback::config::{Bitrate, PlayerConfig};
use librespot_playback::convert::Converter;
use librespot_playback::decoder::AudioPacket;
use librespot_playback::mixer::NoOpVolume;
use librespot_playback::player::{Player, PlayerEvent};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex as AsyncMutex, mpsc};

use crate::http::{ApiError, client, err, parse_range};
use crate::store;

/// librespot's own client ID and the callback Spotify registered for it.
const CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";
const CALLBACK_PORT: u16 = 8898;
const REDIRECT_URI: &str = "http://127.0.0.1:8898/login";
const SCOPES: &str = "streaming user-read-private user-read-email user-library-read playlist-read-private playlist-read-collaborative";
const TOKENS: &str = "spotify-tokens.json";
/// librespot's client ID is shared by every librespot-based player, so
/// its Web API quota is always used up (429). Liked Songs, playlists and
/// search use the person's own Spotify app instead when they add one.
const LIBRARY_TOKENS: &str = "spotify-library-tokens.json";
const LIBRARY_SCOPES: &str = "user-read-private user-library-read playlist-read-private playlist-read-collaborative";

const RATE: u64 = 44_100;
const BYTES_PER_FRAME: u64 = 4; // 16-bit stereo
const HEADER: u64 = 44;

#[derive(Clone, Default, Serialize, Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: String,
    expires_at: u64,
    /// The client the tokens belong to (library tokens only).
    #[serde(default)]
    client_id: String,
}

struct Pending {
    verifier: String,
    state: String,
    at: Instant,
    /// Whose sign-in: librespot's (playback) or the person's own app.
    client_id: String,
    library: bool,
}

fn pending() -> &'static Mutex<Option<Pending>> {
    static P: OnceLock<Mutex<Option<Pending>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

fn token_lock() -> &'static AsyncMutex<()> {
    static L: OnceLock<AsyncMutex<()>> = OnceLock::new();
    L.get_or_init(|| AsyncMutex::new(()))
}

fn session_cell() -> &'static AsyncMutex<Option<Session>> {
    static S: OnceLock<AsyncMutex<Option<Session>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

/// Where the app's own page lives, for the redirect back after sign-in.
static APP_ORIGIN: OnceLock<String> = OnceLock::new();

pub fn set_app_origin(origin: String) {
    let _ = APP_ORIGIN.set(origin);
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn random(n: usize) -> String {
    let mut buf = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

fn cache() -> anyhow::Result<Cache> {
    let dir = store::data_dir().join("spotify");
    Ok(Cache::new(Some(dir.clone()), None, Some(dir.join("audio")), Some(512 * 1024 * 1024))?)
}

pub async fn connected() -> bool {
    let tokens = store::read_json::<Tokens>(TOKENS).await.unwrap_or_default();
    !tokens.refresh_token.is_empty()
}

/// The Client ID of the person's own Spotify app, if they added one.
async fn own_client_id() -> Option<String> {
    Some(store::config().await.spotify_client_id).filter(|id| !id.is_empty())
}

/// Signed in with the person's own app, for the library.
pub async fn library_connected() -> bool {
    let Some(id) = own_client_id().await else { return false };
    let t = store::read_json::<Tokens>(LIBRARY_TOKENS).await.unwrap_or_default();
    !t.refresh_token.is_empty() && t.client_id == id
}

pub const fn redirect_uri() -> &'static str {
    REDIRECT_URI
}

// ---- sign-in ---------------------------------------------------------

/// Sign-in for playback (librespot's client).
pub async fn login_url() -> Result<String, ApiError> {
    authorize_url(CLIENT_ID.to_owned(), false).await
}

/// Sign-in with the person's own app, for Liked Songs, playlists and search.
pub async fn library_login_url() -> Result<String, ApiError> {
    let id = own_client_id().await.ok_or_else(|| err(StatusCode::BAD_REQUEST, "Save your Spotify Client ID first (Settings › Spotify)."))?;
    authorize_url(id, true).await
}

async fn authorize_url(client_id: String, library: bool) -> Result<String, ApiError> {
    ensure_callback_listener().await?;
    authorize_url_now(client_id, library)
}

/// The authorize address, with the callback listener already open.
fn authorize_url_now(client_id: String, library: bool) -> Result<String, ApiError> {
    let verifier = random(64);
    let state = random(16);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url = url::Url::parse("https://accounts.spotify.com/authorize")?;
    url.query_pairs_mut()
        .append_pair("client_id", &client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("code_challenge_method", "S256")
        .append_pair("code_challenge", &challenge)
        .append_pair("scope", if library { LIBRARY_SCOPES } else { SCOPES })
        .append_pair("state", &state);
    *pending().lock().unwrap() = Some(Pending { verifier, state, at: Instant::now(), client_id, library });
    Ok(url.into())
}

/// Opens 127.0.0.1:8898 for the callback; it closes itself after ten
/// minutes or one sign-in. Another app holding the port is reported.
async fn ensure_callback_listener() -> Result<(), ApiError> {
    static OPEN: Mutex<bool> = Mutex::new(false);
    if *OPEN.lock().unwrap() {
        return Ok(());
    }
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", CALLBACK_PORT)).await.map_err(|e| {
        err(
            StatusCode::CONFLICT,
            format!("Port {CALLBACK_PORT} is in use ({e}). Close the other Spotify app that holds it, then try again."),
        )
    })?;
    *OPEN.lock().unwrap() = true;
    let (done_tx, mut done_rx) = mpsc::channel::<()>(1);
    let app = axum::Router::new().route(
        "/login",
        axum::routing::get(move |Query(q): Query<std::collections::HashMap<String, String>>| {
            let done_tx = done_tx.clone();
            async move {
                let origin = APP_ORIGIN.get().cloned().unwrap_or_default();
                let result = match finish_login(&q).await {
                    // Playback is signed in; go straight on to the library sign-in.
                    Ok(Some(next)) => return Redirect::to(&next),
                    Ok(None) => Ok(()),
                    Err(e) => Err(e.1),
                };
                let target = format!("{origin}{}", crate::shell::after_sign_in("Spotify", result));
                let _ = done_tx.try_send(());
                Redirect::to(&target)
            }
        }),
    );
    tokio::spawn(async move {
        let server = axum::serve(listener, app).with_graceful_shutdown(async move {
            tokio::select! {
                _ = done_rx.recv() => tokio::time::sleep(Duration::from_secs(2)).await,
                _ = tokio::time::sleep(Duration::from_secs(600)) => {}
            }
        });
        let _ = server.await;
        *OPEN.lock().unwrap() = false;
    });
    Ok(())
}

/// Finishes a sign-in; `Some(url)` is a further sign-in to open.
async fn finish_login(q: &std::collections::HashMap<String, String>) -> Result<Option<String>, ApiError> {
    if let Some(e) = q.get("error") {
        return Err(err(StatusCode::UNAUTHORIZED, format!("Spotify sign-in: {e}")));
    }
    let p = pending().lock().unwrap().take();
    let Some(p) = p.filter(|p| p.at.elapsed() < Duration::from_secs(600) && Some(&p.state) == q.get("state")) else {
        return Err(err(StatusCode::BAD_REQUEST, "This Spotify sign-in expired. Try again."));
    };
    let code = q.get("code").cloned().unwrap_or_default();
    let file = if p.library { LIBRARY_TOKENS } else { TOKENS };
    token_request(
        file,
        &p.client_id,
        &[("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", REDIRECT_URI), ("client_id", &p.client_id), ("code_verifier", &p.verifier)],
    )
    .await?;
    if p.library {
        return Ok(None);
    }
    // Sign librespot in now, so the first song starts without a wait.
    *session_cell().lock().await = None;
    let _ = std::fs::remove_file(store::data_dir().join("spotify").join("credentials.json"));
    session().await?;
    if let Some(id) = own_client_id().await {
        if !library_connected().await {
            return Ok(Some(authorize_url_now(id, true)?));
        }
    }
    Ok(None)
}

async fn token_request(file: &str, client_id: &str, params: &[(&str, &str)]) -> Result<(), ApiError> {
    let res = client().post("https://accounts.spotify.com/api/token").form(params).send().await?;
    let status = res.status();
    let body: Value = res.json().await.unwrap_or_default();
    if !status.is_success() {
        let why = body["error_description"].as_str().or(body["error"].as_str()).unwrap_or("unknown error");
        if body["error"] == "invalid_grant" {
            let _ = store::write_json(file, &Tokens::default(), true).await;
        }
        return Err(err(StatusCode::UNAUTHORIZED, format!("Spotify sign-in failed: {why}")));
    }
    let old = store::read_json::<Tokens>(file).await.unwrap_or_default();
    let tokens = Tokens {
        access_token: body["access_token"].as_str().unwrap_or_default().to_owned(),
        refresh_token: body["refresh_token"].as_str().map(str::to_owned).unwrap_or(old.refresh_token),
        expires_at: now_ms() + body["expires_in"].as_u64().unwrap_or(3600) * 1000,
        client_id: client_id.to_owned(),
    };
    store::write_json(file, &tokens, true).await?;
    Ok(())
}

pub async fn access_token() -> Result<(String, u64), ApiError> {
    let _guard = token_lock().lock().await;
    let t = store::read_json::<Tokens>(TOKENS).await.unwrap_or_default();
    if t.refresh_token.is_empty() {
        return Err(err(StatusCode::UNAUTHORIZED, "Connect Spotify in Settings."));
    }
    if now_ms() + 60_000 < t.expires_at {
        return Ok((t.access_token, t.expires_at));
    }
    token_request(TOKENS, CLIENT_ID, &[("grant_type", "refresh_token"), ("refresh_token", &t.refresh_token), ("client_id", CLIENT_ID)]).await?;
    let t = store::read_json::<Tokens>(TOKENS).await.unwrap_or_default();
    Ok((t.access_token, t.expires_at))
}

/// A token for the Web API (library, playlists, search): the person's own
/// app when signed in with it, otherwise the playback sign-in.
pub async fn api_token() -> Result<(String, u64, bool), ApiError> {
    if library_connected().await {
        let _guard = token_lock().lock().await;
        let t = store::read_json::<Tokens>(LIBRARY_TOKENS).await.unwrap_or_default();
        if now_ms() + 60_000 < t.expires_at {
            return Ok((t.access_token, t.expires_at, true));
        }
        token_request(LIBRARY_TOKENS, &t.client_id, &[("grant_type", "refresh_token"), ("refresh_token", &t.refresh_token), ("client_id", &t.client_id)]).await?;
        let t = store::read_json::<Tokens>(LIBRARY_TOKENS).await.unwrap_or_default();
        return Ok((t.access_token, t.expires_at, true));
    }
    let (t, at) = access_token().await?;
    Ok((t, at, false))
}

pub async fn logout() -> anyhow::Result<()> {
    if let Some(s) = session_cell().lock().await.take() {
        s.shutdown();
    }
    let _ = std::fs::remove_file(store::data_dir().join("spotify").join("credentials.json"));
    store::write_json(LIBRARY_TOKENS, &Tokens::default(), true).await?;
    store::write_json(TOKENS, &Tokens::default(), true).await
}

// ---- the librespot session -------------------------------------------

async fn session() -> Result<Session, ApiError> {
    let mut cell = session_cell().lock().await;
    if let Some(s) = cell.as_ref().filter(|s| !s.is_invalid()) {
        return Ok(s.clone());
    }
    let cache = cache()?;
    let cached = cache.credentials();
    let config = SessionConfig { client_id: CLIENT_ID.to_owned(), ..SessionConfig::default() };
    let session = Session::new(config.clone(), Some(cache));
    let attempt = match cached {
        Some(c) => session.connect(c, true).await,
        None => Err(librespot_core::Error::unauthenticated("no saved credential")),
    };
    let session = match attempt {
        Ok(()) => session,
        Err(_) => {
            // The saved credential is missing or refused: use the grant.
            let (token, _) = access_token().await?;
            let fresh = Session::new(config, Some(cache_or_err()?));
            fresh
                .connect(Credentials::with_access_token(token), true)
                .await
                .map_err(|e| err(StatusCode::UNAUTHORIZED, format!("Spotify playback sign-in failed: {e}")))?;
            fresh
        }
    };
    *cell = Some(session.clone());
    Ok(session)
}

fn cache_or_err() -> Result<Cache, ApiError> {
    Ok(cache()?)
}

// ---- the library, through the player session --------------------------
//
// The Web API needs a developer app (and Spotify caps those at 5 users),
// so Liked Songs, playlists and search are read the way Spotify's own
// apps read them. Results come back in the Web API's shape, so the
// catalog maps them the same way either way.

fn lib_err(e: impl std::fmt::Display) -> ApiError {
    err(StatusCode::BAD_GATEWAY, format!("Spotify: {e}"))
}

/// Web API-shaped tracks for `uris`, in order (unplayable ones dropped).
async fn tracks_for(session: &Session, uris: Vec<String>) -> Vec<Value> {
    use futures_util::StreamExt;
    futures_util::stream::iter(uris)
        .map(|u| async move {
            let uri = SpotifyUri::from_uri(&u).ok()?;
            let t = librespot_metadata::Track::get(session, &uri).await.ok()?;
            let artists: Vec<Value> = t.artists.iter().map(|a| serde_json::json!({ "name": a.name })).collect();
            Some(serde_json::json!({ "id": uri.to_id().ok()?, "type": "track", "name": t.name, "artists": artists, "album": { "name": t.album.name }, "duration_ms": t.duration }))
        })
        .buffered(32)
        .filter_map(|t| async { t })
        .collect()
        .await
}

/// The track URIs of a context, following its pages.
async fn context_uris(session: &Session, uri: &str, max: usize) -> Result<Vec<String>, ApiError> {
    use librespot_protocol::context_page::ContextPage;
    let ctx = session.spclient().get_context(uri).await.map_err(lib_err)?;
    let mut pages: std::collections::VecDeque<ContextPage> = ctx.pages.into_iter().collect();
    let mut out = Vec::new();
    // ponytail: 200 pages caps a runaway chain of next-page links.
    for _ in 0..200 {
        let Some(p) = pages.pop_front() else { break };
        out.extend(p.tracks.iter().map(|t| t.uri().to_owned()).filter(|u| u.starts_with("spotify:track:")));
        let next = if p.tracks.is_empty() { p.page_url() } else { p.next_page_url() };
        if out.len() >= max || next.is_empty() {
            continue;
        }
        let raw = session.spclient().get_next_page(next).await.map_err(lib_err)?;
        pages.push_back(protobuf_json_mapping::parse_from_str::<ContextPage>(&String::from_utf8_lossy(&raw)).map_err(lib_err)?);
    }
    out.truncate(max);
    Ok(out)
}

pub async fn liked() -> Result<Vec<Value>, ApiError> {
    let s = session().await?;
    let uris = context_uris(&s, &format!("spotify:user:{}:collection", s.username()), 5000).await?;
    Ok(tracks_for(&s, uris).await)
}

pub async fn search(q: &str) -> Result<Vec<Value>, ApiError> {
    let s = session().await?;
    let uris = context_uris(&s, &format!("spotify:search:{}", urlencoding::encode(q)), 50).await?;
    Ok(tracks_for(&s, uris).await)
}

/// Your playlists (folders flattened), Web API-shaped.
pub async fn playlists() -> Result<Vec<Value>, ApiError> {
    use protobuf::Message;
    let s = session().await?;
    let raw = s.spclient().get_rootlist(0, Some(2000)).await.map_err(lib_err)?;
    let list = librespot_protocol::playlist4_external::SelectedListContent::parse_from_bytes(&raw).map_err(lib_err)?;
    let c = list.contents.get_or_default();
    Ok(c.items
        .iter()
        .zip(c.meta_items.iter())
        .filter_map(|(item, meta)| {
            let id = item.uri().strip_prefix("spotify:playlist:")?;
            Some(serde_json::json!({ "id": id, "name": meta.attributes.name(), "items": { "total": meta.length() }, "owner": { "display_name": meta.owner_username() } }))
        })
        .collect())
}

pub async fn playlist(id: &str) -> Result<Vec<Value>, ApiError> {
    let s = session().await?;
    let uri = SpotifyUri::from_uri(&format!("spotify:playlist:{id}")).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
    let p = librespot_metadata::Playlist::get(&s, &uri).await.map_err(lib_err)?;
    let uris = p.tracks().filter_map(|u| u.to_uri().ok()).filter(|u| u.starts_with("spotify:track:")).collect();
    Ok(tracks_for(&s, uris).await)
}

// ---- playback as a WAV file ------------------------------------------

enum Msg {
    Pcm(Vec<u8>),
    End,
}

/// librespot's sink: converts each packet to 16-bit little-endian PCM and
/// hands it to the response. A full channel holds the decoder back, so it
/// decodes only as fast as the page reads.
struct ResponseSink {
    tx: mpsc::Sender<Msg>,
}

impl Sink for ResponseSink {
    fn write(&mut self, packet: AudioPacket, converter: &mut Converter) -> SinkResult<()> {
        let AudioPacket::Samples(samples) = packet else { return Ok(()) };
        let pcm = converter.f64_to_s16(&samples);
        let mut bytes = Vec::with_capacity(pcm.len() * 2);
        for s in pcm {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        // librespot calls this from inside its own runtime (block_on on the
        // player thread), where blocking_send panics; wait by polling instead.
        let mut msg = Msg::Pcm(bytes);
        loop {
            match self.tx.try_send(msg) {
                Ok(()) => return Ok(()),
                Err(mpsc::error::TrySendError::Full(m)) => {
                    msg = m;
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(mpsc::error::TrySendError::Closed(_)) => return Err(SinkError::OnWrite("the listener went away".into())),
            }
        }
    }
}

pub fn wav_header(data_len: u64) -> [u8; 44] {
    let mut h = [0u8; 44];
    let data_len = data_len.min(u32::MAX as u64 - 36) as u32;
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36 + data_len).to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(b"fmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    h[22..24].copy_from_slice(&2u16.to_le_bytes()); // stereo
    h[24..28].copy_from_slice(&(RATE as u32).to_le_bytes());
    h[28..32].copy_from_slice(&((RATE * BYTES_PER_FRAME) as u32).to_le_bytes());
    h[32..34].copy_from_slice(&(BYTES_PER_FRAME as u16).to_le_bytes());
    h[34..36].copy_from_slice(&16u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data_len.to_le_bytes());
    h
}

/// The track's length in bytes of PCM, from its metadata.
async fn data_len(session: &Session, uri: &SpotifyUri) -> Result<u64, ApiError> {
    let track = librespot_metadata::Track::get(session, uri)
        .await
        .map_err(|e| err(StatusCode::NOT_FOUND, format!("Spotify could not find this song: {e}")))?;
    let frames = (track.duration.max(0) as u64) * RATE / 1000;
    Ok(frames * BYTES_PER_FRAME)
}

/// Where a byte offset into the WAV lands: the millisecond to start the
/// player at, and how many PCM bytes to drop after it to land exactly.
pub fn seek_point(start: u64) -> (u32, u64) {
    let pcm_offset = start.saturating_sub(HEADER);
    let frame = pcm_offset / BYTES_PER_FRAME;
    let ms = frame * 1000 / RATE;
    let ms_frame = ms * RATE / 1000;
    (ms as u32, pcm_offset - ms_frame * BYTES_PER_FRAME)
}

pub async fn stream(id: String, headers: HeaderMap) -> Result<Response, ApiError> {
    if !id.chars().all(|c| c.is_ascii_alphanumeric()) || id.len() != 22 {
        return Err(err(StatusCode::BAD_REQUEST, "Not a Spotify track ID"));
    }
    let session = session().await?;
    let uri = SpotifyUri::from_uri(&format!("spotify:track:{id}")).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
    let pcm_len = data_len(&session, &uri).await?;
    let size = HEADER + pcm_len;
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let (start, end, partial) = match parse_range(range, size) {
        Ok(Some((s, e))) => (s, e, true),
        Ok(None) => (0, size - 1, false),
        Err(()) => {
            return Ok(Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{size}"))
                .body(Body::empty())?);
        }
    };
    let want = end - start + 1;

    // The header part of the range, if any.
    let mut prefix = Vec::new();
    if start < HEADER {
        let h = wav_header(pcm_len);
        prefix.extend_from_slice(&h[start as usize..(end + 1).min(HEADER) as usize]);
    }
    let needs_audio = end >= HEADER;

    let (tx, rx) = mpsc::channel::<Msg>(32);
    let player = if needs_audio {
        let (ms, drop) = seek_point(start.max(HEADER));
        let sink_tx = tx.clone();
        let config = PlayerConfig { bitrate: Bitrate::Bitrate320, gapless: false, ..PlayerConfig::default() };
        let player = Player::new(config, session.clone(), Box::new(NoOpVolume), move || Box::new(ResponseSink { tx: sink_tx }));
        let mut events = player.get_player_event_channel();
        let end_tx = tx.clone();
        tokio::spawn(async move {
            while let Some(e) = events.recv().await {
                if matches!(e, PlayerEvent::EndOfTrack { .. } | PlayerEvent::Unavailable { .. } | PlayerEvent::Stopped { .. }) {
                    let _ = end_tx.send(Msg::End).await;
                    break;
                }
            }
        });
        player.load(uri, true, ms);
        (Some(player), drop)
    } else {
        (None, 0)
    };
    drop(tx);

    let (player, skip) = player;
    let body = pcm_body(prefix, want, rx, player, skip);
    let mut res = Response::builder()
        .status(if partial { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK })
        .header(header::CONTENT_TYPE, "audio/wav")
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, want)
        .header(header::CACHE_CONTROL, "no-store");
    if partial {
        res = res.header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{size}"));
    }
    Ok(res.body(body)?)
}

/// Keeps the player alive while the response streams, and shuts it down
/// off the async threads when the response ends or the page stops reading.
struct PlayerGuard(Option<Arc<Player>>);

impl Drop for PlayerGuard {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            p.stop();
            std::thread::spawn(move || drop(p));
        }
    }
}

struct BodyState {
    // Field order matters: the receiver drops before the player, so a
    // decoder blocked on a full channel is released before the join.
    rx: mpsc::Receiver<Msg>,
    _player: PlayerGuard,
    prefix: Option<Vec<u8>>,
    skip: u64,
    left: u64,
    ended: bool,
}

fn pcm_body(prefix: Vec<u8>, want: u64, rx: mpsc::Receiver<Msg>, player: Option<Arc<Player>>, skip: u64) -> Body {
    let state = BodyState { rx, _player: PlayerGuard(player), prefix: Some(prefix), skip, left: want, ended: false };
    let stream = futures_util::stream::unfold(state, |mut st| async move {
        if st.left == 0 {
            return None;
        }
        if let Some(p) = st.prefix.take().filter(|p| !p.is_empty()) {
            let n = (p.len() as u64).min(st.left) as usize;
            st.left -= n as u64;
            return Some((Ok::<_, std::io::Error>(Bytes::from(p[..n].to_vec())), st));
        }
        loop {
            let chunk = if st.ended { None } else { st.rx.recv().await };
            match chunk {
                Some(Msg::Pcm(mut pcm)) => {
                    if st.skip > 0 {
                        let k = (st.skip as usize).min(pcm.len());
                        pcm.drain(..k);
                        st.skip -= k as u64;
                    }
                    if pcm.is_empty() {
                        continue;
                    }
                    let n = (pcm.len() as u64).min(st.left) as usize;
                    pcm.truncate(n);
                    st.left -= n as u64;
                    return Some((Ok(Bytes::from(pcm)), st));
                }
                // The song ended before its stated length: fill the rest
                // with silence so the response is as long as promised.
                Some(Msg::End) | None => {
                    st.ended = true;
                    let n = st.left.min(64 * 1024) as usize;
                    st.left -= n as u64;
                    return Some((Ok(Bytes::from(vec![0u8; n])), st));
                }
            }
        }
    });
    Body::from_stream(stream)
}

pub async fn token_route() -> Result<axum::Json<Value>, ApiError> {
    let (access_token, expires_at, _) = api_token().await?;
    Ok(axum::Json(serde_json::json!({ "accessToken": access_token, "expiresAt": expires_at })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_describes_cd_audio() {
        let h = wav_header(176_400);
        assert_eq!(&h[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(h[24..28].try_into().unwrap()), 44_100);
        assert_eq!(u32::from_le_bytes(h[40..44].try_into().unwrap()), 176_400);
    }

    #[test]
    fn seeks_land_on_the_requested_byte() {
        assert_eq!(seek_point(0), (0, 0));
        assert_eq!(seek_point(44), (0, 0));
        // One second in.
        assert_eq!(seek_point(44 + 176_400), (1000, 0));
        // A frame boundary between milliseconds: 1 ms is 44.1 frames.
        let (ms, drop) = seek_point(44 + 50 * 4);
        assert_eq!(ms, 1);
        assert_eq!(drop, (50 - 44) * 4);
        // A byte inside a frame keeps its offset.
        let (ms, drop) = seek_point(44 + 50 * 4 + 2);
        assert_eq!((ms, drop), (1, (50 - 44) * 4 + 2));
    }
}

/// Debug builds only: a 30-second tone through the same WAV path as
/// Spotify songs (header, byte ranges, seeks, padding), with a generator
/// standing in for librespot. Ends 2 s early to exercise the padding.
#[cfg(debug_assertions)]
pub async fn test_tone(headers: HeaderMap) -> Result<Response, ApiError> {
    let pcm_len = 30 * RATE * BYTES_PER_FRAME;
    let size = HEADER + pcm_len;
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let (start, end, partial) = match parse_range(range, size) {
        Ok(Some((s, e))) => (s, e, true),
        Ok(None) => (0, size - 1, false),
        Err(()) => return Err(err(StatusCode::RANGE_NOT_SATISFIABLE, "range")),
    };
    let mut prefix = Vec::new();
    if start < HEADER {
        prefix.extend_from_slice(&wav_header(pcm_len)[start as usize..(end + 1).min(HEADER) as usize]);
    }
    let (tx, rx) = mpsc::channel::<Msg>(32);
    let (ms, drop) = seek_point(start.max(HEADER));
    std::thread::spawn(move || {
        let mut frame = ms as u64 * RATE / 1000;
        let last = 28 * RATE;
        while frame < last {
            let mut chunk = Vec::with_capacity(4096 * 4);
            for _ in 0..4096.min(last - frame) {
                let v = ((frame as f64 * 440.0 * std::f64::consts::TAU / RATE as f64).sin() * 8000.0) as i16;
                chunk.extend_from_slice(&v.to_le_bytes());
                chunk.extend_from_slice(&v.to_le_bytes());
                frame += 1;
            }
            if tx.blocking_send(Msg::Pcm(chunk)).is_err() {
                return;
            }
        }
        let _ = tx.blocking_send(Msg::End);
    });
    let want = end - start + 1;
    let body = pcm_body(prefix, want, rx, None, drop);
    let mut res = Response::builder()
        .status(if partial { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK })
        .header(header::CONTENT_TYPE, "audio/wav")
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, want);
    if partial {
        res = res.header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{size}"));
    }
    Ok(res.body(body)?)
}
