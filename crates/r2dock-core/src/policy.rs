//! Conflict policies for uploads and downloads.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ConflictPolicy {
    /// Stop and let the user decide for this item.
    #[default]
    Ask,
    Skip,
    Replace,
    /// Add " (1)" before the extension.
    KeepBoth,
}

/// Find the first free "Keep both" name. `exists` reports whether a name is taken.
/// Gives up after `limit` attempts.
pub fn first_free_name(
    name: &str,
    limit: u32,
    mut exists: impl FnMut(&str) -> bool,
) -> Option<String> {
    (1..=limit)
        .map(|n| crate::keys::keep_both_name(name, n))
        .find(|candidate| !exists(candidate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_first_free_name() {
        let taken = ["a (1).txt", "a (2).txt"];
        let name = first_free_name("a.txt", 10, |n| taken.contains(&n));
        assert_eq!(name.as_deref(), Some("a (3).txt"));
        assert_eq!(first_free_name("a.txt", 2, |n| taken.contains(&n)), None);
    }
}
