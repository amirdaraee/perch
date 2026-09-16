//! Which terminal applications this machine has.
//!
//! Detection is a **stat of a known application bundle path** and nothing
//! more: no process is launched, no system service is asked, and nothing is
//! written. That matters twice over — Perch never writes outside its own
//! files, and a settings window that opened a terminal in order to find out
//! whether it existed would be absurd.
//!
//! Following `config.rs`, the function that reads the environment is a thin
//! wrapper over a pure one that takes the directories to look in, so the
//! interesting half is testable without a machine that happens to have
//! Ghostty installed.

use std::path::{Path, PathBuf};

/// A terminal offered by the Preferred terminal picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalChoice {
    /// What `preferred_terminal` stores, and what the launcher matches on.
    pub id: String,
    /// What the picker draws, finished here — including the "(not installed)"
    /// caveat, so no shell composes that phrase in its own words.
    pub label: String,
    /// The application's bundle identifier, for a launcher that would rather
    /// ask for an app by identity than by name. `None` for a terminal Perch
    /// does not know, because Perch cannot invent one.
    pub bundle_id: Option<String>,
    /// False for a row that exists only because the config names it.
    pub installed: bool,
}

/// A terminal Perch knows how to offer and to launch.
struct KnownTerminal {
    /// The stored value, which is also the name the user sees.
    id: &'static str,
    bundle_id: &'static str,
    /// Bundle paths, relative to an applications directory. More than one
    /// because an app's bundle is not always named after its menu name
    /// (iTerm2 ships `iTerm.app`), because Terminal.app sits a level down in
    /// `Utilities`, and because a case-sensitive volume distinguishes
    /// `kitty.app` from `Kitty.app` where the usual one does not.
    bundles: &'static [&'static str],
}

/// Every terminal Perch offers, in the order the picker lists them: Apple's
/// first, then the third-party ones roughly by how many people run them.
const KNOWN: &[KnownTerminal] = &[
    KnownTerminal {
        id: "Terminal",
        bundle_id: "com.apple.Terminal",
        bundles: &["Utilities/Terminal.app", "Terminal.app"],
    },
    KnownTerminal {
        id: "iTerm2",
        bundle_id: "com.googlecode.iterm2",
        bundles: &["iTerm.app", "iTerm2.app"],
    },
    KnownTerminal {
        id: "Warp",
        bundle_id: "dev.warp.Warp-Stable",
        bundles: &["Warp.app"],
    },
    KnownTerminal {
        id: "Ghostty",
        bundle_id: "com.mitchellh.ghostty",
        bundles: &["Ghostty.app"],
    },
    KnownTerminal {
        id: "Alacritty",
        bundle_id: "org.alacritty",
        bundles: &["Alacritty.app"],
    },
    KnownTerminal {
        id: "Kitty",
        bundle_id: "net.kovidgoyal.kitty",
        bundles: &["kitty.app", "Kitty.app"],
    },
    KnownTerminal {
        id: "WezTerm",
        bundle_id: "com.github.wez.wezterm",
        bundles: &["WezTerm.app"],
    },
];

impl KnownTerminal {
    /// A bundle is a directory. `is_dir` follows symlinks, which is what a
    /// Homebrew cask leaves in `/Applications`, and returns false rather than
    /// erroring for a root that does not exist at all.
    fn is_installed_in(&self, roots: &[PathBuf]) -> bool {
        roots.iter().any(|root| {
            self.bundles
                .iter()
                .any(|bundle| Path::new(root).join(bundle).is_dir())
        })
    }

    fn choice(&self, installed: bool) -> TerminalChoice {
        TerminalChoice {
            id: self.id.to_string(),
            label: label_for(self.id, installed),
            bundle_id: Some(self.bundle_id.to_string()),
            installed,
        }
    }
}

fn label_for(name: &str, installed: bool) -> String {
    if installed {
        name.to_string()
    } else {
        format!("{name} (not installed)")
    }
}

/// The directories macOS keeps applications in. `/System/Applications` is
/// where Terminal.app has lived since Catalina; `~/Applications` is where a
/// per-user install lands.
pub fn application_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
    ];
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            roots.push(Path::new(&home).join("Applications"));
        }
    }
    roots
}

/// The terminals installed under `roots`, in the picker's order.
pub fn detected_terminals_in(roots: &[PathBuf]) -> Vec<TerminalChoice> {
    KNOWN
        .iter()
        .filter(|t| t.is_installed_in(roots))
        .map(|t| t.choice(true))
        .collect()
}

/// The terminals installed on this machine.
pub fn detected_terminals() -> Vec<TerminalChoice> {
    detected_terminals_in(&application_roots())
}

/// The picker's rows: every detected terminal, plus whatever the config names
/// even when it was not detected.
///
/// The union is the point. A config hand-edited to a terminal that has since
/// been uninstalled — or that Perch has never heard of — must still render a
/// picker with its own value selected; a picker showing nothing selected
/// looks like a setting that was lost.
pub fn terminal_choices(detected: &[&str], configured: &str) -> Vec<TerminalChoice> {
    let mut out: Vec<TerminalChoice> = KNOWN
        .iter()
        .filter_map(|t| {
            let installed = detected.contains(&t.id);
            (installed || t.id == configured).then(|| t.choice(installed))
        })
        .collect();
    if !configured.is_empty() && !KNOWN.iter().any(|t| t.id == configured) {
        out.push(TerminalChoice {
            id: configured.to_string(),
            label: label_for(configured, false),
            bundle_id: None,
            installed: false,
        });
    }
    out
}

/// Terminal.app: the fallback whenever the configured terminal is one Perch
/// has no bundle identifier for, and the one macOS is guaranteed to have.
const FALLBACK: &str = "com.apple.Terminal";

/// The bundle identifier a launcher should ask macOS for, given whatever the
/// config holds. A name Perch does not know — a hand-edited value, or a
/// terminal newer than this build — falls back to Terminal.app, because a
/// launch that goes nowhere is worse than one that goes somewhere ordinary.
///
/// This mapping lives here, beside the table it reads, so a terminal added to
/// `KNOWN` is launchable the moment it is offered. A shell that kept its own
/// name-to-identifier switch would silently send every terminal it had not
/// heard of to Terminal.app — which is exactly what happened to Warp.
pub fn bundle_id_for(configured: &str) -> String {
    KNOWN
        .iter()
        .find(|t| t.id == configured)
        .map_or_else(|| FALLBACK.to_string(), |t| t.bundle_id.to_string())
}

/// What this machine has, plus what the config chose. The one call a caller
/// building the Preferred terminal row needs.
pub fn terminal_picker(configured: &str) -> Vec<TerminalChoice> {
    let detected = detected_terminals();
    let ids: Vec<&str> = detected.iter().map(|c| c.id.as_str()).collect();
    terminal_choices(&ids, configured)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ids(choices: &[TerminalChoice]) -> Vec<&str> {
        choices.iter().map(|c| c.id.as_str()).collect()
    }

    #[test]
    fn every_offered_terminal_has_a_bundle_id_to_launch_it_by() {
        // The picker and the launcher read one table. Before this, Swift kept
        // its own switch with two entries in it, so choosing Warp opened
        // Terminal.app — the whole point of naming a terminal, undone.
        for t in KNOWN {
            assert_eq!(
                bundle_id_for(t.id),
                t.bundle_id,
                "{} is offered but would launch something else",
                t.id
            );
        }
        assert_eq!(bundle_id_for("Warp"), "dev.warp.Warp-Stable");
    }

    #[test]
    fn an_unknown_terminal_falls_back_to_the_one_macos_always_has() {
        assert_eq!(bundle_id_for("Hyper"), FALLBACK);
        assert_eq!(bundle_id_for(""), FALLBACK);
    }

    #[test]
    fn the_configured_terminal_is_always_offered_even_if_undetected() {
        // A hand-edited config naming a terminal we cannot see must not render
        // a picker with nothing selected -- the defect deferred from the last
        // milestone's review.
        let choices = terminal_choices(&["Terminal"], "Ghostty");
        assert!(choices.iter().any(|c| c.id == "Ghostty"));
        assert!(choices.iter().any(|c| c.id == "Terminal"));
    }

    #[test]
    fn an_undetected_choice_carries_a_finished_label_saying_so() {
        // Rust owns every string the picker draws, including the caveat: a
        // shell appending "(not installed)" itself is how two spellings of the
        // same phrase appear.
        let choices = terminal_choices(&["Terminal"], "Ghostty");
        let ghostty = choices.iter().find(|c| c.id == "Ghostty").unwrap();
        assert!(!ghostty.installed);
        assert_eq!(ghostty.label, "Ghostty (not installed)");
        let terminal = choices.iter().find(|c| c.id == "Terminal").unwrap();
        assert!(terminal.installed);
        assert_eq!(terminal.label, "Terminal");
    }

    #[test]
    fn a_terminal_perch_has_never_heard_of_is_still_offered_by_name() {
        let choices = terminal_choices(&[], "Some Future Term");
        let row = choices.iter().find(|c| c.id == "Some Future Term").unwrap();
        assert_eq!(row.label, "Some Future Term (not installed)");
        assert!(
            row.bundle_id.is_none(),
            "Perch cannot invent a bundle id for an app it does not know"
        );
    }

    #[test]
    fn a_configured_terminal_that_is_installed_appears_exactly_once() {
        let choices = terminal_choices(&["Terminal", "iTerm2"], "iTerm2");
        assert_eq!(
            ids(&choices).iter().filter(|id| **id == "iTerm2").count(),
            1
        );
        assert!(choices.iter().find(|c| c.id == "iTerm2").unwrap().installed);
    }

    #[test]
    fn the_choices_keep_the_known_order_regardless_of_detection_order() {
        let choices = terminal_choices(&["Ghostty", "Terminal"], "Terminal");
        assert_eq!(ids(&choices), vec!["Terminal", "Ghostty"]);
    }

    #[test]
    fn an_empty_configured_value_adds_no_row() {
        // Nothing is configured on a machine whose config has never been
        // written; an empty string is not a terminal.
        let choices = terminal_choices(&["Terminal"], "");
        assert_eq!(ids(&choices), vec!["Terminal"]);
    }

    #[test]
    fn every_terminal_the_launcher_can_open_is_offerable() {
        let all: Vec<&str> = KNOWN.iter().map(|t| t.id).collect();
        for want in [
            "Terminal",
            "iTerm2",
            "Warp",
            "Ghostty",
            "Alacritty",
            "Kitty",
            "WezTerm",
        ] {
            assert!(all.contains(&want), "{want} is not a known terminal");
        }
    }

    #[test]
    fn detection_finds_a_bundle_on_disk_and_ignores_a_missing_one() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("Ghostty.app")).unwrap();
        std::fs::create_dir(root.path().join("Utilities")).unwrap();
        std::fs::create_dir(root.path().join("Utilities/Terminal.app")).unwrap();

        let roots = vec![root.path().to_path_buf()];
        let detected = detected_terminals_in(&roots);
        let found = ids(&detected);
        assert!(found.contains(&"Ghostty"));
        assert!(
            found.contains(&"Terminal"),
            "Terminal.app lives one level down, under Utilities"
        );
        assert!(!found.contains(&"Alacritty"), "nothing else is installed");
    }

    #[test]
    fn a_bundle_named_differently_from_its_menu_name_is_still_found() {
        // iTerm2's bundle on disk is `iTerm.app`; matching on the name Perch
        // stores would miss every installation of it.
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("iTerm.app")).unwrap();
        assert_eq!(
            ids(&detected_terminals_in(&[root.path().to_path_buf()])),
            vec!["iTerm2"]
        );
    }

    #[test]
    fn detection_reads_and_never_writes() {
        // The read-only promise, asserted rather than assumed: detection is a
        // stat of each known bundle path, so a directory it looked at is
        // exactly as it was.
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("Ghostty.app")).unwrap();
        let before: Vec<PathBuf> = std::fs::read_dir(root.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();

        let _ = detected_terminals_in(&[root.path().to_path_buf()]);

        let after: Vec<PathBuf> = std::fs::read_dir(root.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(before, after);
    }

    #[test]
    fn a_root_that_does_not_exist_is_not_an_error() {
        assert!(detected_terminals_in(&[PathBuf::from("/no/such/place")]).is_empty());
    }

    #[test]
    fn the_picker_always_has_the_configured_row_whatever_this_machine_has() {
        // The end-to-end call, run against the real machine: whatever is or
        // is not installed here, the stored value is always selectable.
        let choices = terminal_picker("Ghostty");
        assert!(choices.iter().any(|c| c.id == "Ghostty"));
    }
}
