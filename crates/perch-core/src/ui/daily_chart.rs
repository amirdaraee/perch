//! The Usage view's daily chart: one bar per local day, stacked by model, in
//! whichever measure the person picks. Every value, tick and sentence is
//! worked out here; a shell places the numbers and draws.

use crate::query::DayUsage;
use crate::ui::format::{cost_label, human_cost, human_tokens, last_days_title, plural};
use chrono::NaiveDate;

/// Models past this rank share the "other" colour and series, so the palette
/// never runs out and stays easy to tell apart.
pub const NAMED_COLOURS: u8 = 7;
/// The colour shared by every model past the named ones.
pub const OTHER_COLOUR: u8 = NAMED_COLOURS;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct UsageChart {
    /// "Last 14 days".
    pub title: String,
    pub days: Vec<ChartDay>,
    /// The stack's layers, bottom first. A segment's `series` indexes this.
    pub series: Vec<ChartSeries>,
    /// What the chart can show, the default first.
    pub measures: Vec<ChartMeasure>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ChartDay {
    pub index: u32,
    /// "Nov 12" or "Today" on the days the axis labels; `None` between them.
    pub axis_label: Option<String>,
    /// Today is still in progress, so it is drawn as unfinished.
    pub is_today: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ChartSeries {
    /// A model's name, or "Other models".
    pub name: String,
    /// Into the shell's palette; the same model has the same colour everywhere.
    pub colour: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum MeasureKind {
    Spend,
    OutputTokens,
    AllTokens,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ChartMeasure {
    pub kind: MeasureKind,
    /// "Spend", for the picker.
    pub title: String,
    /// "Estimated cost per day, by model".
    pub caption: String,
    /// The top of the value axis: the last tick, at or above the tallest day.
    pub y_max: f64,
    pub ticks: Vec<ChartTick>,
    /// Non-empty layers only, so a quiet day has none.
    pub segments: Vec<ChartSegment>,
    /// One per day, in day order.
    pub tooltips: Vec<DayTooltip>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ChartTick {
    pub value: f64,
    /// "$20" or "1.5M".
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ChartSegment {
    pub day: u32,
    pub series: u32,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DayTooltip {
    /// "Tue, Nov 12", or "Today so far".
    pub title: String,
    /// "$12.40", "1.2M tokens" or "No activity".
    pub total: String,
    /// Largest first.
    pub lines: Vec<TooltipLine>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TooltipLine {
    pub series: u32,
    pub label: String,
    pub value: String,
}

/// `colour_of` gives a model's colour from the all-time ranking, so the chart
/// and the by-model list agree. `show_cost` false leaves Spend out entirely.
pub fn build(
    days: &[DayUsage],
    today: NaiveDate,
    show_cost: bool,
    colour_of: &dyn Fn(&str) -> u8,
) -> UsageChart {
    let series = series_for(days, colour_of);
    let series_of = |model: &str| {
        let c = colour_of(model).min(OTHER_COLOUR);
        series.iter().position(|s| s.colour == c).unwrap_or(0) as u32
    };
    let total_tokens: u64 = days.iter().map(|d| d.usage.total_tokens()).sum();
    let unpriced: u64 = days.iter().map(|d| d.unpriced_tokens).sum();
    let nothing_priced = total_tokens > 0 && unpriced >= total_tokens;

    let mut measures = Vec::new();
    if show_cost && !nothing_priced {
        let caption = if unpriced > 0 {
            "Estimated cost per day, by model. Models with no price aren't counted."
        } else {
            "Estimated cost per day, by model"
        };
        measures.push(measure(
            MeasureKind::Spend,
            "Spend",
            caption.to_string(),
            days,
            today,
            &series_of,
        ));
    }
    let output_caption = if show_cost && nothing_priced {
        "Output tokens per day, by model, because these models have no price"
    } else {
        "Output tokens per day, by model"
    };
    measures.push(measure(
        MeasureKind::OutputTokens,
        "Output tokens",
        output_caption.to_string(),
        days,
        today,
        &series_of,
    ));
    let cache_read: u64 = days.iter().map(|d| d.usage.cache_read).sum();
    let all_caption = if total_tokens > 0 && cache_read * 2 >= total_tokens {
        format!(
            "All tokens per day, by model. Cache reads are {}% of this.",
            cache_read * 100 / total_tokens
        )
    } else {
        "All tokens per day, by model".to_string()
    };
    measures.push(measure(
        MeasureKind::AllTokens,
        "All tokens",
        all_caption,
        days,
        today,
        &series_of,
    ));

    UsageChart {
        title: last_days_title(days.len() as u32),
        days: axis_days(days, today),
        series,
        measures,
    }
}

/// One series per named colour present in the window, then "Other models"
/// for the rest. Ordered by colour, so the biggest models sit at the bottom.
fn series_for(days: &[DayUsage], colour_of: &dyn Fn(&str) -> u8) -> Vec<ChartSeries> {
    let mut present: Vec<(u8, String)> = Vec::new();
    for d in days {
        for m in &d.by_model {
            let colour = colour_of(&m.model).min(OTHER_COLOUR);
            let name = if colour == OTHER_COLOUR {
                "Other models".to_string()
            } else {
                m.model.clone()
            };
            if !present.iter().any(|(c, _)| *c == colour) {
                present.push((colour, name));
            }
        }
    }
    present.sort_by_key(|(c, _)| *c);
    present
        .into_iter()
        .map(|(colour, name)| ChartSeries { name, colour })
        .collect()
}

fn axis_days(days: &[DayUsage], today: NaiveDate) -> Vec<ChartDay> {
    let n = days.len();
    // About seven labels whatever the span, always including today.
    let stride = n.div_ceil(7).max(1);
    days.iter()
        .enumerate()
        .map(|(i, d)| ChartDay {
            index: i as u32,
            axis_label: (n - 1 - i).is_multiple_of(stride).then(|| {
                if d.date == today {
                    "Today".to_string()
                } else {
                    crate::ui::heatmap::date_label(d.date, today)
                }
            }),
            is_today: d.date == today,
        })
        .collect()
}

fn measure(
    kind: MeasureKind,
    title: &str,
    caption: String,
    days: &[DayUsage],
    today: NaiveDate,
    series_of: &dyn Fn(&str) -> u32,
) -> ChartMeasure {
    let value = |m: &crate::query::ModelDay| -> f64 {
        match kind {
            MeasureKind::Spend => m.cost_usd.unwrap_or(0.0),
            MeasureKind::OutputTokens => m.usage.output as f64,
            MeasureKind::AllTokens => m.usage.total_tokens() as f64,
        }
    };
    let mut segments = Vec::new();
    let mut tooltips = Vec::new();
    let mut tallest: f64 = 0.0;
    for (i, d) in days.iter().enumerate() {
        // Several models can share the "other" series, so sum per series first.
        let mut per_series: Vec<(u32, f64)> = Vec::new();
        for m in &d.by_model {
            let s = series_of(&m.model);
            match per_series.iter_mut().find(|(x, _)| *x == s) {
                Some((_, v)) => *v += value(m),
                None => per_series.push((s, value(m))),
            }
        }
        per_series.sort_by_key(|(s, _)| *s);
        let mut top = 0.0;
        for (s, v) in &per_series {
            if *v > 0.0 {
                segments.push(ChartSegment {
                    day: i as u32,
                    series: *s,
                    start: top,
                    end: top + v,
                });
                top += v;
            }
        }
        tallest = tallest.max(top);
        tooltips.push(tooltip(kind, d, today, series_of, &value));
    }
    let (y_max, ticks) = ticks(kind, tallest);
    ChartMeasure {
        kind,
        title: title.to_string(),
        caption,
        y_max,
        ticks,
        segments,
        tooltips,
    }
}

fn tooltip(
    kind: MeasureKind,
    d: &DayUsage,
    today: NaiveDate,
    series_of: &dyn Fn(&str) -> u32,
    value: &dyn Fn(&crate::query::ModelDay) -> f64,
) -> DayTooltip {
    let title = if d.date == today {
        "Today so far".to_string()
    } else {
        format!(
            "{}, {}",
            d.date.format("%a"),
            crate::ui::heatmap::date_label(d.date, today)
        )
    };
    if d.by_model.is_empty() {
        return DayTooltip {
            title,
            total: "No activity".into(),
            lines: Vec::new(),
        };
    }
    let total = match kind {
        MeasureKind::Spend => cost_label(d.cost_usd, d.unpriced_tokens, d.usage.total_tokens()),
        MeasureKind::OutputTokens => tokens_total(d.usage.output),
        MeasureKind::AllTokens => tokens_total(d.usage.total_tokens()),
    };
    let mut models: Vec<&crate::query::ModelDay> = d.by_model.iter().collect();
    models.sort_by(|a, b| {
        value(b)
            .total_cmp(&value(a))
            .then_with(|| a.model.cmp(&b.model))
    });
    let lines = models
        .into_iter()
        .map(|m| TooltipLine {
            series: series_of(&m.model),
            label: m.model.clone(),
            value: match kind {
                MeasureKind::Spend => m
                    .cost_usd
                    .map(human_cost)
                    .unwrap_or_else(|| "no price".into()),
                MeasureKind::OutputTokens => human_tokens(m.usage.output),
                MeasureKind::AllTokens => human_tokens(m.usage.total_tokens()),
            },
        })
        .collect();
    DayTooltip {
        title,
        total,
        lines,
    }
}

fn tokens_total(n: u64) -> String {
    if n < 1_000 {
        plural(n as i64, "token", "tokens")
    } else {
        format!("{} tokens", human_tokens(n))
    }
}

/// Four or five evenly spaced ticks on a round step, from zero to at or
/// above `tallest`.
fn ticks(kind: MeasureKind, tallest: f64) -> (f64, Vec<ChartTick>) {
    let floor = match kind {
        MeasureKind::Spend => 1.0,
        _ => 1_000.0,
    };
    let step = nice_step(tallest.max(floor) / 4.0);
    let count = (tallest / step).ceil().max(1.0) as usize;
    let ticks: Vec<ChartTick> = (0..=count)
        .map(|i| {
            let value = step * i as f64;
            ChartTick {
                value,
                label: match kind {
                    MeasureKind::Spend => axis_cost(value, step),
                    _ => axis_tokens(value),
                },
            }
        })
        .collect();
    (step * count as f64, ticks)
}

/// 1, 2, 2.5 or 5 times a power of ten, at or above `raw`.
fn nice_step(raw: f64) -> f64 {
    let magnitude = 10f64.powf(raw.log10().floor());
    let fraction = raw / magnitude;
    let nice = if fraction <= 1.0 {
        1.0
    } else if fraction <= 2.0 {
        2.0
    } else if fraction <= 2.5 {
        2.5
    } else if fraction <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * magnitude
}

/// "$20", or "$2.50" when the step needs cents.
fn axis_cost(value: f64, step: f64) -> String {
    if step.fract() == 0.0 {
        format!("${value:.0}")
    } else {
        format!("${value:.2}")
    }
}

/// "0", "500", "250k", "1.5M": short, with no trailing ".0".
fn axis_tokens(value: f64) -> String {
    let (n, unit) = if value >= 1_000_000.0 {
        (value / 1_000_000.0, "M")
    } else if value >= 1_000.0 {
        (value / 1_000.0, "k")
    } else {
        (value, "")
    };
    let s = format!("{n:.1}");
    let s = s.strip_suffix(".0").unwrap_or(&s);
    format!("{s}{unit}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TurnUsage;
    use crate::query::ModelDay;

    fn date(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
    }

    fn model(name: &str, output: u64, cache_read: u64, cost: Option<f64>) -> ModelDay {
        ModelDay {
            model: name.into(),
            usage: TurnUsage {
                output,
                cache_read,
                ..TurnUsage::default()
            },
            cost_usd: cost,
        }
    }

    fn day(d: u32, models: Vec<ModelDay>) -> DayUsage {
        let mut usage = TurnUsage::default();
        let mut cost = 0.0;
        let mut unpriced = 0;
        for m in &models {
            usage = usage.plus(&m.usage);
            match m.cost_usd {
                Some(c) => cost += c,
                None => unpriced += m.usage.total_tokens(),
            }
        }
        DayUsage {
            date: date(d),
            responses: models.len() as u64,
            sessions: 1,
            usage,
            cost_usd: cost,
            unpriced_tokens: unpriced,
            by_model: models,
        }
    }

    fn rank(model: &str) -> u8 {
        match model {
            "big" => 0,
            "small" => 1,
            "free" => 2,
            _ => 9,
        }
    }

    fn quiet_window(n: u32) -> Vec<DayUsage> {
        (1..=n).map(|d| day(d, Vec::new())).collect()
    }

    #[test]
    fn spend_is_the_default_stacked_by_model_bottom_up() {
        let days = vec![
            day(
                25,
                vec![
                    model("small", 10, 0, Some(1.0)),
                    model("big", 10, 0, Some(3.0)),
                ],
            ),
            day(26, Vec::new()),
        ];
        let c = build(&days, date(26), true, &rank);
        assert_eq!(c.measures[0].kind, MeasureKind::Spend);
        assert_eq!(c.series[0].name, "big");
        assert_eq!(c.series[1].name, "small");
        let s = &c.measures[0].segments;
        assert_eq!(s.len(), 2, "the quiet day has no segments");
        assert_eq!((s[0].series, s[0].start, s[0].end), (0, 0.0, 3.0));
        assert_eq!((s[1].series, s[1].start, s[1].end), (1, 3.0, 4.0));
        let t = &c.measures[0].tooltips;
        assert_eq!(t[0].total, "$4.00");
        assert_eq!(t[0].lines[0].label, "big");
        assert_eq!(t[0].lines[0].value, "$3.00");
        assert_eq!(t[1].title, "Today so far");
        assert_eq!(t[1].total, "No activity");
        assert!(c.days[1].is_today);
    }

    #[test]
    fn hidden_costs_leave_spend_out_and_nothing_priced_says_why() {
        let days = vec![day(26, vec![model("free", 10, 0, None)])];
        let hidden = build(&days, date(26), false, &rank);
        assert_eq!(hidden.measures[0].kind, MeasureKind::OutputTokens);
        assert_eq!(
            hidden.measures[0].caption,
            "Output tokens per day, by model"
        );

        let unpriced = build(&days, date(26), true, &rank);
        assert_eq!(unpriced.measures[0].kind, MeasureKind::OutputTokens);
        assert_eq!(
            unpriced.measures[0].caption,
            "Output tokens per day, by model, because these models have no price"
        );
    }

    #[test]
    fn a_partly_priced_window_keeps_spend_and_names_what_is_missing() {
        let days = vec![day(
            26,
            vec![model("big", 10, 0, Some(2.0)), model("free", 10, 0, None)],
        )];
        let c = build(&days, date(26), true, &rank);
        let spend = &c.measures[0];
        assert!(spend
            .caption
            .ends_with("Models with no price aren't counted."));
        assert_eq!(spend.tooltips[0].total, "at least $2.00");
        let free = spend.tooltips[0]
            .lines
            .iter()
            .find(|l| l.label == "free")
            .unwrap();
        assert_eq!(free.value, "no price");
    }

    #[test]
    fn all_tokens_says_how_much_is_cache() {
        let days = vec![day(26, vec![model("big", 30, 970, Some(1.0))])];
        let c = build(&days, date(26), true, &rank);
        let all = c
            .measures
            .iter()
            .find(|m| m.kind == MeasureKind::AllTokens)
            .unwrap();
        assert_eq!(
            all.caption,
            "All tokens per day, by model. Cache reads are 97% of this."
        );
    }

    #[test]
    fn models_past_the_palette_share_one_other_series() {
        let days = vec![day(
            26,
            vec![
                model("big", 10, 0, Some(1.0)),
                model("x", 10, 0, Some(1.0)),
                model("y", 5, 0, Some(1.0)),
            ],
        )];
        let c = build(&days, date(26), true, &rank);
        assert_eq!(c.series.len(), 2);
        assert_eq!(c.series[1].name, "Other models");
        assert_eq!(c.series[1].colour, OTHER_COLOUR);
        let segs = &c.measures[0].segments;
        assert_eq!(segs.len(), 2, "both others stack as one layer");
        assert_eq!(segs[1].end - segs[1].start, 2.0);
    }

    #[test]
    fn the_axis_labels_about_seven_days_and_always_today() {
        for n in [1u32, 7, 14, 30, 90] {
            let days: Vec<DayUsage> = (0..n)
                .map(|i| DayUsage {
                    date: date(26) - chrono::Days::new(u64::from(n - 1 - i)),
                    ..day(26, Vec::new())
                })
                .collect();
            let c = build(&days, date(26), true, &rank);
            let labelled: Vec<&str> = c
                .days
                .iter()
                .filter_map(|d| d.axis_label.as_deref())
                .collect();
            assert!(labelled.len() <= 8, "{n} days: {labelled:?}");
            assert_eq!(*labelled.last().unwrap(), "Today", "{n} days");
            assert_eq!(c.days.len(), n as usize);
            assert_eq!(c.title, format!("Last {n} days"));
        }
    }

    #[test]
    fn ticks_are_round_and_reach_the_tallest_day() {
        let (top, t) = ticks(MeasureKind::Spend, 37.2);
        assert_eq!(top, 40.0);
        let labels: Vec<&str> = t.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, ["$0", "$10", "$20", "$30", "$40"]);

        let (top, t) = ticks(MeasureKind::Spend, 0.9);
        assert_eq!(top, 1.0);
        assert_eq!(t[1].label, "$0.25");

        let (top, t) = ticks(MeasureKind::AllTokens, 3_400_000.0);
        assert_eq!(top, 4_000_000.0);
        assert_eq!(t.last().unwrap().label, "4M");
        assert_eq!(t[1].label, "1M");

        let (_, t) = ticks(MeasureKind::OutputTokens, 0.0);
        assert_eq!(t[0].label, "0", "an empty chart still has an axis");
        assert!(t.len() >= 2);
        assert_eq!(axis_tokens(1_500_000.0), "1.5M");
        assert_eq!(axis_tokens(250_000.0), "250k");
    }

    #[test]
    fn a_quiet_window_still_draws_every_day() {
        let c = build(&quiet_window(14), date(14), true, &rank);
        assert!(c.series.is_empty());
        assert_eq!(c.days.len(), 14);
        assert!(c.measures.iter().all(|m| m.segments.is_empty()));
        assert!(c.measures[0]
            .tooltips
            .iter()
            .all(|t| t.total == "No activity"));
    }
}
