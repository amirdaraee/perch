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
