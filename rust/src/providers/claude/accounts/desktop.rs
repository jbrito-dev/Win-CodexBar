//! Make Claude Desktop and running Claude Code CLI sessions follow a Claude
//! account switch.
//!
//! Claude Desktop keeps its own login (Electron profile data), separate from
//! the Claude Code CLI credentials that `AccountManager::switch` moves. To make
//! Desktop follow a switch, its login state is stashed per saved account:
//! on a switch the outgoing account's Desktop session is saved under
//! `%APPDATA%\CodexBar\claude-desktop-sessions\<account id>` and the incoming
//! account's saved session (if any) is restored. An account without a saved
//! Desktop session gets a clean Desktop profile, so Desktop asks to sign in
//! once for that account; later switches restore it automatically.
//!
//! `claude_desktop_config.json` (MCP servers) and other app settings are not
//! in the swapped entry list and stay shared between accounts.

use std::io;
use std::path::{Path, PathBuf};

/// Electron profile entries that hold Claude Desktop's signed-in session.
pub const DESKTOP_SESSION_ENTRIES: &[&str] = &[
    "blob_storage",
    "config.json",
    "Cookies",
    "Cookies-journal",
    "DIPS",
    "DIPS-wal",
    "IndexedDB",
    "Local State",
    "Local Storage",
    "Network",
    "Partitions",
    "Preferences",
    "Session Storage",
    "SharedStorage",
    "SharedStorage-wal",
    "shared_proto_db",
];

/// What `stop_claude_processes` found and stopped.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoppedClaudeProcesses {
    /// Claude Code CLI processes that were stopped.
    #[serde(default)]
    pub cli_stopped: usize,
    /// Executable of the Claude Desktop instance that was running, if any.
    #[serde(default)]
    pub desktop_path: Option<String>,
}

/// Claude Desktop's Electron profile directory: the MSIX package's
/// virtualized roaming folder when present, else `%APPDATA%\Claude`.
pub fn desktop_session_root() -> Option<PathBuf> {
    if let Some(local) = dirs::data_local_dir() {
        let packages = local.join("Packages");
        if let Ok(entries) = std::fs::read_dir(&packages) {
            let mut candidates: Vec<PathBuf> = entries
                .filter_map(|entry| entry.ok().map(|e| e.path()))
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.to_ascii_lowercase().contains("claude"))
                })
                .map(|path| path.join("LocalCache").join("Roaming").join("Claude"))
                .filter(|path| path.is_dir())
                .collect();
            candidates.sort();
            if let Some(found) = candidates.into_iter().next() {
                return Some(found);
            }
        }
    }
    dirs::config_dir()
        .map(|roaming| roaming.join("Claude"))
        .filter(|path| path.is_dir())
}

fn sessions_root() -> io::Result<PathBuf> {
    dirs::config_dir()
        .map(|dir| dir.join("CodexBar").join("claude-desktop-sessions"))
        .ok_or_else(|| io::Error::other("Configuration directory not found."))
}

/// Directory name for a saved account id (`accountUuid:organizationUuid`).
fn safe_id(id: &str) -> io::Result<String> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':')
    {
        return Err(io::Error::other("Invalid Claude account id."));
    }
    Ok(id.replace(':', "_"))
}

fn remove_entry(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn copy_entry(source: &Path, destination: &Path) -> io::Result<()> {
    let meta = match std::fs::symlink_metadata(source) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if meta.is_dir() {
        std::fs::create_dir_all(destination)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            copy_entry(&entry.path(), &destination.join(entry.file_name()))?;
        }
        Ok(())
    } else if meta.is_file() {
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Chromium lock/journal files can be briefly locked; skip those.
        match std::fs::copy(source, destination) {
            Ok(_) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => Ok(()),
            Err(error) => Err(error),
        }
    } else {
        Ok(())
    }
}

/// Save the Desktop session in `root` as `from_id`'s, then restore `to_id`'s
/// saved session (or clear it so Desktop asks to sign in). Desktop must not be
/// running. Returns true when a saved session for `to_id` was restored.
pub fn swap_desktop_session_in(
    root: &Path,
    store: &Path,
    from_id: Option<&str>,
    to_id: &str,
) -> io::Result<bool> {
    let to_dir = store.join(safe_id(to_id)?);
    if let Some(from_id) = from_id {
        let from_dir = store.join(safe_id(from_id)?);
        let staging = store.join(format!("{}.saving", safe_id(from_id)?));
        remove_entry(&staging)?;
        std::fs::create_dir_all(&staging)?;
        for entry in DESKTOP_SESSION_ENTRIES {
            copy_entry(&root.join(entry), &staging.join(entry))?;
        }
        remove_entry(&from_dir)?;
        std::fs::rename(&staging, &from_dir)?;
    }
    let restored = to_dir.is_dir();
    for entry in DESKTOP_SESSION_ENTRIES {
        remove_entry(&root.join(entry))?;
    }
    if restored {
        for entry in DESKTOP_SESSION_ENTRIES {
            copy_entry(&to_dir.join(entry), &root.join(entry))?;
        }
    }
    Ok(restored)
}

/// [`swap_desktop_session_in`] for the installed Claude Desktop. A machine
/// without Claude Desktop is not an error (nothing to swap).
pub fn swap_desktop_session(from_id: Option<&str>, to_id: &str) -> io::Result<bool> {
    let Some(root) = desktop_session_root() else {
        return Ok(false);
    };
    let store = sessions_root()?;
    std::fs::create_dir_all(&store)?;
    swap_desktop_session_in(&root, &store, from_id, to_id)
}

#[cfg(windows)]
const STOP_SCRIPT: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
function Get-ClaudeProcesses {
    Get-Process | Where-Object {
        $_.ProcessName -eq 'claude' -or $_.Path -eq $env:CODEXBAR_CLAUDE_EXE -or
        $_.Path -like '*\.local\share\claude\versions\*'
    }
}
function Test-Desktop($p) { $p.MainModule.FileVersionInfo.ProductName -eq 'Claude' }
$cli = @(); $desktop = @()
foreach ($p in Get-ClaudeProcesses) {
    if (Test-Desktop $p) { $desktop += $p } else { $cli += $p }
}
# npm-installed Claude Code runs under node.exe.
$nodeCli = @(Get-CimInstance Win32_Process -Filter "Name='node.exe'" | Where-Object {
    $_.CommandLine -match '@anthropic-ai[\\/]claude-code'
})
$desktopPath = $null
if ($desktop.Count -gt 0) { $desktopPath = ($desktop | Where-Object { $_.Path } | Select-Object -First 1).Path }
foreach ($p in $cli) { Stop-Process -Id $p.Id -Force }
foreach ($p in $nodeCli) { Stop-Process -Id $p.ProcessId -Force }
foreach ($p in $desktop) { Stop-Process -Id $p.Id -Force }
$deadline = (Get-Date).AddSeconds(10)
do {
    $left = @(Get-ClaudeProcesses) + @(Get-CimInstance Win32_Process -Filter "Name='node.exe'" | Where-Object {
        $_.CommandLine -match '@anthropic-ai[\\/]claude-code'
    })
    if ($left.Count -eq 0) { break }
    Start-Sleep -Milliseconds 250
} while ((Get-Date) -lt $deadline)
# Let Chromium release its profile file locks.
Start-Sleep -Milliseconds 600
@{ cliStopped = ($cli.Count + $nodeCli.Count); desktopPath = $desktopPath } | ConvertTo-Json -Compress
"#;

/// Stop every running Claude Code CLI session and Claude Desktop. Returns
/// what was stopped so Desktop can be relaunched afterwards.
pub fn stop_claude_processes() -> io::Result<StoppedClaudeProcesses> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let exe = super::login::executable()
            .ok()
            .and_then(|path| path.canonicalize().ok())
            .map(|path| {
                path.to_string_lossy()
                    .trim_start_matches(r"\\?\")
                    .to_string()
            })
            .unwrap_or_default();
        let output = std::process::Command::new("powershell.exe")
            .env("CODEXBAR_CLAUDE_EXE", exe)
            .args(["-NoProfile", "-NonInteractive", "-Command", STOP_SCRIPT])
            .creation_flags(0x0800_0000)
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other(
                "Could not close Claude Code and Claude Desktop.",
            ));
        }
        let text = String::from_utf8_lossy(&output.stdout);
        Ok(serde_json::from_str(text.trim()).unwrap_or_default())
    }
    #[cfg(not(windows))]
    {
        Ok(StoppedClaudeProcesses::default())
    }
}

#[cfg(windows)]
const LAUNCH_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$path = $env:CODEXBAR_CLAUDE_DESKTOP
if ($path -like '*\WindowsApps\*') {
    $package = Get-AppxPackage | Where-Object {
        $_.InstallLocation -and $path.StartsWith($_.InstallLocation.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)
    } | Select-Object -First 1
    if ($package) {
        [xml]$manifest = Get-Content -LiteralPath (Join-Path $package.InstallLocation 'AppxManifest.xml')
        $app = $manifest.Package.Applications.Application | Select-Object -First 1
        Start-Process -FilePath 'explorer.exe' -ArgumentList ("shell:AppsFolder\" + $package.PackageFamilyName + "!" + $app.Id)
        exit 0
    }
}
Start-Process -FilePath $path
"#;

/// Relaunch Claude Desktop from the executable that was running.
pub fn launch_desktop(path: &str) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let status = std::process::Command::new("powershell.exe")
            .env("CODEXBAR_CLAUDE_DESKTOP", path)
            .args(["-NoProfile", "-NonInteractive", "-Command", LAUNCH_SCRIPT])
            .creation_flags(0x0800_0000)
            .status()?;
        if !status.success() {
            return Err(io::Error::other("Could not reopen Claude Desktop."));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn swap_saves_outgoing_and_restores_incoming() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Claude");
        let store = temp.path().join("store");
        write(&root.join("config.json"), "A");
        write(&root.join("Local Storage/leveldb/1.log"), "A-ls");
        write(&root.join("claude_desktop_config.json"), "mcp");

        // First switch to B: A is saved, B has no session yet → cleared.
        assert!(!swap_desktop_session_in(&root, &store, Some("acct-a:org"), "acct-b:org").unwrap());
        assert!(!root.join("config.json").exists());
        assert_eq!(
            std::fs::read_to_string(root.join("claude_desktop_config.json")).unwrap(),
            "mcp"
        );
        assert_eq!(
            std::fs::read_to_string(store.join("acct-a_org/config.json")).unwrap(),
            "A"
        );

        // User signs in to B in Desktop.
        write(&root.join("config.json"), "B");

        // Back to A: B saved, A restored.
        assert!(swap_desktop_session_in(&root, &store, Some("acct-b:org"), "acct-a:org").unwrap());
        assert_eq!(
            std::fs::read_to_string(root.join("config.json")).unwrap(),
            "A"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("Local Storage/leveldb/1.log")).unwrap(),
            "A-ls"
        );
        assert_eq!(
            std::fs::read_to_string(store.join("acct-b_org/config.json")).unwrap(),
            "B"
        );
    }

    #[test]
    fn rejects_path_like_ids() {
        let temp = tempfile::tempdir().unwrap();
        assert!(swap_desktop_session_in(temp.path(), temp.path(), None, "../x").is_err());
    }
}
