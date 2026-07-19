#[cfg(target_os = "macos")]
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

const GITHUB_MAIN_COMMIT_URL: &str = "https://api.github.com/repos/jkuang7/StickyMD/commits/main";
const INSTALLED_SHA: &str = env!("STICKY_INSTALLED_SHA");
const UPDATE_CHECK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Deserialize)]
struct GitHubCommit {
    sha: String,
}

#[derive(Serialize)]
pub struct UpdateStatus {
    installed_sha: String,
    latest_sha: String,
    update_available: bool,
}

fn valid_sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn installed_build_sha() -> Option<&'static str> {
    valid_sha(INSTALLED_SHA).then_some(INSTALLED_SHA)
}

#[tauri::command]
pub async fn check_for_update() -> Result<UpdateStatus, String> {
    let installed_sha = installed_build_sha()
        .ok_or_else(|| "This copy of Sticky has unknown build information and cannot be compared with GitHub. Reinstall Sticky to restore update checks.".to_string())?;
    let client = reqwest::Client::builder()
        .user_agent(concat!("Sticky/", env!("CARGO_PKG_VERSION")))
        .timeout(UPDATE_CHECK_TIMEOUT)
        .build()
        .map_err(|_| "Sticky could not prepare a secure connection to GitHub.".to_string())?;
    let response = client
        .get(GITHUB_MAIN_COMMIT_URL)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2026-03-10")
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                "The update check timed out. Check your internet connection and try again."
                    .to_string()
            } else {
                "Could not reach GitHub. Check your internet connection and try again.".to_string()
            }
        })?;

    let status = response.status();
    if !status.is_success() {
        return if status.as_u16() == 403 || status.as_u16() == 429 {
            Err("GitHub's request limit was reached. Try the update check again later.".to_string())
        } else {
            Err(format!(
                "GitHub could not complete the update check (HTTP {}). Try again later.",
                status.as_u16()
            ))
        };
    }

    let commit: GitHubCommit = response.json().await.map_err(|_| {
        "GitHub returned an invalid update response. Try the update check again later.".to_string()
    })?;
    let latest_sha = commit.sha.trim().to_ascii_lowercase();
    if !valid_sha(&latest_sha) {
        return Err(
            "GitHub returned an invalid commit identifier. Try the update check again later."
                .to_string(),
        );
    }

    Ok(UpdateStatus {
        installed_sha: installed_sha.to_string(),
        update_available: installed_sha != latest_sha,
        latest_sha,
    })
}

#[tauri::command]
pub fn launch_update() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "Sticky could not locate your home folder.".to_string())?;
        let wrapper = PathBuf::from(home).join("StickyMD/scripts/update.command");
        if !wrapper.is_file() {
            return Err(
                "The Sticky update launcher is missing. Reinstall Sticky from the README instructions."
                    .to_string(),
            );
        }

        let status = std::process::Command::new("/usr/bin/open")
            .args(["-a", "Terminal"])
            .arg(wrapper)
            .status()
            .map_err(|_| "Sticky could not open the update in Terminal.".to_string())?;
        if !status.success() {
            return Err("Terminal could not start the Sticky update.".to_string());
        }
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        Err("Sticky updates are only supported on macOS.".to_string())
    }
}
