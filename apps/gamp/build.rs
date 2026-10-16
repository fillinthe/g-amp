// The program icon and details for G-Amp.exe on Windows.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/logo/gamp.ico");
        res.set("ProductName", "G-Amp");
        res.set("FileDescription", "G-Amp music player");
        res.set("CompanyName", "Fill in the Blank, LLC");
        if let Err(e) = res.compile() {
            println!("cargo:warning=no Windows icon: {e}");
        }
    }
}
