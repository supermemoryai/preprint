use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::path::Path;

#[cfg(feature = "embedded-agent-browser")]
use anyhow::Context;
use anyhow::bail;
#[cfg(feature = "embedded-agent-browser")]
use anyhow::anyhow;

#[cfg(feature = "embedded-agent-browser")]
pub const AGENT_BROWSER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[tracing::instrument(skip_all)]
pub async fn resolve_agent_browser() -> anyhow::Result<PathBuf> {
    let target = current_target()?;

    if let Some(sidecar) = sidecar_path(target) {
        if sidecar.exists() {
            return Ok(sidecar);
        }
    }

    #[cfg(feature = "embedded-agent-browser")]
    {
        if let Some(path) = materialise_embedded(target)? {
            return Ok(path);
        }
    }

    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.display().to_string()))
        .unwrap_or_else(|| "<unknown>".to_string());
    bail!(
        "could not locate agent-browser-{target}. \
         Looked next to the preprint executable ({exe_dir}). \
         If installed via npm, reinstall. If installed via cargo, this is a bug — please report."
    )
}

/// Sidecar layout used by the npm tarball: `bin/preprint-<plat>` and
/// `bin/agent-browser-<plat>` live in the same directory.
fn sidecar_path(target: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let parent = exe.parent()?;
    Some(parent.join(asset_name(target)))
}

#[cfg(feature = "embedded-agent-browser")]
fn materialise_embedded(target: &str) -> anyhow::Result<Option<PathBuf>> {
    let dest = cache_path(target)?;
    if dest.exists() {
        return Ok(Some(dest));
    }

    let Some(bytes) = embedded_bytes(target) else {
        return Ok(None);
    };

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create cache directory {}", parent.display()))?;
    }

    let tmp = dest.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp, bytes).with_context(|| format!("failed to write {}", tmp.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
            .with_context(|| format!("failed to chmod {}", tmp.display()))?;
    }

    std::fs::rename(&tmp, &dest)
        .with_context(|| format!("failed to rename {} to {}", tmp.display(), dest.display()))?;

    tracing::info!(
        path = %dest.display(),
        bytes = bytes.len(),
        "agent-browser materialised from embedded bundle"
    );
    Ok(Some(dest))
}

#[cfg(feature = "embedded-agent-browser")]
fn embedded_bytes(target: &str) -> Option<&'static [u8]> {
    match target {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        "darwin-arm64" => Some(include_bytes!("../agent-browser/agent-browser-darwin-arm64")),

        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        "darwin-x64" => Some(include_bytes!("../agent-browser/agent-browser-darwin-x64")),

        #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
        "linux-x64" => Some(include_bytes!("../agent-browser/agent-browser-linux-x64")),

        #[cfg(all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"))]
        "linux-arm64" => Some(include_bytes!("../agent-browser/agent-browser-linux-arm64")),

        #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "musl"))]
        "linux-musl-x64" => Some(include_bytes!("../agent-browser/agent-browser-linux-musl-x64")),

        #[cfg(all(target_os = "linux", target_arch = "aarch64", target_env = "musl"))]
        "linux-musl-arm64" => Some(include_bytes!("../agent-browser/agent-browser-linux-musl-arm64")),

        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        "win32-x64" => Some(include_bytes!("../agent-browser/agent-browser-win32-x64.exe")),

        _ => None,
    }
}

fn current_target() -> anyhow::Result<&'static str> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    Ok(match (os, arch) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", "x86_64") => "darwin-x64",
        ("linux", "aarch64") => {
            if is_musl() {
                "linux-musl-arm64"
            } else {
                "linux-arm64"
            }
        }
        ("linux", "x86_64") => {
            if is_musl() {
                "linux-musl-x64"
            } else {
                "linux-x64"
            }
        }
        ("windows", "x86_64") => "win32-x64",
        _ => bail!(
            "unsupported platform {os}-{arch}. supported: darwin-arm64, darwin-x64, linux-arm64, linux-x64, linux-musl-arm64, linux-musl-x64, win32-x64"
        ),
    })
}

#[cfg(target_os = "linux")]
fn is_musl() -> bool {
    use std::process::Command;
    if let Ok(output) = Command::new("ldd").arg("--version").output() {
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if combined.to_lowercase().contains("musl") {
            return true;
        }
    }
    Path::new("/lib/ld-musl-x86_64.so.1").exists()
        || Path::new("/lib/ld-musl-aarch64.so.1").exists()
}

#[cfg(not(target_os = "linux"))]
fn is_musl() -> bool {
    false
}

fn asset_name(target: &str) -> String {
    if target == "win32-x64" {
        format!("agent-browser-{target}.exe")
    } else {
        format!("agent-browser-{target}")
    }
}

#[cfg(feature = "embedded-agent-browser")]
fn cache_path(target: &str) -> anyhow::Result<PathBuf> {
    let base =
        dirs::cache_dir().ok_or_else(|| anyhow!("could not determine user cache directory"))?;
    Ok(base
        .join("preprint")
        .join("agent-browser")
        .join(AGENT_BROWSER_VERSION)
        .join(asset_name(target)))
}
