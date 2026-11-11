//! Gives the Windows program its icon. Elsewhere it does nothing.
fn main() {
    println!("cargo:rerun-if-changed=assets/app_icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut r = winresource::WindowsResource::new();
        r.set_icon("assets/app_icon.ico");
        r.set("ProductName", "TellusHive");
        r.set("FileDescription", "TellusHive: an AI design team for civil engineers");
        if let Err(e) = r.compile() {
            println!("cargo:warning=no icon in the Windows program: {e}");
        }
    }
}
