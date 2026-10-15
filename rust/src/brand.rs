//! AgentBar branding.
//!
//! AgentBar is a fork of Win-CodexBar (nesszer/Win-CodexBar), itself the
//! Windows port of CodexBar by Peter Steinberger. All three are MIT licensed;
//! see LICENSE, NOTICE and FORK.md.
//!
//! Branding lives here so the fork can keep merging upstream: localized
//! strings still say "CodexBar" in the `.ftl` files and are rebranded when
//! looked up, and internal names (data folders, binaries, crate names) are
//! left as upstream has them.

/// User-facing product name.
pub const APP_NAME: &str = "AgentBar";
/// Name of the upstream product the strings are written for.
pub const UPSTREAM_APP_NAME: &str = "CodexBar";
/// GitHub repository that publishes AgentBar releases (used by the updater).
pub const REPO: &str = "jbrito-dev/Win-CodexBar";

/// Swap the upstream product name for AgentBar in a user-facing string.
/// The upstream project name "Win-CodexBar" is kept (used in credits).
pub fn rebrand(text: String) -> String {
    if text.contains(UPSTREAM_APP_NAME) {
        // "Win-CodexBar" names the upstream project (fork credits): keep it.
        const KEEP: &str = "Win-CodexBar";
        const MARK: &str = "\u{1}WCB\u{1}";
        text.replace(KEEP, MARK)
            .replace(UPSTREAM_APP_NAME, APP_NAME)
            .replace(MARK, KEEP)
    } else {
        text
    }
}

/// AgentBar build number, set by the installer workflow (`AGENTBAR_BUILD`).
/// Local builds are 0.
pub fn build_number() -> u32 {
    option_env!("AGENTBAR_BUILD")
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

fn triplet(version: &str) -> (u32, u32, u32) {
    let mut parts = version
        .trim()
        .trim_start_matches('v')
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// Parse a release tag: `v0.70.0-ab.15` → ((0, 70, 0), 15); `v0.71.0` → build 0.
pub fn parse_release_tag(tag: &str) -> ((u32, u32, u32), u32) {
    let tag = tag.trim().trim_start_matches('v');
    let (version, suffix) = tag.split_once('-').unwrap_or((tag, ""));
    let build = suffix
        .strip_prefix("ab.")
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    (triplet(version), build)
}

/// Whether a release tag is newer than this build (`current` is the upstream
/// version this build was made from).
pub fn is_newer_release(tag: &str, current: &str) -> bool {
    is_newer_release_than(tag, current, build_number())
}

fn is_newer_release_than(tag: &str, current: &str, current_build: u32) -> bool {
    let (remote, remote_build) = parse_release_tag(tag);
    (remote, remote_build) > (triplet(current), current_build)
}

#[cfg(test)]
mod tests {
    #[test]
    fn compares_version_then_build() {
        assert!(is_newer_release_than("v0.70.0-ab.16", "0.70.0", 15));
        assert!(!is_newer_release_than("v0.70.0-ab.15", "0.70.0", 15));
        assert!(is_newer_release_than("v0.71.0-ab.2", "0.70.0", 15));
        assert!(!is_newer_release_than("v0.70.0-ab.99", "0.71.0", 1));
        assert!(is_newer_release_than("v0.70.0", "0.69.9", 0));
    }

    use super::*;

    #[test]
    fn rebrands_product_names_only() {
        assert_eq!(rebrand("About CodexBar".into()), "About AgentBar");
        assert_eq!(
            rebrand("fork of Win-CodexBar".into()),
            "fork of Win-CodexBar"
        );
        assert_eq!(rebrand("Codex Accounts".into()), "Codex Accounts");
    }
}
