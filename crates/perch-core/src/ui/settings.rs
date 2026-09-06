//! The settings window's view-model: today's settings, the file they came
//! from, and anything wrong with that file. Same rule as every other
//! view-model here — every string a shell shows is finished by the time it
//! leaves this module.

use crate::settings::store::load;
use crate::settings::Settings;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct SettingsModel {
    pub settings: Settings,
    pub config_path: String,
    pub notes: Vec<String>,
    pub error: Option<String>,
}

/// Load `path` (this never fails outright — see `settings::store::load`) and
/// hand back everything the settings window needs: the settings themselves,
/// the exact path a user would find them at on disk, and anything `load`
/// had to say about the file it found there.
pub fn build_settings(path: &Path) -> SettingsModel {
    let loaded = load(path);
    SettingsModel {
        settings: loaded.settings,
        config_path: path.display().to_string(),
        notes: loaded.notes,
        error: loaded.error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_yields_defaults_with_no_notes_or_error() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let model = build_settings(&path);

        assert_eq!(model.settings, Settings::default());
        assert_eq!(model.config_path, path.display().to_string());
        assert!(model.notes.is_empty());
        assert!(model.error.is_none());
    }

    #[test]
    fn a_malformed_file_surfaces_a_parse_error_and_still_runs_on_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "this is not = = toml").unwrap();

        let model = build_settings(&path);

        assert_eq!(model.settings, Settings::default());
        let err = model.error.expect("a parse failure must be reported");
        assert!(
            err.contains("config.toml") || err.contains("parse"),
            "actionable: {err}"
        );
    }

    #[test]
    fn a_clamped_value_earns_a_note_and_no_error() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "[sessions]\npoll_seconds = 0\n").unwrap();

        let model = build_settings(&path);

        assert_eq!(model.settings.poll_seconds, 1, "clamped, not rejected");
        assert_eq!(model.notes.len(), 1);
        assert!(model.notes[0].contains("poll_seconds"));
        assert!(
            model.error.is_none(),
            "a clamp is not the same failure as a parse error"
        );
    }

    #[test]
    fn config_path_is_reported_verbatim() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested").join("config.toml");
        let model = build_settings(&path);
        assert_eq!(model.config_path, path.display().to_string());
    }
}
