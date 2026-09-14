//! The settings window's view-model: today's settings, the file they came
//! from, and anything wrong with that file. Same rule as every other
//! view-model here — every string a shell shows is finished by the time it
//! leaves this module.

use crate::settings::store::load;
use crate::settings::Settings;
use crate::ui::format::plural;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct SettingsModel {
    pub settings: Settings,
    pub config_path: String,
    pub notes: Vec<String>,
    pub error: Option<String>,
}

/// The "Check every N seconds" caption under the Sessions pane's poll
/// stepper, for the value that stepper is *currently* showing. It exists for
/// the same reason `main_window::custom_notify_label` does: a shell renders
/// this string, it does not compose it, so the number is formatted and
/// pluralized here — where "1 second" is spelled correctly — and never in a
/// UI's own interpolation.
pub fn poll_seconds_label(seconds: u32) -> String {
    format!(
        "Check every {}",
        plural(i64::from(seconds), "second", "seconds")
    )
}

/// The "After N minutes" caption under the Notifications pane's threshold
/// stepper, on the same terms as `poll_seconds_label` above.
///
/// It reads identically to `main_window::custom_notify_label` today, and is
/// deliberately not routed through it: that one captions a single project's
/// override, this one the global default those overrides depart from, and
/// either window's wording can move without dragging the other with it.
pub fn waiting_after_minutes_label(minutes: u32) -> String {
    format!("After {}", plural(i64::from(minutes), "minute", "minutes"))
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
    fn the_poll_stepper_caption_is_pluralized_here_not_in_the_shell() {
        assert_eq!(poll_seconds_label(1), "Check every 1 second");
        assert_eq!(poll_seconds_label(5), "Check every 5 seconds");
    }

    #[test]
    fn the_waiting_stepper_caption_is_pluralized_here_not_in_the_shell() {
        assert_eq!(waiting_after_minutes_label(1), "After 1 minute");
        assert_eq!(waiting_after_minutes_label(10), "After 10 minutes");
    }

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
    fn a_claude_config_dir_that_is_not_a_directory_earns_a_note_the_window_can_show() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let not_a_dir = tmp.path().join("regular-file");
        std::fs::write(&not_a_dir, b"i am a file").unwrap();
        std::fs::write(
            &path,
            format!(
                "[general]\nclaude_config_dir = {:?}\n",
                not_a_dir.to_string_lossy()
            ),
        )
        .unwrap();

        let model = build_settings(&path);

        assert_eq!(
            model.settings.claude_config_dir, "",
            "the window must not echo back a value nothing will honour"
        );
        assert_eq!(model.notes.len(), 1);
        assert!(
            model.notes[0].contains("claude_config_dir"),
            "actionable: {}",
            model.notes[0]
        );
        assert!(
            model.error.is_none(),
            "a rejected directory is not a file that failed to parse"
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
