//! Manual QA: run the wizard and print the resulting InstallRequest as JSON (key redacted).
fn main() -> Result<(), Box<dyn std::error::Error>> {
    match kora_tui::run_install_wizard()? {
        Some(req) => {
            let mut v = serde_json::to_value(&req)?;
            if v["license_key"].is_string() {
                v["license_key"] = "<redacted>".into();
            }
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        None => println!("cancelled"),
    }
    Ok(())
}
