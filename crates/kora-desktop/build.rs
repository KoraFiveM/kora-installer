// Windows file metadata (consistent product name and version; SignPath requires product name "Kora").
fn main() {
    println!("cargo:rerun-if-changed=../../assets/kora.ico");
    #[cfg(windows)]
    {
        let version = std::env::var("CARGO_PKG_VERSION").unwrap();
        let mut res = winresource::WindowsResource::new();
        res.set("ProductName", "Kora");
        res.set("FileDescription", "Kora desktop app (tray icon and window)");
        res.set("CompanyName", "Kora");
        res.set("OriginalFilename", "kora-desktop.exe");
        res.set("ProductVersion", &version);
        res.set("FileVersion", &version);
        res.set_icon("../../assets/kora.ico");
        res.compile().expect("failed to compile Windows resources");
    }
}
