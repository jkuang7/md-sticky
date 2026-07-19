use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

const BUILD_SHA_OVERRIDE: &str = "STICKY_BUILD_SHA";

fn git_output(manifest_dir: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(manifest_dir)
        .args(arguments)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn valid_sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn watch_git_metadata(manifest_dir: &Path) {
    let Some(head_path) = git_output(
        manifest_dir,
        &["rev-parse", "--path-format=absolute", "--git-path", "HEAD"],
    ) else {
        return;
    };
    println!("cargo:rerun-if-changed={head_path}");

    if let Some(reference) = git_output(manifest_dir, &["symbolic-ref", "-q", "HEAD"]) {
        if let Some(reference_path) = git_output(
            manifest_dir,
            &[
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                &reference,
            ],
        ) {
            println!("cargo:rerun-if-changed={reference_path}");
        }
        if let Some(packed_refs_path) = git_output(
            manifest_dir,
            &[
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                "packed-refs",
            ],
        ) {
            println!("cargo:rerun-if-changed={packed_refs_path}");
        }
    }
}

fn installed_sha(manifest_dir: &Path) -> String {
    if let Ok(value) = env::var(BUILD_SHA_OVERRIDE) {
        let value = value.trim().to_ascii_lowercase();
        if valid_sha(&value) {
            return value;
        }
        println!(
            "cargo:warning={BUILD_SHA_OVERRIDE} must be a full 40-character hexadecimal commit SHA"
        );
        return String::new();
    }

    git_output(manifest_dir, &["rev-parse", "--verify", "HEAD"])
        .filter(|value| valid_sha(value))
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default()
}

fn main() {
    println!("cargo:rerun-if-env-changed={BUILD_SHA_OVERRIDE}");
    let manifest_dir = env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    watch_git_metadata(&manifest_dir);
    println!(
        "cargo:rustc-env=STICKY_INSTALLED_SHA={}",
        installed_sha(&manifest_dir)
    );
    tauri_build::build()
}
