//! Staying up to date: Gamp asks GitHub for the latest release when it
//! starts, every few hours while open, and when you choose Check for
//! updates; it says so in the display and installs it on request: the new
//! app replaces this one and starts. When that doesn't work (a failed
//! download, a read-only install, an installer that gave up), Gamp opens
//! the release's download page so you can install it by hand.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use crate::app::App;

const API: &str = "https://api.github.com/repos/fillinthe/g-amp/releases/latest";
const RELEASES: &str = "https://github.com/fillinthe/g-amp/releases/latest";
/// How often a running Gamp asks again.
pub const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
/// Left in the data folder while an update installs; the next start reads it.
const ATTEMPT: &str = "update-attempt.json";

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Release {
    pub version: String,
    pub page: String,
    /// The file for this computer, if the release has one.
    pub asset: Option<(String, String)>,
}

pub enum Msg {
    Checked(Result<Option<Release>, String>, bool),
    Progress(u8),
    Ready(PathBuf, Release),
    Failed(String),
}

/// "0.10.2" > "0.9.9", tolerating a leading "v".
pub fn newer(theirs: &str, ours: &str) -> bool {
    let parse = |v: &str| v.trim_start_matches('v').split(['.', '-']).take(3).map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    parse(theirs) > parse(ours)
}

/// Which release file suits this computer.
pub fn pick_asset(names: &[String]) -> Option<usize> {
    let want: &[&str] = if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") { &["apple-silicon.dmg", "aarch64.dmg"] } else { &["intel.dmg", "x64.dmg"] }
    } else if cfg!(windows) {
        &["setup.exe", ".msi"]
    } else if std::env::var_os("APPIMAGE").is_some() {
        &[".appimage"]
    } else {
        &[".deb", ".appimage"]
    };
    want.iter().find_map(|w| names.iter().position(|n| n.to_lowercase().ends_with(w)))
}

fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder().user_agent(concat!("Gamp/", env!("CARGO_PKG_VERSION"))).connect_timeout(Duration::from_secs(15)).timeout(None).build().expect("http client")
}

fn check() -> Result<Option<Release>, String> {
    let r: serde_json::Value = client().get(API).timeout(Duration::from_secs(20)).send().map_err(|e| e.to_string())?.json().map_err(|e| e.to_string())?;
    let tag = r["tag_name"].as_str().unwrap_or("").to_owned();
    if tag.is_empty() {
        return Err("no releases yet".into());
    }
    if !newer(&tag, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let assets: Vec<(String, String)> = r["assets"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| Some((x["name"].as_str()?.to_owned(), x["browser_download_url"].as_str()?.to_owned()))).collect())
        .unwrap_or_default();
    let names: Vec<String> = assets.iter().map(|a| a.0.clone()).collect();
    let asset = pick_asset(&names).map(|i| assets[i].clone());
    Ok(Some(Release { version: tag.trim_start_matches('v').to_owned(), page: r["html_url"].as_str().unwrap_or("").to_owned(), asset }))
}

fn download(url: &str, to: &Path, tx: &mpsc::Sender<Msg>) -> Result<(), String> {
    use std::io::{Read, Write};
    let mut res = client().get(url).send().map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("download answered {}", res.status()));
    }
    let total = res.content_length().unwrap_or(0);
    let mut file = std::fs::File::create(to).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 256 * 1024];
    let (mut got, mut last) = (0u64, 0u8);
    loop {
        let n = res.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        got += n as u64;
        if total > 0 {
            let pct = (got * 100 / total) as u8;
            if pct != last {
                last = pct;
                let _ = tx.send(Msg::Progress(pct));
            }
        }
    }
    Ok(())
}

/// The .app this program runs from (macOS), or None when run loose.
fn this_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let app = exe.parent()?.parent()?.parent()?;
    (app.extension()? == "app").then(|| app.to_path_buf())
}

/// What to tell the person after a start: the update `attempt` that was
/// left behind didn't take if this is still an older version.
pub fn failed_attempt(attempt: &Release, ours: &str) -> bool {
    newer(&attempt.version, ours)
}

/// Whether files can be created next to `path` (where the new app goes).
fn can_write_beside(path: &Path) -> bool {
    let Some(dir) = path.parent() else { return false };
    let probe = dir.join(format!(".gamp-update-{}", std::process::id()));
    let ok = std::fs::File::create(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

impl App {
    /// Asks GitHub in the background; `said` reports "up to date" too.
    pub fn update_check(&mut self, said: bool) {
        self.update_checked = std::time::Instant::now();
        let (tx, rx) = mpsc::channel();
        self.update_rx = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Checked(check(), said));
        });
    }

    pub fn update_poll(&mut self) {
        let Some(rx) = &self.update_rx else { return };
        let msgs: Vec<Msg> = rx.try_iter().collect();
        for m in msgs {
            match m {
                Msg::Checked(Ok(Some(r)), _) => {
                    self.main.message = Some(format!("GAMP {} IS OUT - RIGHT-CLICK › UPDATE", r.version));
                    self.message_until = Some(std::time::Instant::now() + Duration::from_secs(8));
                    self.update = Some(r);
                }
                Msg::Checked(Ok(None), said) => {
                    if said {
                        self.say(&format!("GAMP {} IS UP TO DATE", env!("CARGO_PKG_VERSION")));
                    }
                }
                Msg::Checked(Err(e), said) => {
                    if said {
                        self.say(&format!("CAN'T CHECK FOR UPDATES: {e}"));
                    }
                }
                Msg::Progress(p) => {
                    self.main.message = Some(format!("DOWNLOADING UPDATE {p}%"));
                    self.message_until = Some(std::time::Instant::now() + Duration::from_secs(3));
                }
                Msg::Ready(file, r) => self.update_apply(&file, &r),
                Msg::Failed(e) => self.update_by_hand(&format!("UPDATE FAILED ({e})")),
            }
            self.mark(crate::app::Kind::Main);
        }
    }

    /// The automatic update didn't work: open the download page instead.
    fn update_by_hand(&mut self, why: &str) {
        let page = self.update.as_ref().map(|r| r.page.clone()).filter(|p| !p.is_empty()).unwrap_or_else(|| RELEASES.into());
        crate::backend::open_in_browser(&page);
        self.say(&format!("{why} - OPENED THE DOWNLOAD PAGE"));
    }

    /// At start: did the update we quit for install? If not, say so, open
    /// its download page and return true (no need to ask GitHub again).
    pub fn update_after_restart(&mut self) -> bool {
        let path = self.data_dir.join(ATTEMPT);
        let Some(attempt) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Release>(&b).ok()) else { return false };
        let _ = std::fs::remove_file(&path);
        if !failed_attempt(&attempt, env!("CARGO_PKG_VERSION")) {
            return false;
        }
        self.update = Some(attempt.clone());
        self.update_by_hand(&format!("GAMP {} DIDN'T INSTALL", attempt.version));
        true
    }

    /// Asks again every few hours while Gamp stays open.
    pub fn update_tick(&mut self) {
        if self.update.is_none() && self.update_checked.elapsed() >= CHECK_EVERY && std::env::var_os("GAMP_NO_UPDATE_CHECK").is_none() {
            self.update_check(false);
        }
    }

    /// Shows a message in the display for a few seconds.
    pub fn say(&mut self, text: &str) {
        self.main.message = Some(text.to_uppercase());
        // Long enough to scroll past once (a letter every 220 ms) when it doesn't fit.
        let secs = (text.chars().count() as u64 * 22 / 100 + 3).max(5);
        self.message_until = Some(std::time::Instant::now() + Duration::from_secs(secs));
        self.mark(crate::app::Kind::Main);
    }

    /// Downloads the new version, then installs it.
    pub fn update_install(&mut self) {
        let Some(r) = self.update.clone() else { return self.update_check(true) };
        let Some((name, url)) = r.asset.clone() else {
            crate::backend::open_in_browser(&r.page);
            return self.say("OPENED THE DOWNLOAD PAGE");
        };
        let (tx, rx) = mpsc::channel();
        self.update_rx = Some(rx);
        self.say("DOWNLOADING UPDATE…");
        std::thread::spawn(move || {
            let dir = std::env::temp_dir().join(format!("gamp-update-{}", std::process::id()));
            let _ = std::fs::create_dir_all(&dir);
            let file = dir.join(&name);
            match download(&url, &file, &tx) {
                Ok(()) => {
                    let _ = tx.send(Msg::Ready(file, r));
                }
                Err(e) => {
                    let _ = tx.send(Msg::Failed(e));
                }
            }
        });
    }

    /// Hands over to a small script that swaps the app once Gamp has
    /// quit, then starts the new one.
    fn update_apply(&mut self, file: &Path, r: &Release) {
        let pid = std::process::id();
        let result: Result<(), String> = if cfg!(target_os = "macos") {
            match this_bundle() {
                None => Err("run Gamp from its .app to update it".into()),
                // Run from the disk image, or from where macOS keeps apps that
                // weren't moved (App Translocation): nothing can be replaced there.
                Some(app) if !can_write_beside(&app) => Err("Gamp can't replace itself where it is".into()),
                Some(app) => {
                    let mount = file.with_extension("mount");
                    // Whatever happens, Gamp opens again: the new one, or the old one,
                    // which then sees the update didn't take and opens the download page.
                    let script = format!(
                        r#"while kill -0 {pid} 2>/dev/null; do sleep 0.3; done
if hdiutil attach -nobrowse -quiet -mountpoint "{m}" "{f}"; then
  new=$(ls -d "{m}"/*.app | head -1)
  rm -rf "{a}.old"
  # Only once the old app is safely aside is anything at its place touched.
  if mv "{a}" "{a}.old"; then
    if cp -R "$new" "{a}"; then rm -rf "{a}.old"; else rm -rf "{a}"; mv "{a}.old" "{a}"; fi
  fi
  hdiutil detach -quiet "{m}"
fi
open "{a}""#,
                        m = mount.display(),
                        f = file.display(),
                        a = app.display()
                    );
                    std::process::Command::new("sh").arg("-c").arg(script).spawn().map(|_| ()).map_err(|e| e.to_string())
                }
            }
        } else if cfg!(windows) {
            let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
            // Wait for Gamp to close, install silently, start it again.
            let script = format!(
                "$p = Get-Process -Id {pid} -ErrorAction SilentlyContinue; if ($p) {{ $p.WaitForExit() }}; Start-Process -Wait -FilePath '{f}' -ArgumentList '/S'; Start-Process -FilePath '{exe}'",
                f = file.display()
            );
            std::process::Command::new("powershell").args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script]).spawn().map(|_| ()).map_err(|e| e.to_string())
        } else if let Some(image) = std::env::var_os("APPIMAGE").filter(|_| file.extension().is_some_and(|e| e.eq_ignore_ascii_case("appimage"))) {
            let image = PathBuf::from(image);
            let script = format!(
                r#"while kill -0 {pid} 2>/dev/null; do sleep 0.3; done
chmod +x "{f}" && mv -f "{f}" "{i}"; "{i}" &"#,
                f = file.display(),
                i = image.display()
            );
            std::process::Command::new("sh").arg("-c").arg(script).spawn().map(|_| ()).map_err(|e| e.to_string())
        } else {
            // A .deb needs the system's installer (and your password).
            crate::backend::open_in_browser(&file.display().to_string());
            self.say(&format!("OPENED GAMP {} IN YOUR SOFTWARE INSTALLER", r.version));
            return;
        };
        match result {
            Ok(()) => {
                // The next start checks this to see whether the update took.
                if let Ok(json) = serde_json::to_vec(r) {
                    let _ = std::fs::write(self.data_dir.join(ATTEMPT), json);
                }
                self.save_session();
                self.quit = true;
            }
            Err(e) => self.update_by_hand(&format!("UPDATE FAILED ({e})")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_and_files() {
        assert!(newer("v0.3.0", "0.2.0"));
        assert!(newer("0.10.0", "0.9.9"));
        assert!(!newer("v0.2.0", "0.2.0"));
        assert!(!newer("0.1.9", "0.2.0"));
        let names: Vec<String> = ["Gamp-v0.3.0-macOS-apple-silicon.dmg", "Gamp-v0.3.0-macOS-intel.dmg", "Gamp-v0.3.0-Windows-setup.exe", "Gamp-v0.3.0-Linux-amd64.deb", "Gamp-v0.3.0-Linux-x86_64.AppImage"].map(String::from).to_vec();
        let i = pick_asset(&names).unwrap();
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            assert_eq!(names[i], "Gamp-v0.3.0-macOS-apple-silicon.dmg");
        }
        if cfg!(windows) {
            assert_eq!(names[i], "Gamp-v0.3.0-Windows-setup.exe");
        }
    }

    #[test]
    fn a_start_after_an_update_knows_whether_it_took() {
        let r = Release { version: "0.4.0".into(), page: String::new(), asset: None };
        assert!(failed_attempt(&r, "0.3.0"), "still the old version: it didn't take");
        assert!(!failed_attempt(&r, "0.4.0"));
        assert!(!failed_attempt(&r, "0.4.1"));
    }

    #[test]
    fn knows_where_it_can_replace_itself() {
        let dir = std::env::temp_dir().join(format!("gamp-update-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(can_write_beside(&dir.join("Gamp.app")));
        assert!(!can_write_beside(Path::new("/System/Gamp.app")));
        let _ = std::fs::remove_dir(&dir);
    }
}
