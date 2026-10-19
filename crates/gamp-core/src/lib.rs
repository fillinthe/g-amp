//! Gamp's core: every music source (folders and network drives, Plex,
//! Jellyfin, Subsonic, Synology, WebDAV, DLNA, Dropbox, radio, podcasts,
//! the Internet Archive, SoundCloud, Spotify), the folder library, the
//! settings store, and the local server the iPhone app's page runs on.
//! No UI here: the native desktop app and the iPhone app both build on it.

pub mod catalog;
pub mod http;
pub mod library;
pub mod server;
pub mod shell;
pub mod soundcloud;
pub mod soundcloud_web;
pub mod sources;
pub mod spotify;
pub mod store;

use std::path::PathBuf;

/// Fixed, because SoundCloud matches the sign-in callback address exactly.
pub const PORT: u16 = 8742;

pub fn port() -> u16 {
    std::env::var("WINAMP_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(PORT)
}

/// Opens the data folder, starts the local server (with `page` serving
/// everything that isn't an API route) and rescans the library.
pub async fn start_server(data: PathBuf, documents: Option<PathBuf>, page: axum::Router) -> anyhow::Result<u16> {
    store::init(data, documents);
    // Sign-ins Gamp no longer has (0.3.0's own-app Spotify library and
    // SoundCloud API keys): their refresh tokens shouldn't linger.
    for old in ["spotify-library-tokens.json", "soundcloud-tokens.json"] {
        let _ = std::fs::remove_file(store::data_dir().join(old));
    }
    let port = port();
    let listener = server::serve(port).await.map_err(|e| anyhow::anyhow!("Port {port} is already in use ({e}). Is Gamp already open?"))?;
    spotify::set_app_origin(format!("http://{}:{port}", server::HOST));
    // Pick up music added since last time. Unchanged files are skipped,
    // so this is cheap.
    tokio::spawn(async {
        if !store::config().await.folders.is_empty() {
            library::rescan().await;
        }
    });
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, server::router(port, page)).await {
            log::error!("server stopped: {e}");
        }
    });
    Ok(port)
}

/// The app used to be called Winamp Hub; bring its settings, sign-ins and
/// library along the first time Gamp starts.
pub fn adopt_old_data(data: &std::path::Path) {
    let Some(old) = data.parent().map(|p| p.join("rocks.winamphub.player")) else { return };
    if data.exists() || !old.is_dir() {
        return;
    }
    fn copy(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)? {
            let e = e?;
            let target = to.join(e.file_name());
            if e.file_type()?.is_dir() { copy(&e.path(), &target)? } else { std::fs::copy(e.path(), target).map(|_| ())? }
        }
        Ok(())
    }
    if let Err(e) = copy(&old, data) {
        log::warn!("couldn't bring over the old settings: {e}");
    }
}
