//! Staying up to date: G-Amp asks GitHub for the latest release when it
//! starts (and when you choose Check for updates), says so in the display,
//! and installs it on request: the new app replaces this one and starts.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use crate::app::App;

const API: &str = "https://api.github.com/repos/fillinthe/g-amp/releases/latest";

#[derive(Clone, Debug)]
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
    reqwest::blocking::Client::builder().user_agent(concat!("G-Amp/", env!("CARGO_PKG_VERSION"))).connect_timeout(Duration::from_secs(15)).timeout(None).build().expect("http client")
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

impl App {
    /// Asks GitHub in the background; `said` reports "up to date" too.
    pub fn update_check(&mut self, said: bool) {
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
                    self.main.message = Some(format!("G-AMP {} IS OUT - RIGHT-CLICK › UPDATE", r.version));
                    self.message_until = Some(std::time::Instant::now() + Duration::from_secs(8));
                    self.update = Some(r);
                }
                Msg::Checked(Ok(None), said) => {
                    if said {
                        self.say(&format!("G-AMP {} IS UP TO DATE", env!("CARGO_PKG_VERSION")));
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
                Msg::Failed(e) => self.say(&format!("UPDATE FAILED: {e}")),
            }
            self.mark(crate::app::Kind::Main);
        }
    }

    /// Shows a message in the display for a few seconds.
    pub fn say(&mut self, text: &str) {
        self.main.message = Some(text.to_uppercase());
        self.message_until = Some(std::time::Instant::now() + Duration::from_secs(5));
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

    /// Hands over to a small script that swaps the app once G-Amp has
    /// quit, then starts the new one.
    fn update_apply(&mut self, file: &Path, r: &Release) {
        let pid = std::process::id();
        let result: Result<(), String> = if cfg!(target_os = "macos") {
            match this_bundle() {
                None => Err("run G-Amp from its .app to update it".into()),
                Some(app) => {
                    let mount = file.with_extension("mount");
                    let script = format!(
                        r#"while kill -0 {pid} 2>/dev/null; do sleep 0.3; done
hdiutil attach -nobrowse -quiet -mountpoint "{m}" "{f}" || exit 1
new=$(ls -d "{m}"/*.app | head -1)
rm -rf "{a}.old" && mv "{a}" "{a}.old" && cp -R "$new" "{a}" && rm -rf "{a}.old" || {{ mv "{a}.old" "{a}"; }}
hdiutil detach -quiet "{m}"
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
            // Wait for G-Amp to close, install silently, start it again.
            let script = format!(
                "$p = Get-Process -Id {pid} -ErrorAction SilentlyContinue; if ($p) {{ $p.WaitForExit() }}; Start-Process -Wait -FilePath '{f}' -ArgumentList '/S'; Start-Process -FilePath '{exe}'",
                f = file.display()
            );
            std::process::Command::new("powershell").args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script]).spawn().map(|_| ()).map_err(|e| e.to_string())
        } else if let Some(image) = std::env::var_os("APPIMAGE").filter(|_| file.extension().is_some_and(|e| e.eq_ignore_ascii_case("appimage"))) {
            let image = PathBuf::from(image);
            let script = format!(
                r#"while kill -0 {pid} 2>/dev/null; do sleep 0.3; done
chmod +x "{f}" && mv -f "{f}" "{i}" && "{i}" &"#,
                f = file.display(),
                i = image.display()
            );
            std::process::Command::new("sh").arg("-c").arg(script).spawn().map(|_| ()).map_err(|e| e.to_string())
        } else {
            // A .deb needs the system's installer (and your password).
            crate::backend::open_in_browser(&file.display().to_string());
            self.say(&format!("OPENED G-AMP {} IN YOUR SOFTWARE INSTALLER", r.version));
            return;
        };
        match result {
            Ok(()) => {
                self.save_session();
                self.quit = true;
            }
            Err(e) => self.say(&format!("UPDATE FAILED: {e}")),
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
        let names: Vec<String> = ["G-Amp-v0.3.0-macOS-apple-silicon.dmg", "G-Amp-v0.3.0-macOS-intel.dmg", "G-Amp-v0.3.0-Windows-setup.exe", "G-Amp-v0.3.0-Linux-amd64.deb", "G-Amp-v0.3.0-Linux-x86_64.AppImage"].map(String::from).to_vec();
        let i = pick_asset(&names).unwrap();
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            assert_eq!(names[i], "G-Amp-v0.3.0-macOS-apple-silicon.dmg");
        }
        if cfg!(windows) {
            assert_eq!(names[i], "G-Amp-v0.3.0-Windows-setup.exe");
        }
    }
}
