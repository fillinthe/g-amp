<p align="center"><img src="assets/logo/gamp-icon.png" width="128" alt="Gamp"></p>

# Gamp

The classic Winamp 2 player, rebuilt from scratch in Rust as a native app for
macOS and Windows (Linux builds too). It plays everything you have: music
folders and network drives, your NAS and media servers, Dropbox, Google Drive and OneDrive, internet
radio, podcasts, the Internet Archive, SoundCloud and Spotify. The
equalizer, skins and the G-Drop visualizer work on every source.

<p align="center"><img src="assets/screenshot.png" width="825" alt="Gamp's player, equalizer and playlist, with the Media Library showing a SoundCloud search sorted by plays"></p>

<p align="center"><img src="assets/screenshot-skins.png" width="825" alt="Gamp wearing the Vaporwave skin from the Winamp Skin Museum, with the Media Library listing the museum's most popular skins"></p>
<p align="center"><em>Wearing Vaporwave, one of about 100,000 classic skins in the built-in Skin Museum.</em></p>

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
| **Synology Audio Station** | Your QuickConnect ID (or the DSM address, `http://nas:5000`), user, password, and two-step code if you use one. With QuickConnect, Gamp finds the NAS at home and away by itself. |
| **WebDAV** (Synology WebDAV Server, Nextcloud, ownCloud, pCloud, QNAP...) | Folder address, user name and password. |
| **DLNA / UPnP** (Synology Media Server, minidlna, routers, Windows Media sharing) | Nothing. Servers on your network appear by themselves. |
| **Dropbox, Google Drive, OneDrive** | Their desktop app, signed in. Gamp finds the folder by itself and plays from it; songs download as you play them. |
| **Internet radio** | Nothing. About 50,000 stations from the [radio-browser.info](https://www.radio-browser.info) directory. Paste any stream, `.pls` or `.m3u` address to play it. |
| **Podcasts** | Nothing. Search Apple's directory or paste any RSS feed, then subscribe. |
| **Internet Archive** | Nothing. The Live Music Archive, 78 RPM records, netlabels, old-time radio, and search. |
| **Spotify** | A Premium account. Click Connect and sign in: Liked Songs, playlists and search show up. |
| **SoundCloud** | Nothing. Search, or paste any track, playlist or artist link. For your likes and playlists, add your profile's address (they need to be public). Tracks show when they were posted, plays and likes; click a header to sort, and right-click a track for **Latest from** its artist. |

Every server address works at home or over the internet, so a NAS can be
played from anywhere it can be reached. Self-signed certificates (common on
NAS boxes) can be accepted per server.

Not supported: Mixcloud (its API doesn't allow playback in other apps),
YouTube, Apple Music, Tidal and Deezer (no public playback APIs), Google
Drive and OneDrive without their desktop apps.

## Download

Builds for macOS (Apple silicon and Intel), Windows and Linux are on the
[Releases](https://github.com/fillinthe/g-amp/releases) page. They are
unsigned: on a Mac, right-click › Open the first time; on Windows, More info ›
Run anyway. Gamp checks for new versions when it opens and every few hours,
asks whether to update (with what's new), then updates itself and opens again.
Right-click › Check for updates… asks now; `gamp --update` updates from a terminal.

## Using it

- Drag a title bar to move a window; windows dock and snap together.
  Clicking any window brings all of them in front of other programs.
  Lost one? Right-click › Gather windows (Alt+W) stacks them all back on
  the main window; on a Mac, clicking Gamp in the Dock does it when a
  window is off-screen.
- Z X C V B: previous, play, pause, stop, next. Shift+V fades out and
  stops; Ctrl+V stops after the current track. Right-click › Options ›
  Sleep timer stops the music after 15 to 90 minutes, fading out. L opens files, S shuffle,
  R repeat. J jumps to a file: type part of a title, Enter plays it.
  Ctrl+D double size. ⌘+ / ⌘− / ⌘0 (Ctrl on Windows and Linux) make every
  window bigger or smaller in 10% steps, up to 300%, or back to normal, as
  in a browser; right-click › Options › Size lists the sizes. Ctrl+T remaining time, Ctrl+A always on top.
- Alt (⌥) + L Media Library, G Equalizer, E Playlist, K G-Drop.
- In the library, double-click a folder, album or playlist to open it, and
  Backspace (or ‹ Back) to return. Double-click a track to play from there.
  Paste a SoundCloud link or a radio stream address into Search to play it.
  Right-click a track (or Shift+Enter) to add it to the playlist without
  interrupting what's playing.
- In the playlist, MISC › Sort list sorts by title, file name or path,
  reverses or randomizes the list; the playing track keeps playing.
  MISC › File info (Alt+3) shows a track's tags, format, size and length;
  for files the tags can be edited (Tab between fields) and saved.
  MISC › Misc opts generates an HTML playlist (opens in the browser) or
  re-reads the tags of the selected files.
- The playlist is kept when Gamp quits. The playlist's LIST button saves it
  by name (Save list) and Load list shows your saved playlists, or opens any
  `.m3u8`, `.m3u` or `.pls` file. Saved lists keep radio stations and library tracks, not
  only files, and a station address from any other program plays as well.
- The equalizer's PRESETS button has Winamp's presets, saves your own
  (Save…) and opens Winamp `.eqf` presets and `.q1` libraries (From file…,
  or drop one on Gamp).
- Songs, playlists, skins and presets open from the Finder too (double-click
  or Open With › Gamp).
- Skins › Skin Museum… (Alt+S) browses and searches about 100,000
  classic skins from the [Winamp Skin Museum](https://skins.webamp.org):
  double-click one to wear it. Skins › Surprise me puts on a random
  favourite, Keep this skin adds one you like to the Skins menu, and
  Skins › Original skin always goes back to GAMP Classic. Any `.wsz` you
  drop onto the player works too.
- G-Drop has 40 presets. Right-click it for the list, next/previous/random,
  full screen, and Cycle presets (on: it changes by itself; off: it stays on
  one; R toggles). The □ button or F goes full screen, Esc leaves. Drag the
  picture to move it, any edge to resize.
- Songs play at an even volume: files with ReplayGain tags (MP3, FLAC, M4A,
  Ogg...) are leveled by them, and Spotify uses its own leveling.

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
