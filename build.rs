//! Bettet den Git-Commit ein, aus dem gebaut wird (für die Update-Prüfung).
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=BFL_COMMIT");
    println!("cargo:rerun-if-env-changed=BFL_REPO");
    println!("cargo:rerun-if-env-changed=BFL_COMMIT_TIME");
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

    // Commit-Zeitpunkt (Unix-Zeit): verhindert, dass ein älterer Stand als Update gilt.
    let time = std::env::var("BFL_COMMIT_TIME")
        .ok()
        .filter(|t| t.parse::<u64>().is_ok())
        .or_else(|| {
            Command::new("git")
                .args(["log", "-1", "--format=%ct"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|t| t.parse::<u64>().is_ok())
        })
        .unwrap_or_else(|| "0".into());
    println!("cargo:rustc-env=BFL_COMMIT_TIME={time}");
}
