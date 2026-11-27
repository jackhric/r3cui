use std::path::Path;
use std::process::Command;

fn main() {
    slint_build::compile("ui/app.slint").expect("compile slint");
    println!("cargo:rustc-env=R3CUI_VERSION={}", version());
}

/// What this build calls itself: `scripts/version.sh`, so the binary and the archive's name agree.
fn version() -> String {
    println!("cargo:rerun-if-env-changed=R3CUI_VERSION");
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // A commit or a tag changes the answer; these are where git records them.
    for watched in [".git/HEAD", ".git/refs", ".git/packed-refs", "Cargo.toml", "scripts/version.sh"] {
        let path = repo.join(watched);
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    Command::new("sh")
        .arg(repo.join("scripts/version.sh"))
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| std::env::var("CARGO_PKG_VERSION").expect("cargo sets it"))
}
