// Embeds the app icon into lumen.exe on Windows. A failure here is not fatal.
fn main() {
    println!("cargo:rerun-if-changed=assets/lumen.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/lumen.ico");
        res.set("ProductName", "Lumen");
        res.set("FileDescription", "Lumen browser");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed icon: {e}");
        }
    }
}
