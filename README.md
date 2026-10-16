<p align="center"><img src="assets/logo/gamp-icon.png" width="128" alt="G-Amp"></p>

# G-Amp

The classic Winamp 2 player, via [Webamp](https://webamp.org), as a small native
app for macOS, Windows and iOS (Linux builds too, untested). It plays everything you have: music
folders and network drives, your NAS and media servers, Dropbox, internet
radio, podcasts, the Internet Archive, SoundCloud and Spotify. Winamp's
equalizer, skins and MilkDrop visualizer work on every source.

It runs in its own borderless window, exactly the size of the Winamp windows,
like the original. The Media Library opens beside the player and is drawn
from the current skin.

## Sources

| Source | What you need |
|---|---|
| **Folders and network drives** | Any folder this computer can open: a mounted SMB/AFP/NFS share (macOS `/Volumes`, Windows `\\server\share` or a drive letter, Linux `/mnt`). Settings can connect to `smb://` shares for you. |
| **Plex** | Sign in with Plex. Every server on your account is added, with its home and away addresses. |
| **Jellyfin / Emby** | Server address, user name and password. |
| **Navidrome, Airsonic, Gonic, Ampache, LMS** (any Subsonic API server) | Server address, user name and password. |
| **Synology Audio Station** | Your DSM address (`http://nas:5000`, or `https://you.synology.me:5001` from outside), user, password, and two-step code if you use one. |
| **WebDAV** (Synology WebDAV Server, Nextcloud, ownCloud, pCloud, QNAP...) | Folder address, user name and password. |
| **DLNA / UPnP** (Synology Media Server, minidlna, routers, Windows Media sharing) | Nothing. Servers on your network appear by themselves (not on iOS). |
| **Dropbox** | Your own Dropbox app key (Dropbox issues one per app; the steps are in Settings). |
| **Internet radio** | Nothing. About 50,000 stations from the [radio-browser.info](https://www.radio-browser.info) directory. Paste any stream, `.pls` or `.m3u` address to play it. |
| **Podcasts** | Nothing. Search Apple's directory or paste any RSS feed, then subscribe. |
| **Internet Archive** | Nothing. The Live Music Archive, 78 RPM records, netlabels, old-time radio, and search. |
| **Spotify** | A Premium account. Click Connect and sign in. |
| **SoundCloud** | API keys, which SoundCloud gives to Artist Pro accounts. Register an app with the Redirect URI shown in Settings. |

Every server address works at home or over the internet, so a NAS can be
played from anywhere it can be reached. Self-signed certificates (common on
NAS boxes) can be accepted per server.

Not supported: Mixcloud (its API doesn't allow playback in other apps),
YouTube, Apple Music, Tidal and Deezer (no public playback APIs), Google
Drive and OneDrive (yet; WebDAV via `rclone serve webdav` works meanwhile).

### About Spotify

Spotify playback uses [librespot](https://github.com/librespot-org/librespot),
the open-source engine also used by spotifyd, ncspot and Spotifast. Spotify
does not officially support third-party players, so use it at your own risk.

## Download

Builds for macOS (Apple silicon and Intel) and Windows are on the
[Releases](https://github.com/fillinthe/g-amp/releases) page. They are
unsigned: on a Mac, right-click › Open the first time; on Windows, More info ›
Run anyway. iPhone builds need your own Apple developer account (see Build).

## Using it

- Drag any Winamp title bar to move the app. Winamp's close button quits.
- **Window** menu (or the tray icon): Media Library ⌥L, Equalizer ⌥G,
  Playlist ⌥E, MilkDrop ⌥K, Always on Top.
- In the library, double-click a folder, album or playlist to open it, and
  Backspace (or ‹ Back) to return. Double-click a track to play from there.
- Drop any `.wsz` skin onto the player. Thousands are at
  [skins.webamp.org](https://skins.webamp.org).
- MilkDrop has about 400 presets: Space for the next, H for a hard cut,
  double-click for full screen.

## How it works

`src-tauri/` is the Rust app ([Tauri 2](https://tauri.app)). A server on
`127.0.0.1:8742` serves the Webamp page (`web/`) and all of the audio, so
every source reaches the player as one plain, seekable audio file and the
equalizer and visualizers see the real signal:

- local files are served with byte ranges;
- media servers, WebDAV, Dropbox, DLNA, podcasts and the Archive are proxied
  with byte ranges (`src-tauri/src/sources/`);
- radio streams are passed through, including old SHOUTcast servers, and
  the station's song titles show in Winamp's scrolling display;
- SoundCloud's HLS is stitched into one seekable MP3;
- Spotify is decoded by librespot into a seekable WAV stream.

The server only listens on 127.0.0.1, refuses requests from other web
origins, and only fetches track addresses it signed itself. Passwords and
tokens stay in the app's data folder, readable by your user alone.

`g-amp --server <data-dir>` runs the server without a window (for a NAS
or a headless box), then open `http://127.0.0.1:8742` in a browser.

## Build

Needs Rust 1.90+ and Node 20+.

The repository is a Cargo workspace:

- `crates/gamp-core`: every music source, the library, Spotify, SoundCloud, the settings store and the local server.
- `apps/gamp-ios`: the current app (Tauri with the Webamp page), used for the 0.1 releases and the iPhone build.
- A fully native Rust player (its own skin engine, audio engine and visualizers) is being built in `crates/` and `apps/gamp`. See the roadmap in the issues.

```sh
cd apps/gamp-ios
npm install
npm run app          # run it in development
npm run release      # macOS .app (in target/release/bundle at the repo root)
npm run windows      # Windows installer from macOS (needs cargo-xwin, llvm, lld, makensis)
cd ../.. && cargo test --workspace --lib
python3 scripts/smoke.py target/debug/gamp-ios
```

For iPhone (needs Xcode and CocoaPods), set your Apple team and generate the
Xcode project once, from `apps/gamp-ios`:

```sh
export APPLE_DEVELOPMENT_TEAM=YOURTEAMID
npx tauri ios init
npm run ios
```

## Credits

Built on [Webamp](https://github.com/captbaritone/webamp) and
[Butterchurn](https://github.com/jberg/butterchurn) (MIT), with the MilkDrop
presets from butterchurn-presets, [librespot](https://github.com/librespot-org/librespot)
(MIT) and Tauri (MIT/Apache-2.0).

Winamp is a trademark of its owner. This project is not affiliated with or
endorsed by Winamp, Spotify, SoundCloud, Plex, Synology, Dropbox or any
other service it connects to.

## License

MIT. See [LICENSE](LICENSE).
