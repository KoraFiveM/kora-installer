// Windows file metadata (SignPath requires consistent product name/version).
fn main() {
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
        // TODO: add an icon via res.set_icon("assets/kora.ico") once artwork exists.
        res.compile().expect("failed to compile Windows resources");
    }
}
