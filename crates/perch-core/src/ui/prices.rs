//! The prices pane's view-model.
//!
//! The price table is the one place a user can move a number that the whole
//! cost estimate hangs off, so this module answers two questions and not one:
//! *what does Perch price?* and *what is it silently failing to price?* The
//! second is the interesting one — a model with recorded turns and no price
//! row contributes every one of its tokens and none of its dollars, which
//! reads on every screen as work that cost nothing. Perch makes no network
//! requests and so cannot look a rate up; the only honest fix is to hand the
//! user the model id and let them type the rate, which is what
//! [`PricesModel::unpriced`] exists to make a one-click job rather than a
//! typing exercise against a model id nobody has memorised.
//!
//! Every string here is finished. The four rates cross as **text** in both
//! directions: the shell renders what it was given and hands back what the
//! user typed, and [`parse_rates`] is the single place a rate becomes a
//! number — so a rate that cannot be parsed is refused here, with a sentence
//! naming the column, rather than becoming a silent zero somewhere in a
//! shell's own number parsing.

use crate::db::Db;
use crate::pricing::{self, ModelPrice};
use crate::query;
use crate::ui::format::{human_tokens, plural};
use anyhow::Result;

/// The four rates as an editable row of text, exactly as they should appear
/// in four text fields and exactly as they come back from them.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
pub struct RateFields {
    pub input: String,
    pub output: String,
    pub cache_read: String,
    pub cache_write: String,
}

/// One priced model.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PriceRow {
    pub model: String,
    pub rates: RateFields,
    /// True only for a model Perch ships that still holds the rate Perch
    /// shipped — the reset affordance appears only where something drifted.
    pub is_default: bool,
    /// What this model has actually cost you in tokens, or `None` when Perch
    /// has never seen a turn from it: a priced model you have never used is
    /// not a problem, and saying "0 tokens" would imply it were one.
    pub usage_label: Option<String>,
}

/// A model with recorded turns and no price row.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct UnpricedModel {
    pub model: String,
    pub tokens_label: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PricesModel {
    /// Stated above the table, because a user who enters a per-token rate
    /// inflates every dollar figure in the app by a millionfold and nothing
    /// anywhere would look wrong.
    pub header: String,
    pub rows: Vec<PriceRow>,
    /// Every model in use that has no price, biggest first — the ones costing
    /// the estimate the most are the ones worth fixing first.
    pub unpriced: Vec<UnpricedModel>,
    /// The sentence introducing `unpriced`, or `None` when nothing is
    /// unpriced. Never a "0 models" line: an empty list says it better.
    pub unpriced_summary: Option<String>,
    /// The two facts that must survive an editable table.
    pub notes: Vec<String>,
    /// Shown in the confirmation before a reset, which discards user-added
    /// models as well as edits.
    pub reset_warning: String,
}

/// The header above the four rate columns. A constant rather than a computed
/// string so the one sentence that prevents a millionfold error has exactly
/// one spelling.
const HEADER: &str = "Rates are in US dollars per million tokens.";

const ESTIMATE_NOTE: &str = "Tokens are truth; dollars are an estimate. Perch counts tokens from \
                             your transcripts and multiplies them by this table.";

const NO_PRICE_NOTE: &str = "A model with no price row still counts every one of its tokens and \
                             adds no cost. Perch makes no network requests, so it never guesses a \
                             rate — a missing row is Perch saying it does not know, which is a \
                             different claim from free.";

const RESET_WARNING: &str = "This restores the rates Perch ships and removes every model you \
                             added. Any rate you typed is discarded.";

/// The prices pane in full: the table, what it is missing, and the sentences
/// that keep both honest.
pub fn build_prices(db: &Db) -> Result<PricesModel> {
    let used = query::usage_by_model(db)?;
    let priced = pricing::all_prices(db)?;

    let rows = priced
        .iter()
        .map(|r| {
            let tokens = used
                .iter()
                .find(|(model, _, _)| model == &r.model)
                .map(|(_, u, _)| u.total_tokens())
                .unwrap_or(0);
            PriceRow {
                model: r.model.clone(),
                rates: rate_fields(&r.price),
                is_default: r.is_default,
                usage_label: (tokens > 0).then(|| tokens_phrase(tokens)),
            }
        })
        .collect();

    let unpriced = unpriced_models(&priced, &used);
    let unpriced_summary = (!unpriced.is_empty()).then(|| unpriced_summary(unpriced.len()));

    Ok(PricesModel {
        header: HEADER.to_string(),
        rows,
        unpriced,
        unpriced_summary,
        notes: vec![ESTIMATE_NOTE.to_string(), NO_PRICE_NOTE.to_string()],
        reset_warning: RESET_WARNING.to_string(),
    })
}

/// How many models are in use with no price — the count behind the sidebar's
/// "3 unpriced" and the Prices preview's own sentence.
///
/// Shares [`unpriced_models`] with the pane rather than counting a second
/// way, so the badge, the preview and the list the user acts on can never
/// disagree about how many there are.
pub fn unpriced_count(db: &Db) -> Result<usize> {
    let priced = pricing::all_prices(db)?;
    let used = query::usage_by_model(db)?;
    Ok(unpriced_models(&priced, &used).len())
}

/// Models with recorded turns and no price row, biggest first.
///
/// **Zero-token models are excluded deliberately.** A real index carries
/// entries like `<synthetic>` that never spent a token; a model that has
/// never consumed anything cannot be costing the estimate anything either,
/// and listing it as an unpriced model in use would send the user to type a
/// rate for something that is not a model.
fn unpriced_models(
    priced: &[pricing::ModelPriceRow],
    used: &[(String, crate::model::TurnUsage, f64)],
) -> Vec<UnpricedModel> {
    let mut out: Vec<(u64, UnpricedModel)> = used
        .iter()
        .filter(|(model, u, _)| u.total_tokens() > 0 && !priced.iter().any(|p| &p.model == model))
        .map(|(model, u, _)| {
            let tokens = u.total_tokens();
            (
                tokens,
                UnpricedModel {
                    model: model.clone(),
                    tokens_label: tokens_phrase(tokens),
                },
            )
        })
        .collect();
    // Biggest first, ties broken by name so the list is stable rather than
    // whatever SQLite's `GROUP BY` happened to return.
    out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.model.cmp(&b.1.model)));
    out.into_iter().map(|(_, m)| m).collect()
}

fn unpriced_summary(n: usize) -> String {
    let models = plural(n as i64, "model", "models");
    format!(
        "{models} you have used {} no price, so {} tokens count and {} dollars do not. Perch \
         will not guess a rate; add one and every figure that model touches is corrected.",
        if n == 1 { "has" } else { "have" },
        if n == 1 { "its" } else { "their" },
        if n == 1 { "its" } else { "their" },
    )
}

/// "1 token", "999 tokens", "2.0M tokens" — the same phrasing the usage view
/// uses, so one model's total reads identically in both places.
fn tokens_phrase(n: u64) -> String {
    if n < 1_000 {
        plural(n as i64, "token", "tokens")
    } else {
        format!("{} tokens", human_tokens(n))
    }
}

/// A rate as it should appear in a text field a user is about to edit: the
/// number they typed, without the trailing zeros a fixed-precision format
/// would invent (`18.75`, not `18.750000`, and `15`, not `15.00`).
fn rate_field(v: f64) -> String {
    if !v.is_finite() {
        return String::new();
    }
    let mut s = format!("{v:.6}");
    if s.contains('.') {
        s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    s
}

fn rate_fields(p: &ModelPrice) -> RateFields {
    RateFields {
        input: rate_field(p.input_per_mtok),
        output: rate_field(p.output_per_mtok),
        cache_read: rate_field(p.cache_read_per_mtok),
        cache_write: rate_field(p.cache_write_per_mtok),
    }
}

/// The four columns, in the order they are drawn, each with the name the
/// error message uses. A user reading "that is not a rate" needs to know
/// *which* box they are looking for.
const COLUMNS: [&str; 4] = ["Input", "Output", "Cache read", "Cache write"];

/// Turn four typed strings into a price, or say — in a finished sentence
/// naming the column and quoting what was typed — why they are not one.
///
/// A blank field is refused rather than read as zero: a zero rate is a
/// legitimate thing to mean (a model that genuinely costs nothing for that
/// class) and must be typed, because inferring it from an empty box is how a
/// half-filled row becomes a confidently wrong cost.
pub fn parse_rates(fields: &RateFields) -> Result<ModelPrice, String> {
    let RateFields {
        input,
        output,
        cache_read,
        cache_write,
    } = fields;
    let values = [input, output, cache_read, cache_write];
    let mut parsed = [0.0_f64; 4];
    for (i, raw) in values.iter().enumerate() {
        parsed[i] = parse_rate(raw, COLUMNS[i])?;
    }
    Ok(ModelPrice {
        input_per_mtok: parsed[0],
        output_per_mtok: parsed[1],
        cache_read_per_mtok: parsed[2],
        cache_write_per_mtok: parsed[3],
    })
}

fn parse_rate(raw: &str, column: &str) -> Result<f64, String> {
    let text = raw.trim().trim_start_matches('$').trim();
    if text.is_empty() {
        return Err(format!(
            "{column} is empty. Enter a rate in dollars per million tokens — type 0 if that class \
             really is free."
        ));
    }
    match text.parse::<f64>() {
        Ok(v) if v.is_finite() && v >= 0.0 => Ok(v),
        Ok(v) if v < 0.0 => Err(format!(
            "{column} is “{text}”. A rate cannot be negative, so nothing was changed."
        )),
        _ => Err(format!(
            "{column} is “{text}”, which is not a number. Nothing was changed — the rate Perch \
             already had is still in use."
        )),
    }
}

/// A model id as it can be stored: trimmed, and never empty.
pub fn parse_model_id(raw: &str) -> Result<String, String> {
    let id = raw.trim();
    if id.is_empty() {
        return Err("Enter the model id exactly as it appears in your transcripts.".to_string());
    }
    Ok(id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::model::{Turn, TurnUsage};

    fn seeded_db() -> Db {
        let db = open_in_memory().unwrap();
        pricing::seed_default_prices(&db).unwrap();
        db.conn()
            .execute_batch(
                "INSERT INTO projects (id, slug, real_path) VALUES (1, 'p', '/p');
                 INSERT INTO sessions (id, project_id, file_path) VALUES ('s1', 1, '/p/s1.jsonl');",
            )
            .unwrap();
        db
    }

    fn turn(model: &str, input: u64) -> Turn {
        Turn {
            ts: 1_000,
            model: model.to_string(),
            usage: TurnUsage {
                input,
                ..Default::default()
            },
        }
    }

    #[test]
    fn a_model_in_use_with_no_price_is_listed_for_the_user_to_fix() {
        let db = seeded_db();
        db.insert_turns("s1", &[turn("claude-fable-5-1", 775_000_000)])
            .unwrap();

        let m = build_prices(&db).unwrap();

        assert_eq!(m.unpriced.len(), 1, "the gap must be visible, not implied");
        assert_eq!(m.unpriced[0].model, "claude-fable-5-1");
        assert_eq!(m.unpriced[0].tokens_label, "775.0M tokens");
        assert!(m.unpriced_summary.is_some());
    }

    #[test]
    fn a_zero_token_entry_is_not_an_unpriced_model_in_use() {
        // A real index carries `<synthetic>`: no tokens, no cost, not a model
        // anyone can price. Listing it would send the user to invent a rate
        // for something that is not a model.
        let db = seeded_db();
        db.insert_turns("s1", &[turn("<synthetic>", 0)]).unwrap();
        assert!(build_prices(&db).unwrap().unpriced.is_empty());
        assert_eq!(unpriced_count(&db).unwrap(), 0);
    }

    #[test]
    fn the_biggest_gap_is_listed_first() {
        let db = seeded_db();
        db.insert_turns(
            "s1",
            &[
                turn("claude-opus-4-7", 96_000_000),
                turn("claude-fable-5-1", 775_000_000),
                turn("claude-opus-4-8", 10_000_000),
            ],
        )
        .unwrap();

        let m = build_prices(&db).unwrap();
        let models: Vec<&str> = m.unpriced.iter().map(|u| u.model.as_str()).collect();
        assert_eq!(
            models,
            ["claude-fable-5-1", "claude-opus-4-7", "claude-opus-4-8"]
        );
    }

    #[test]
    fn the_count_the_badge_shows_is_the_length_of_the_list_the_user_acts_on() {
        let db = seeded_db();
        db.insert_turns(
            "s1",
            &[
                turn("claude-fable-5-1", 775_000_000),
                turn("<synthetic>", 0),
            ],
        )
        .unwrap();
        assert_eq!(
            unpriced_count(&db).unwrap(),
            build_prices(&db).unwrap().unpriced.len()
        );
    }

    #[test]
    fn a_priced_model_reports_what_it_has_actually_used() {
        let db = seeded_db();
        db.insert_turns("s1", &[turn("claude-opus-5", 2_000_000)])
            .unwrap();

        let m = build_prices(&db).unwrap();
        let opus = m.rows.iter().find(|r| r.model == "claude-opus-5").unwrap();
        assert_eq!(opus.usage_label.as_deref(), Some("2.0M tokens"));
        let unused = m
            .rows
            .iter()
            .find(|r| r.model == "claude-sonnet-5")
            .unwrap();
        assert_eq!(
            unused.usage_label, None,
            "a priced model you never used is not a zero, it is nothing to report"
        );
    }

    #[test]
    fn rates_cross_as_the_numbers_a_person_typed() {
        let db = seeded_db();
        let m = build_prices(&db).unwrap();
        let opus = m.rows.iter().find(|r| r.model == "claude-opus-5").unwrap();
        assert_eq!(opus.rates.input, "15");
        assert_eq!(opus.rates.cache_write, "18.75");
    }

    #[test]
    fn a_rate_that_is_not_a_number_is_refused_by_name() {
        let err = parse_rates(&RateFields {
            input: "15".into(),
            output: "seventy five".into(),
            cache_read: "1.5".into(),
            cache_write: "18.75".into(),
        })
        .unwrap_err();
        assert!(
            err.starts_with("Output is"),
            "the column must be named: {err}"
        );
        assert!(err.contains("Nothing was changed"));
    }

    #[test]
    fn a_blank_rate_is_refused_rather_than_read_as_zero() {
        let err = parse_rates(&RateFields {
            input: "15".into(),
            output: "".into(),
            cache_read: "1.5".into(),
            cache_write: "18.75".into(),
        })
        .unwrap_err();
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn a_negative_rate_is_refused() {
        let err = parse_rates(&RateFields {
            input: "-1".into(),
            output: "75".into(),
            cache_read: "1.5".into(),
            cache_write: "18.75".into(),
        })
        .unwrap_err();
        assert!(err.contains("cannot be negative"), "{err}");
    }

    #[test]
    fn a_dollar_sign_and_surrounding_space_are_forgiven() {
        let p = parse_rates(&RateFields {
            input: " $15 ".into(),
            output: "75".into(),
            cache_read: "1.5".into(),
            cache_write: "18.75".into(),
        })
        .unwrap();
        assert_eq!(p.input_per_mtok, 15.0);
    }

    #[test]
    fn zero_is_a_rate_a_person_may_type() {
        let p = parse_rates(&RateFields {
            input: "0".into(),
            output: "0".into(),
            cache_read: "0".into(),
            cache_write: "0".into(),
        })
        .unwrap();
        assert_eq!(p.output_per_mtok, 0.0);
    }

    #[test]
    fn a_rate_survives_the_round_trip_through_its_text() {
        let original = ModelPrice {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
            cache_read_per_mtok: 0.3,
            cache_write_per_mtok: 3.75,
        };
        assert_eq!(parse_rates(&rate_fields(&original)).unwrap(), original);
    }

    #[test]
    fn an_empty_model_id_is_refused() {
        assert!(parse_model_id("   ").is_err());
        assert_eq!(
            parse_model_id(" claude-fable-5-1 ").unwrap(),
            "claude-fable-5-1"
        );
    }
}
