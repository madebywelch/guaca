fn main() {
    println!("cargo:rerun-if-env-changed=GUACA_BACKEND_IMAGE");
    // Only the desktop host has a Tauri context to generate. A daemon build
    // has no window, no `dist/` and no `tauri.conf.json` to validate, and
    // running this for one fails on a frontend bundle it would never serve.
    if std::env::var_os("CARGO_FEATURE_DESKTOP").is_some() {
        tauri_build::build()
    }
}
