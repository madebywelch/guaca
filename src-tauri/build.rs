fn main() {
    println!("cargo:rerun-if-env-changed=GUACA_RELEASE");
    println!("cargo:rerun-if-env-changed=GUACA_COMMIT");
    println!("cargo:rerun-if-env-changed=GUACA_BACKEND_IMAGE");
    println!("cargo:rerun-if-env-changed=GUACA_BROWSER_IMAGE");
    if std::env::var("GUACA_RELEASE").as_deref() == Ok("1") {
        let commit = std::env::var("GUACA_COMMIT").unwrap_or_default();
        assert!(
            commit.len() == 40
                && commit.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
            "Published releases require GUACA_COMMIT as a full source SHA"
        );
        if std::env::var_os("CARGO_FEATURE_DESKTOP").is_some() {
            let image = std::env::var("GUACA_BACKEND_IMAGE").unwrap_or_default();
            let digest =
                image.strip_prefix("ghcr.io/madebywelch/guaca/guacad@sha256:").unwrap_or_default();
            assert!(digest.len() == 64 && digest.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)), "Published desktop releases require GUACA_BACKEND_IMAGE pinned to the published digest");
            let browser = std::env::var("GUACA_BROWSER_IMAGE").unwrap_or_default();
            let digest = browser
                .strip_prefix("ghcr.io/madebywelch/guaca/browser@sha256:")
                .unwrap_or_default();
            assert!(digest.len() == 64 && digest.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)), "Published desktop releases require GUACA_BROWSER_IMAGE pinned to the digest the same release run published");
        }
    }
    if std::env::var_os("CARGO_FEATURE_DESKTOP").is_some() {
        tauri_build::build()
    }
}
