<p align="center"><img src="assets/logo/gamp-icon.png" width="128" alt="G-Amp"></p>

# G-Amp

The classic Winamp 2 player, rebuilt from scratch in Rust as a native app for
macOS and Windows (Linux builds too). It plays everything you have: music
folders and network drives, your NAS and media servers, Dropbox, internet
radio, podcasts, the Internet Archive, SoundCloud and Spotify. The
equalizer, skins and the G-Drop visualizer work on every source.

Every window is its own small borderless window drawn pixel for pixel from
the skin, like the original: player, equalizer, playlist, Media Library and
G-Drop. Classic `.wsz` skins load as they are. Idle, it uses next to no CPU.

## Sources

| Source | What you need |
|---|---|
| **Folders and network drives** | Any folder this computer can open: a mounted SMB/AFP/NFS share (macOS `/Volumes`, Windows `\\server\share` or a drive letter, Linux `/mnt`). Settings can connect to `smb://` shares for you. |
| **Plex** | Sign in with Plex. Every server on your account is added, with its home and away addresses. |
| **Jellyfin / Emby** | Server address, user name and password. |
| **Navidrome, Airsonic, Gonic, Ampache, LMS** (any Subsonic API server) | Server address, user name and password. |
| **Synology Audio Station** | Your DSM address (`http://nas:5000`, or `https://you.synology.me:5001` from outside), user, password, and two-step code if you use one. |
| **WebDAV** (Synology WebDAV Server, Nextcloud, ownCloud, pCloud, QNAP...) | Folder address, user name and password. |
| **DLNA / UPnP** (Synology Media Server, minidlna, routers, Windows Media sharing) | Nothing. Servers on your network appear by themselves. |
| **Dropbox** | Your own Dropbox app key (Dropbox issues one per app; the steps are in Settings). |
| **Internet radio** | Nothing. About 50,000 stations from the [radio-browser.info](https://www.radio-browser.info) directory. Paste any stream, `.pls` or `.m3u` address to play it. |
| **Podcasts** | Nothing. Search Apple's directory or paste any RSS feed, then subscribe. |
| **Internet Archive** | Nothing. The Live Music Archive, 78 RPM records, netlabels, old-time radio, and search. |
| **Spotify** | A Premium account. Click Connect and sign in. For Liked Songs, playlists and search, also add your own free Spotify app's Client ID (steps in Settings). |
| **SoundCloud** | Nothing to paste a link: any track, playlist or artist link plays. For your likes, playlists and search, API keys (SoundCloud gives them to Artist Pro accounts). |

Every server address works at home or over the internet, so a NAS can be
played from anywhere it can be reached. Self-signed certificates (common on
NAS boxes) can be accepted per server.

Not supported: Mixcloud (its API doesn't allow playback in other apps),
YouTube, Apple Music, Tidal and Deezer (no public playback APIs), Google
Drive and OneDrive (yet; WebDAV via `rclone serve webdav` works meanwhile).

## Download

Builds for macOS (Apple silicon and Intel), Windows and Linux are on the
[Releases](https://github.com/fillinthe/g-amp/releases) page. They are
unsigned: on a Mac, right-click › Open the first time; on Windows, More info ›
Run anyway. G-Amp checks for new versions and updates itself (right-click ›
Check for updates…).

## Using it

- Drag a title bar to move a window; windows dock and snap together.
- Z X C V B: previous, play, pause, stop, next. L opens files, S shuffle,
  R repeat. Ctrl+D double size, Ctrl+T remaining time, Ctrl+A always on top.
- Alt (⌥) + L Media Library, G Equalizer, E Playlist, K G-Drop.
- In the library, double-click a folder, album or playlist to open it, and
  Backspace (or ‹ Back) to return. Double-click a track to play from there.
  Paste a SoundCloud link or a radio stream address into Search to play it.
- The playlist's LIST button saves and opens `.m3u8` playlists (`.m3u` and
  `.pls` open too). Saved lists keep radio stations and library tracks, not
  only files, and a station address from any other program plays as well.
- Drop any `.wsz` skin onto the player. Thousands are at
  [skins.webamp.org](https://skins.webamp.org).
- G-Drop has 25 presets: H for a random one, F for full screen.

## How it works

Everything is our own Rust: the skin engine, the pixel renderer, the audio
engine (decode, resample, 10-band EQ, gapless) and the visualizers. Inside,
a small server on `127.0.0.1:8742` turns every source into one plain,
seekable audio stream:

- media servers, WebDAV, Dropbox, DLNA, podcasts and the Archive are proxied
  with byte ranges;
- radio streams are passed through, including old SHOUTcast servers, and
  the station's song titles show in the scrolling display;
- SoundCloud's HLS (MP3 or AAC) is stitched into one seekable file;
- Spotify is decoded by librespot into a seekable stream.

The server only listens on 127.0.0.1, refuses requests from other web
origins, and only fetches track addresses it signed itself. Passwords and
tokens stay in the app's data folder, readable by your user alone.

## Build

Needs Rust 1.90+. On Linux, the ALSA, X11/Wayland and xkbcommon headers.

- `crates/gamp-core`: every music source, the library, Spotify, SoundCloud, the settings store and the local server.
- `crates/gamp-audio`: the audio engine.
- `crates/gamp-skin`, `crates/gamp-ui`: Winamp skins and the windows.
- `crates/gamp-vis`: G-Drop and the spectrum.
- `apps/gamp`: the app.
- `tools/skin-gen`: draws the default skin, GAMP Classic.

```sh
cargo run --release -p gamp     # run it
cargo test --workspace          # tests
target/debug/gamp --selftest    # the app, headless: plays, scans, seeks, draws
cargo packager --release -p gamp  # dmg / setup.exe / deb / AppImage (cargo install cargo-packager)
```

## Credits

The skin sprite map comes from [Webamp](https://github.com/captbaritone/webamp)
(MIT). Spotify playback uses [librespot](https://github.com/librespot-org/librespot)
(MIT), the open-source engine also used by spotifyd and ncspot. Spotify
does not officially support third-party players, so use it at your own risk.

Winamp is a trademark of its owner. This project is not affiliated with or
endorsed by Winamp, Spotify, SoundCloud, Plex, Synology, Dropbox or any
other service it connects to.

## License

MIT. See [LICENSE](LICENSE).
