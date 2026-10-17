//! User settings (DESIGN.md Screen 7). Stored as versioned JSON.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::policy::ConflictPolicy;

pub const SETTINGS_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Theme {
    #[default]
    Dark,
    Paper,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Density {
    #[default]
    Comfortable,
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TextSize {
    #[default]
    Default,
    Large,
    Larger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ViewMode {
    #[default]
    List,
    Grid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LogLevel {
    #[default]
    Standard,
    Detailed,
}

/// Settings the frontend may read and change. The download folder is not
/// here: it is only changed through a Rust-side folder picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct Preferences {
    pub theme: Theme,
    pub density: Density,
    pub text_size: TextSize,
    pub show_thumbnails: bool,
    pub default_view: ViewMode,
    pub show_details_panel: bool,
    pub concurrency: u8,
    pub ask_where_to_save: bool,
    pub conflict_policy: ConflictPolicy,
    pub history_days: u32,
    pub multipart_threshold_mib: u32,
    pub typed_confirm_threshold: u32,
    pub warn_executables: bool,
    pub notify_on_complete: bool,
    pub notify_on_failure: bool,
    pub log_level: LogLevel,
    pub include_hidden_os_files: bool,
    pub rename_notice_seen: bool,
    /// Record user actions and their results in the activity log.
    pub activity_log: bool,
    /// Write bucket, folder, and file names in the activity log instead of hashes.
    pub activity_include_names: bool,
    /// Days to keep activity log files.
    pub activity_days: u32,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            density: Density::Comfortable,
            text_size: TextSize::Default,
            show_thumbnails: true,
            default_view: ViewMode::List,
            show_details_panel: true,
            concurrency: 3,
            ask_where_to_save: false,
            conflict_policy: ConflictPolicy::Ask,
            history_days: 30,
            multipart_threshold_mib: 100,
            typed_confirm_threshold: 50,
            warn_executables: true,
            notify_on_complete: true,
            notify_on_failure: true,
            log_level: LogLevel::Standard,
            include_hidden_os_files: false,
            rename_notice_seen: false,
            activity_log: true,
            activity_include_names: false,
            activity_days: 14,
        }
    }
}

impl Preferences {
    /// Clamp every numeric value into its allowed range.
    pub fn sanitized(mut self) -> Self {
        self.concurrency = self.concurrency.clamp(1, 8);
        self.history_days = self.history_days.clamp(1, 365);
        self.multipart_threshold_mib = self.multipart_threshold_mib.clamp(16, 4096);
        self.typed_confirm_threshold = self.typed_confirm_threshold.clamp(1, 10_000);
        self.activity_days = self.activity_days.clamp(1, 365);
        self
    }
}

/// The settings file on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SettingsFile {
    pub version: u32,
    pub preferences: Preferences,
    /// Absolute download folder chosen with the folder picker. `None` means
    /// the system Downloads folder.
    pub download_dir: Option<std::path::PathBuf>,
    /// Activity log folder chosen with the folder picker. `None` means the
    /// `activity` folder inside the app's log folder.
    pub activity_dir: Option<std::path::PathBuf>,
}

impl Default for SettingsFile {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            preferences: Preferences::default(),
            download_dir: None,
            activity_dir: None,
        }
    }
}

impl SettingsFile {
    /// Bring an older or partially written file up to the current version.
    pub fn migrate(mut self) -> Self {
        if self.version != SETTINGS_VERSION {
            // Version 1 is the first format; unknown versions keep known fields
            // (serde defaults fill the rest) and are re-stamped.
            self.version = SETTINGS_VERSION;
        }
        self.preferences = self.preferences.sanitized();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_design() {
        let p = Preferences::default();
        assert_eq!(p.concurrency, 3);
        assert_eq!(p.multipart_threshold_mib, 100);
        assert_eq!(p.typed_confirm_threshold, 50);
        assert_eq!(p.history_days, 30);
        assert_eq!(p.theme, Theme::Dark);
        assert!(p.activity_log);
        assert!(!p.activity_include_names, "names are private by default");
        assert_eq!(p.activity_days, 14);
    }

    #[test]
    fn partial_file_fills_defaults() {
        let file: SettingsFile =
            serde_json::from_str(r#"{"preferences":{"theme":"paper"}}"#).unwrap();
        let file = file.migrate();
        assert_eq!(file.version, SETTINGS_VERSION);
        assert_eq!(file.preferences.theme, Theme::Paper);
        assert_eq!(file.preferences.concurrency, 3);
    }

    #[test]
    fn values_are_clamped() {
        let p = Preferences {
            concurrency: 50,
            multipart_threshold_mib: 1,
            ..Default::default()
        }
        .sanitized();
        assert_eq!(p.concurrency, 8);
        assert_eq!(p.multipart_threshold_mib, 16);
    }

    #[test]
    fn future_version_is_restamped() {
        let file: SettingsFile = serde_json::from_str(r#"{"version":99}"#).unwrap();
        assert_eq!(file.migrate().version, SETTINGS_VERSION);
    }
}
