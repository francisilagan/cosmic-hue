//! Pick the panel icon: a local override in `res/icons/local/` (kept out
//! of git) wins over the bundled icon.

use std::path::Path;

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("res/icons");
    let local = dir.join("local/cosmic-hue-symbolic.svg");
    let icon = if local.is_file() {
        local
    } else {
        dir.join("cosmic-hue-symbolic.svg")
    };
    println!("cargo:rerun-if-changed=res/icons");
    println!("cargo:rerun-if-changed=res/icons/local");
    println!("cargo:rustc-env=COSMIC_HUE_ICON={}", icon.display());
}
