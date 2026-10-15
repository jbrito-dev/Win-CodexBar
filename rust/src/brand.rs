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

#[cfg(test)]
mod tests {
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
