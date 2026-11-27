use std::path::Path;
use std::process::Command;

fn main() {
    slint_build::compile("ui/app.slint").expect("compile slint");
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // A commit or a tag changes the answers; these are where git records them.
    for watched in [".git/HEAD", ".git/refs", ".git/packed-refs", "Cargo.toml", "scripts/version.sh"] {
        let path = repo.join(watched);
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    println!("cargo:rerun-if-env-changed=R3CUI_VERSION");
    println!("cargo:rerun-if-env-changed=R3CUI_COMMIT");
    let version = ask(&repo, &[]).unwrap_or_else(|| std::env::var("CARGO_PKG_VERSION").expect("cargo sets it"));
    println!("cargo:rustc-env=R3CUI_VERSION={version}");
    println!("cargo:rustc-env=R3CUI_COMMIT={}", ask(&repo, &["--commit"]).unwrap_or_default());
}

/// What `scripts/version.sh` says, so the binary and the archive's name agree.
fn ask(repo: &Path, args: &[&str]) -> Option<String> {
    Command::new("sh")
        .arg(repo.join("scripts/version.sh"))
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}
