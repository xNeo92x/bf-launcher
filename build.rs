//! Bettet den Git-Commit ein, aus dem gebaut wird (für die Update-Prüfung).
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=BFL_COMMIT");
    println!("cargo:rerun-if-env-changed=BFL_REPO");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads");

    let commit = std::env::var("BFL_COMMIT")
        .ok()
        .filter(|c| !c.is_empty())
        .or_else(|| {
            Command::new("git")
                .args(["rev-parse", "HEAD"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        })
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=BFL_COMMIT={commit}");
}
