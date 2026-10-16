//! `cargo run -p gamp-core --example probe -- <source> <node|?query>`:
//! lists a source's node (or searches it) using the app's real data
//! folder, to check sign-ins without opening the app.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let data = std::env::var_os("WINAMP_DATA").map(std::path::PathBuf::from).expect("set WINAMP_DATA");
    gamp_core::store::init(data, None);
    let mut args = std::env::args().skip(1);
    let sid = args.next().unwrap_or_else(|| "spotify".into());
    let what = args.next().unwrap_or_default();
    if what == "token" {
        let (t, _) = gamp_core::spotify::access_token().await.map_err(|e| anyhow::anyhow!(e.1))?;
        let r = reqwest::Client::new().get("https://api.spotify.com/v1/me").bearer_auth(t).send().await?;
        println!("{} retry-after={:?}", r.status(), r.headers().get("retry-after"));
        return Ok(());
    }
    if what.is_empty() {
        for s in gamp_core::catalog::sources().await {
            println!("{} search={} setup={} roots={:?}", s.id, s.search, s.needs_setup, s.roots);
        }
        return Ok(());
    }
    let res = match what.strip_prefix('?') {
        Some(q) => gamp_core::catalog::search(&sid, q).await,
        None => gamp_core::catalog::browse(&sid, &what).await,
    };
    match res {
        Ok(l) => {
            println!("{} items", l.items.len());
            for i in l.items.iter().take(8) {
                println!("  {} | {} | {}", i.title, i.artist, i.id);
            }
        }
        Err(e) => println!("error {}: {}", e.0, e.1),
    }
    Ok(())
}
