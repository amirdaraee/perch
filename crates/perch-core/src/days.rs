//! Calendar days in the user's time zone. A day isn't always 24 hours: daylight
//! saving makes some 23 or 25, so a day's edges are worked out from its date
//! rather than by adding a fixed length.

use chrono::offset::LocalResult;
use chrono::{DateTime, Days, Duration, NaiveDate, TimeZone, Utc};

/// One calendar day and the instants it covers, `start_ms` inclusive and
/// `end_ms` exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Day {
    pub date: NaiveDate,
    pub start_ms: i64,
    pub end_ms: i64,
}

/// `days` consecutive days ending with the one containing `now_ms`, oldest first.
pub fn window<Tz: TimeZone>(tz: &Tz, days: usize, now_ms: i64) -> Vec<Day> {
    let now = DateTime::from_timestamp_millis(now_ms).unwrap_or_else(Utc::now);
    let today = now.with_timezone(tz).date_naive();
    (0..days as u64)
        .rev()
        .filter_map(|ago| {
            let date = today.checked_sub_days(Days::new(ago))?;
            let next = date.checked_add_days(Days::new(1))?;
            Some(Day {
                date,
                start_ms: start_of(tz, date),
                end_ms: start_of(tz, next),
            })
        })
        .collect()
}

/// The first instant of a date. Some places change their clocks at midnight:
/// when 00:00 happens twice the earlier one counts, and when it never happens
/// the day starts at the first time that does.
fn start_of<Tz: TimeZone>(tz: &Tz, date: NaiveDate) -> i64 {
    let midnight = date.and_hms_opt(0, 0, 0).unwrap_or_default();
    // Clock changes move in steps of 15 minutes or more, and never skip a day.
    for quarter in 0..96 {
        let local = midnight + Duration::minutes(quarter * 15);
        match tz.from_local_datetime(&local) {
            LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => return t.timestamp_millis(),
            LocalResult::None => {}
        }
    }
    midnight.and_utc().timestamp_millis()
}

#[cfg(test)]
pub(crate) mod test_zones {
    use chrono::offset::LocalResult;
    use chrono::{FixedOffset, NaiveDate, NaiveDateTime, Offset, TimeZone};

    /// Time zones with daylight saving for 2026, so these tests don't depend on
    /// the machine's own zone and need no time zone database.
    #[derive(Debug, Clone, Copy)]
    pub(crate) enum Zone {
        /// +1, or +2 from 29 March to 25 October. Clocks change at 01:00 UTC.
        Berlin,
        /// -5, or -4 from 8 March to 1 November. Clocks change at local midnight,
        /// so 8 March has no 00:00 and 1 November has two.
        Havana,
    }

    fn utc(y: i32, m: u32, d: u32, h: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, 0, 0)
            .unwrap()
    }

    fn hours(h: i32) -> FixedOffset {
        FixedOffset::east_opt(h * 3600).unwrap()
    }

    impl Zone {
        /// (standard, daylight, daylight starts, daylight ends), the last two in UTC.
        fn rules(self) -> (i32, i32, NaiveDateTime, NaiveDateTime) {
            match self {
                Zone::Berlin => (1, 2, utc(2026, 3, 29, 1), utc(2026, 10, 25, 1)),
                Zone::Havana => (-5, -4, utc(2026, 3, 8, 5), utc(2026, 11, 1, 5)),
            }
        }
    }

    impl TimeZone for Zone {
        type Offset = FixedOffset;

        fn from_offset(_: &FixedOffset) -> Self {
            // Nothing here turns an offset back into a zone.
            Zone::Berlin
        }

        fn offset_from_local_date(&self, date: &NaiveDate) -> LocalResult<FixedOffset> {
            self.offset_from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
        }

        fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> LocalResult<FixedOffset> {
            let (standard, daylight, _, _) = self.rules();
            let fits: Vec<FixedOffset> = [standard, daylight]
                .into_iter()
                .map(hours)
                .filter(|o| self.offset_from_utc_datetime(&(*local - *o)).fix() == *o)
                .collect();
            match fits[..] {
                [one] => LocalResult::Single(one),
                // Oldest instant first: the larger offset is the earlier one.
                [a, b] => LocalResult::Ambiguous(b, a),
                _ => LocalResult::None,
            }
        }

        fn offset_from_utc_date(&self, date: &NaiveDate) -> FixedOffset {
            self.offset_from_utc_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
        }

        fn offset_from_utc_datetime(&self, at: &NaiveDateTime) -> FixedOffset {
            let (standard, daylight, starts, ends) = self.rules();
            hours(if *at >= starts && *at < ends {
                daylight
            } else {
                standard
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_zones::Zone;
    use super::*;
    use chrono::NaiveDateTime;

    const HOUR_MS: i64 = 3_600_000;

    fn date(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    fn at(m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        date(m, d).and_hms_opt(h, min, 0).unwrap()
    }

    fn ms(tz: &Zone, local: NaiveDateTime) -> i64 {
        tz.from_local_datetime(&local).unwrap().timestamp_millis()
    }

    #[test]
    fn today_is_the_local_date_not_the_utc_one() {
        // 00:30 on the 15th in Berlin is still the 14th in UTC.
        let now = ms(&Zone::Berlin, at(1, 15, 0, 30));
        assert_eq!(
            window(&Zone::Berlin, 1, now)[0].date,
            date(1, 15),
            "after midnight belongs to the new local day"
        );
        // 23:30 on the 15th in Havana is already the 16th in UTC.
        let now = ms(&Zone::Havana, at(1, 15, 23, 30));
        assert_eq!(
            window(&Zone::Havana, 1, now)[0].date,
            date(1, 15),
            "late evening stays on the day it happened"
        );
    }

    #[test]
    fn the_window_is_oldest_first_and_has_no_gaps() {
        let now = ms(&Zone::Berlin, at(3, 30, 12, 0));
        let days = window(&Zone::Berlin, 5, now);
        assert_eq!(days.len(), 5);
        assert_eq!(days[0].date, date(3, 26));
        assert_eq!(days[4].date, date(3, 30));
        for pair in days.windows(2) {
            assert_eq!(pair[0].end_ms, pair[1].start_ms, "no instant falls between");
        }
    }

    #[test]
    fn daylight_saving_makes_a_day_shorter_or_longer() {
        let length = |tz: &Zone, m, d| {
            let now = ms(tz, at(m, d, 12, 0));
            let day = window(tz, 1, now)[0];
            assert_eq!(day.date, date(m, d));
            day.end_ms - day.start_ms
        };
        assert_eq!(length(&Zone::Berlin, 3, 28), 24 * HOUR_MS);
        assert_eq!(length(&Zone::Berlin, 3, 29), 23 * HOUR_MS);
        assert_eq!(length(&Zone::Berlin, 10, 25), 25 * HOUR_MS);
        assert_eq!(length(&Zone::Berlin, 10, 26), 24 * HOUR_MS);
    }

    #[test]
    fn a_day_with_no_midnight_starts_at_its_first_real_time() {
        let now = ms(&Zone::Havana, at(3, 8, 12, 0));
        let day = window(&Zone::Havana, 1, now)[0];
        assert_eq!(day.start_ms, ms(&Zone::Havana, at(3, 8, 1, 0)));
        assert_eq!(day.end_ms - day.start_ms, 23 * HOUR_MS);
    }

    #[test]
    fn a_day_with_two_midnights_starts_at_the_first() {
        let now = ms(&Zone::Havana, at(11, 1, 12, 0));
        let day = window(&Zone::Havana, 1, now)[0];
        assert_eq!(day.end_ms - day.start_ms, 25 * HOUR_MS);
    }

    #[test]
    fn no_days_asked_for_means_no_days() {
        assert!(window(&Utc, 0, 0).is_empty());
    }
}
