//! Estimated dollar cost. Tokens are truth; dollars are an estimate.

use crate::db::Db;
use crate::model::TurnUsage;
use anyhow::Result;
use rusqlite::params;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPrice {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_read_per_mtok: f64,
    pub cache_write_per_mtok: f64,
}

/// USD per million tokens. These are estimates for display only and are
/// user-editable at runtime; seeding never overwrites an existing row.
const DEFAULTS: &[(&str, ModelPrice)] = &[
    (
        "claude-fable-5",
        ModelPrice {
            input_per_mtok: 15.0,
            output_per_mtok: 75.0,
            cache_read_per_mtok: 1.5,
            cache_write_per_mtok: 18.75,
        },
    ),
    (
        "claude-opus-5",
        ModelPrice {
            input_per_mtok: 15.0,
            output_per_mtok: 75.0,
            cache_read_per_mtok: 1.5,
            cache_write_per_mtok: 18.75,
        },
    ),
    (
        "claude-sonnet-5",
        ModelPrice {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
            cache_read_per_mtok: 0.3,
            cache_write_per_mtok: 3.75,
        },
    ),
    (
        "claude-haiku-4-5-20251001",
        ModelPrice {
            input_per_mtok: 1.0,
            output_per_mtok: 5.0,
            cache_read_per_mtok: 0.1,
            cache_write_per_mtok: 1.25,
        },
    ),
];

pub fn seed_default_prices(db: &Db) -> Result<()> {
    for (model, p) in DEFAULTS {
        db.conn().execute(
            "INSERT INTO prices (model, input_per_mtok, output_per_mtok,
                 cache_read_per_mtok, cache_write_per_mtok)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(model) DO NOTHING",
            params![
                model,
                p.input_per_mtok,
                p.output_per_mtok,
                p.cache_read_per_mtok,
                p.cache_write_per_mtok
            ],
        )?;
    }
    Ok(())
}

/// One row of the price table as the settings window edits it.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelPriceRow {
    pub model: String,
    pub price: ModelPrice,
    /// True when this model ships with Perch *and* still holds the rate Perch
    /// shipped, so a shell can offer a reset only where something drifted. A
    /// model the user added is never a default: there is nothing to restore it
    /// to, and a reset removes it rather than repricing it.
    pub is_default: bool,
}

/// The rate Perch ships for a model, or `None` for one it does not know.
pub fn default_price(model: &str) -> Option<ModelPrice> {
    DEFAULTS.iter().find(|(m, _)| *m == model).map(|(_, p)| *p)
}

/// Every priced model, alphabetically, each marked against the built-in table.
pub fn all_prices(db: &Db) -> Result<Vec<ModelPriceRow>> {
    let mut stmt = db.conn().prepare(
        "SELECT model, input_per_mtok, output_per_mtok, cache_read_per_mtok, cache_write_per_mtok
         FROM prices ORDER BY model",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            ModelPrice {
                input_per_mtok: r.get(1)?,
                output_per_mtok: r.get(2)?,
                cache_read_per_mtok: r.get(3)?,
                cache_write_per_mtok: r.get(4)?,
            },
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (model, price) = row?;
        // Exact comparison is right here: SQLite stores an f64 as an IEEE-754
        // double and hands back the same bits, so an untouched seeded row
        // still equals the constant it was seeded from.
        let is_default = default_price(&model) == Some(price);
        out.push(ModelPriceRow {
            model,
            price,
            is_default,
        });
    }
    Ok(out)
}

/// Store a rate, for a built-in model or one the user added.
pub fn set_price(db: &Db, model: &str, price: ModelPrice) -> Result<()> {
    db.conn().execute(
        "INSERT INTO prices (model, input_per_mtok, output_per_mtok,
             cache_read_per_mtok, cache_write_per_mtok)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(model) DO UPDATE SET
             input_per_mtok = excluded.input_per_mtok,
             output_per_mtok = excluded.output_per_mtok,
             cache_read_per_mtok = excluded.cache_read_per_mtok,
             cache_write_per_mtok = excluded.cache_write_per_mtok",
        params![
            model,
            price.input_per_mtok,
            price.output_per_mtok,
            price.cache_read_per_mtok,
            price.cache_write_per_mtok
        ],
    )?;
    Ok(())
}

/// Forget a model's rate. Its turns keep counting their tokens and contribute
/// no cost, per spec §8 — removing a price is how a user says "I do not know
/// what this costs", which is a different claim from "it costs nothing".
/// Removing a row that is not there is not an error: the end state is the one
/// the caller asked for.
pub fn remove_price(db: &Db, model: &str) -> Result<()> {
    db.conn()
        .execute("DELETE FROM prices WHERE model = ?1", params![model])?;
    Ok(())
}

/// Restore the built-in table exactly: every edit undone and every model the
/// user added gone. Anything less would leave a table that is neither what
/// Perch ships nor what the user chose. One transaction, so a failure leaves
/// the old table intact rather than a half-reset one.
pub fn reset_prices_to_defaults(db: &Db) -> Result<()> {
    let tx = db.conn().unchecked_transaction()?;
    tx.execute("DELETE FROM prices", [])?;
    seed_default_prices(db)?;
    tx.commit()?;
    Ok(())
}

pub fn price_for(db: &Db, model: &str) -> Result<Option<ModelPrice>> {
    let found = db.conn().query_row(
        "SELECT input_per_mtok, output_per_mtok, cache_read_per_mtok, cache_write_per_mtok
         FROM prices WHERE model = ?1",
        params![model],
        |r| {
            Ok(ModelPrice {
                input_per_mtok: r.get(0)?,
                output_per_mtok: r.get(1)?,
                cache_read_per_mtok: r.get(2)?,
                cache_write_per_mtok: r.get(3)?,
            })
        },
    );
    match found {
        Ok(p) => Ok(Some(p)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

const MTOK: f64 = 1_000_000.0;

/// `thinking` is a subset of `output` and is deliberately not priced again.
pub fn cost_usd(price: &ModelPrice, usage: &TurnUsage) -> f64 {
    (usage.input as f64 / MTOK) * price.input_per_mtok
        + (usage.output as f64 / MTOK) * price.output_per_mtok
        + (usage.cache_read as f64 / MTOK) * price.cache_read_per_mtok
        + (usage.cache_write_total() as f64 / MTOK) * price.cache_write_per_mtok
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    #[test]
    fn cost_prices_each_class_separately() {
        let p = ModelPrice {
            input_per_mtok: 10.0,
            output_per_mtok: 50.0,
            cache_read_per_mtok: 1.0,
            cache_write_per_mtok: 12.5,
        };
        let u = TurnUsage {
            input: 2_000_000,
            output: 1_000_000,
            cache_read: 400_000,
            cache_write_5m: 60_000,
            cache_write_1h: 40_000,
            thinking: 900_000,
        };
        // Each class has a distinct token count, so a misassigned rate (e.g. swapping
        // input and cache_read) would change the total: 20 + 50 + 0.4 + 1.25 = 71.65.
        // cache_write_5m/1h are uneven (60k/40k) to prove cache_write_total() sums both
        // buckets rather than reading just one. thinking (900_000) must not add anything.
        assert!((cost_usd(&p, &u) - 71.65).abs() < 1e-9);
    }

    #[test]
    fn zero_usage_costs_nothing() {
        let p = ModelPrice {
            input_per_mtok: 10.0,
            output_per_mtok: 50.0,
            cache_read_per_mtok: 1.0,
            cache_write_per_mtok: 12.5,
        };
        assert_eq!(cost_usd(&p, &TurnUsage::default()), 0.0);
    }

    #[test]
    fn seeded_prices_are_readable() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let p = price_for(&db, "claude-fable-5").unwrap();
        assert!(p.is_some(), "the default model must be seeded");
    }

    #[test]
    fn unknown_model_has_no_price_and_is_not_an_error() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        assert!(price_for(&db, "some-future-model").unwrap().is_none());
    }

    #[test]
    fn seeding_twice_does_not_duplicate_or_fail() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        seed_default_prices(&db).unwrap();
        let n: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM prices WHERE model = 'claude-fable-5'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    fn seeded_db() -> Db {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        db
    }

    /// The built-in rate for a model, for comparing against a reset.
    fn builtin(model: &str) -> ModelPrice {
        default_price(model).expect("a built-in model")
    }

    fn find<'a>(rows: &'a [ModelPriceRow], model: &str) -> &'a ModelPriceRow {
        rows.iter()
            .find(|r| r.model == model)
            .unwrap_or_else(|| panic!("{model} is not in the table"))
    }

    impl ModelPrice {
        /// A fixture: all four rates equal. Not production code — no real
        /// model prices its cache reads at its input rate.
        fn flat(rate: f64) -> Self {
            ModelPrice {
                input_per_mtok: rate,
                output_per_mtok: rate,
                cache_read_per_mtok: rate,
                cache_write_per_mtok: rate,
            }
        }
    }

    #[test]
    fn prices_come_back_with_their_defaults_marked() {
        let db = seeded_db();
        let rows = all_prices(&db).unwrap();
        assert_eq!(rows.len(), DEFAULTS.len(), "every built-in model is listed");
        assert!(
            rows.iter().all(|r| r.is_default),
            "an untouched table is all defaults"
        );
    }

    #[test]
    fn an_edited_price_is_no_longer_marked_default() {
        let db = seeded_db();
        set_price(
            &db,
            "claude-opus-5",
            ModelPrice {
                input_per_mtok: 99.0,
                ..builtin("claude-opus-5")
            },
        )
        .unwrap();
        let rows = all_prices(&db).unwrap();
        let row = find(&rows, "claude-opus-5");
        assert!(
            !row.is_default,
            "the reset affordance appears only once a value drifted"
        );
        assert_eq!(row.price.input_per_mtok, 99.0);
        assert!(
            find(&rows, "claude-sonnet-5").is_default,
            "editing one model says nothing about another"
        );
    }

    #[test]
    fn a_model_perch_does_not_ship_is_never_a_default() {
        let db = seeded_db();
        set_price(&db, "my-local-model", ModelPrice::flat(1.0)).unwrap();
        assert!(!find(&all_prices(&db).unwrap(), "my-local-model").is_default);
    }

    #[test]
    fn reset_restores_the_built_in_table_and_drops_added_models() {
        let db = seeded_db();
        set_price(
            &db,
            "claude-opus-5",
            ModelPrice {
                input_per_mtok: 99.0,
                ..builtin("claude-opus-5")
            },
        )
        .unwrap();
        set_price(&db, "my-local-model", ModelPrice::flat(1.0)).unwrap();

        reset_prices_to_defaults(&db).unwrap();

        let rows = all_prices(&db).unwrap();
        assert_eq!(find(&rows, "claude-opus-5").price, builtin("claude-opus-5"));
        assert!(
            rows.iter().all(|r| r.model != "my-local-model"),
            "reset means the built-in table, not the built-in table plus leftovers"
        );
        assert!(rows.iter().all(|r| r.is_default));
    }

    #[test]
    fn a_removed_model_still_counts_its_tokens_and_costs_nothing() {
        // The documented rule from spec §8, which must survive an editable table.
        let db = seeded_db();
        db.conn()
            .execute_batch(
                "INSERT INTO projects (id, slug, real_path) VALUES (1, 'p', '/p');
                 INSERT INTO sessions (id, project_id, file_path) VALUES ('s1', 1, '/p/s1.jsonl');
                 INSERT INTO turns (session_id, ts, model, input, output)
                     VALUES ('s1', 1, 'claude-opus-5', 1000, 2000);",
            )
            .unwrap();

        let (_, priced) = crate::query::usage_since(&db, 0).unwrap();
        assert!(priced > 0.0, "the model is priced before it is removed");

        remove_price(&db, "claude-opus-5").unwrap();

        let (usage, cost) = crate::query::usage_since(&db, 0).unwrap();
        assert_eq!(usage.input, 1000, "tokens are truth");
        assert_eq!(usage.output, 2000);
        assert_eq!(cost, 0.0, "dollars are an estimate, and there is none");
    }

    #[test]
    fn removing_a_model_that_is_not_there_is_not_an_error() {
        // A shell may race two removals of the same row; neither is a failure.
        let db = seeded_db();
        remove_price(&db, "never-existed").unwrap();
    }

    #[test]
    fn a_user_edited_price_survives_reseeding() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        db.conn()
            .execute(
                "UPDATE prices SET input_per_mtok = 99.0 WHERE model = 'claude-fable-5'",
                [],
            )
            .unwrap();
        seed_default_prices(&db).unwrap();
        assert_eq!(
            price_for(&db, "claude-fable-5")
                .unwrap()
                .unwrap()
                .input_per_mtok,
            99.0
        );
    }
}
