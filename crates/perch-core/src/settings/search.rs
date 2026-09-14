//! Searching the settings schema.
//!
//! A function over what [`build_schema`](super::build_schema) returns, not a
//! hand-maintained index beside it: a search list written in a shell is a
//! second copy of every label and every help string, and the copy is what
//! goes stale when a row is renamed.

use super::schema::SettingsPane;

/// The panes and rows that match `query`.
///
/// A row matches on its label, its help text, or one of its `search_terms` —
/// the synonyms a user is likely to type instead of the word the row happens
/// to use. Matching is case-insensitive, and the query is trimmed.
///
/// Three rules, each of which a filter gets wrong in a way worth naming:
///
/// - An empty or whitespace-only query returns everything, unchanged.
/// - A query that matches nothing returns **nothing**. Falling back to
///   "everything" when a filter finds no hits is indistinguishable from a
///   broken filter, and it tells the user their nonsense query was a real
///   setting.
/// - A pane whose *title* matches keeps all of its rows. This is what lets
///   search reach [`PaneId::Prices`](super::PaneId::Prices),
///   [`PaneId::Diagnostics`](super::PaneId::Diagnostics) and
///   [`PaneId::Advanced`](super::PaneId::Advanced), which carry no rows at
///   all because their content is drawn bespoke: without it, typing the name
///   of a pane that is plainly in the sidebar would find nothing.
pub fn filter_panes(panes: &[SettingsPane], query: &str) -> Vec<SettingsPane> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return panes.to_vec();
    }

    panes
        .iter()
        .filter_map(|pane| {
            if pane.title.to_lowercase().contains(&needle) {
                return Some(pane.clone());
            }

            let groups: Vec<_> = pane
                .groups
                .iter()
                .filter_map(|group| {
                    let rows: Vec<_> = group
                        .rows
                        .iter()
                        .filter(|row| row_matches(row, &needle))
                        .cloned()
                        .collect();
                    (!rows.is_empty()).then(|| super::schema::SettingGroup {
                        heading: group.heading.clone(),
                        rows,
                    })
                })
                .collect();

            (!groups.is_empty()).then(|| SettingsPane {
                groups,
                ..pane.clone()
            })
        })
        .collect()
}

/// Label, help and the row's own synonyms — never the control's contents. A
/// picker's option labels are values, not names for the setting, and matching
/// them would make "Terminal" reach every row whose choice list happens to
/// mention one.
fn row_matches(row: &super::schema::SettingRow, needle: &str) -> bool {
    row.label.to_lowercase().contains(needle)
        || row
            .help
            .as_deref()
            .is_some_and(|h| h.to_lowercase().contains(needle))
        || row
            .search_terms
            .iter()
            .any(|t| t.to_lowercase().contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::schema::test_support::*;
    use crate::settings::{build_schema, PaneId, SchemaContext, SettingKey, Settings};

    #[test]
    fn search_matches_a_label() {
        let panes = build_schema(&Settings::default(), &SchemaContext::empty());
        let hits = filter_panes(&panes, "terminal");
        assert_eq!(count_rows(&hits), 1);
        assert_eq!(first_row(&hits).key, Some(SettingKey::PreferredTerminal));
    }

    #[test]
    fn search_matches_help_text_not_only_labels() {
        // "hourglass" appears in the menu-bar display option's help, not its label.
        let panes = build_schema(&Settings::default(), &SchemaContext::empty());
        assert!(count_rows(&filter_panes(&panes, "hourglass")) > 0);
    }

    #[test]
    fn search_ignores_case_and_surrounding_space() {
        let panes = build_schema(&Settings::default(), &SchemaContext::empty());
        assert_eq!(count_rows(&filter_panes(&panes, "  TERMINAL ")), 1);
    }

    #[test]
    fn an_empty_query_returns_everything_unchanged() {
        let panes = build_schema(&Settings::default(), &SchemaContext::empty());
        assert_eq!(count_rows(&filter_panes(&panes, "   ")), count_rows(&panes));
        assert_eq!(filter_panes(&panes, "   "), panes, "and byte for byte");
    }

    #[test]
    fn a_query_that_matches_nothing_returns_nothing() {
        // The failure mode worth naming: a filter that falls back to "everything"
        // when it matches nothing is indistinguishable from a broken filter.
        let panes = build_schema(&Settings::default(), &SchemaContext::empty());
        assert_eq!(count_rows(&filter_panes(&panes, "zzzznotasetting")), 0);
        assert!(filter_panes(&panes, "zzzznotasetting").is_empty());
    }

    #[test]
    fn a_pane_that_keeps_no_rows_is_dropped_entirely() {
        let panes = build_schema(&Settings::default(), &SchemaContext::empty());
        let hits = filter_panes(&panes, "terminal");
        assert_eq!(hits.len(), 1, "only the pane holding the match survives");
    }

    #[test]
    fn search_reaches_a_rowless_pane_by_its_title() {
        // Prices has no rows at all — its content is a bespoke price table —
        // so a filter that only looked at rows could never find it, however
        // plainly "Prices" sits in the sidebar.
        let panes = build_schema(&Settings::default(), &SchemaContext::empty());
        let hits = filter_panes(&panes, "prices");
        let kept = pane(&hits, PaneId::Prices);
        assert!(kept.groups.is_empty(), "it had no rows to keep");
    }
}
