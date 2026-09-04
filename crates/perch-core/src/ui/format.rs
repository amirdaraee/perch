//! Human formatting. One place, shared by every shell.

pub fn human_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

pub fn human_cost(usd: f64) -> String {
    format!("${usd:.2}")
}

/// Deliberately no "days" bucket: a session blocked for 32 hours should read
/// as 32h, which is more alarming than 1d 8h. That alarm is the point.
pub fn human_elapsed(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else {
        format!("{}h", s / 3600)
    }
}

/// A missing timestamp is `0` on the wire. Formatting `now - 0` renders ~56 years,
/// so the caller's "absent" must become a dash, never a duration.
pub fn elapsed_or_dash(now_ms: i64, since_ms: i64) -> String {
    if since_ms <= 0 {
        "—".to_string()
    } else {
        human_elapsed(now_ms - since_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_compact() {
        assert_eq!(human_tokens(0), "0");
        assert_eq!(human_tokens(999), "999");
        assert_eq!(human_tokens(1_500), "1.5k");
        assert_eq!(human_tokens(24_221), "24.2k");
        assert_eq!(human_tokens(4_100_000), "4.1M");
    }

    #[test]
    fn cost_has_two_decimals() {
        assert_eq!(human_cost(0.0), "$0.00");
        assert_eq!(human_cost(8.204), "$8.20");
    }

    #[test]
    fn elapsed_buckets_without_days() {
        assert_eq!(human_elapsed(0), "0s");
        assert_eq!(human_elapsed(45_000), "45s");
        assert_eq!(human_elapsed(90_000), "1m");
        assert_eq!(human_elapsed(3_600_000), "1h");
        assert_eq!(human_elapsed(115_200_000), "32h");
        assert_eq!(
            human_elapsed(-5_000),
            "0s",
            "clock skew must not panic or go negative"
        );
    }

    #[test]
    fn absent_timestamp_is_a_dash_not_the_epoch() {
        assert_eq!(elapsed_or_dash(1_000_000, 0), "—");
        assert_eq!(elapsed_or_dash(1_000_000, -1), "—");
        assert_eq!(elapsed_or_dash(1_000_000, 940_000), "1m");
    }

    #[test]
    fn a_real_timestamp_formats_the_same_as_human_elapsed() {
        assert_eq!(
            elapsed_or_dash(10_000_000, 2_800_000),
            human_elapsed(7_200_000)
        );
    }
}
