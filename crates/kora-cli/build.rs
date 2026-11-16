// Windows file metadata (consistent product name and version).
fn main() {
    println!("cargo:rerun-if-changed=../../assets/kora.ico");
    #[cfg(windows)]
    {
        let version = std::env::var("CARGO_PKG_VERSION").unwrap();
        let mut res = winresource::WindowsResource::new();
        res.set("ProductName", "Kora");
        res.set("FileDescription", "Kora installer and server manager");
        res.set("CompanyName", "Kora"); // TODO: confirm publisher/organisation name
        res.set("OriginalFilename", "kora.exe");
        res.set("ProductVersion", &version);
        res.set("FileVersion", &version);
        // The Kora fox (assets/README.md): the exe, Start Menu / Desktop shortcuts and Apps & features all use it.
        res.set_icon("../../assets/kora.ico");
        res.compile().expect("failed to compile Windows resources");
    }
}
