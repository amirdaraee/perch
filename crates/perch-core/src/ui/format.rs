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

/// "1 minute", never "1 minutes". One helper, in the one module every shell
/// already formats through, because a second copy is how two spellings of the
/// same duration appear -- and this app has shipped "1 seconds" twice. Every
/// caption that interpolates a count goes through here: the settings schema's
/// stepper captions, the notification labels, the project row summaries.
pub fn plural(n: i64, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
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

/// A file size as a person reads it. Decimal units, because that is what
/// Finder shows for the same file and a settings pane that disagrees with
/// Finder about the size of one file on disk is just wrong twice.
pub fn human_bytes(n: u64) -> String {
    const KB: f64 = 1_000.0;
    let n = n as f64;
    if n < KB {
        format!("{n:.0} bytes")
    } else if n < KB * KB {
        format!("{:.0} KB", n / KB)
    } else if n < KB * KB * KB {
        format!("{:.1} MB", n / (KB * KB))
    } else {
        format!("{:.1} GB", n / (KB * KB * KB))
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
    fn bytes_read_the_way_finder_reads_them() {
        assert_eq!(human_bytes(0), "0 bytes");
        assert_eq!(human_bytes(512), "512 bytes");
        assert_eq!(human_bytes(2_048), "2 KB");
        assert_eq!(human_bytes(12_400_000), "12.4 MB");
        assert_eq!(human_bytes(3_200_000_000), "3.2 GB");
    }

    #[test]
    fn cost_has_two_decimals() {
        assert_eq!(human_cost(0.0), "$0.00");
        assert_eq!(human_cost(8.204), "$8.20");
        assert_eq!(human_cost(38.0), "$38.00");
    }

    #[test]
    fn one_is_singular() {
        // "Check every 1 seconds" has reached a shipped build of this app
        // twice. This is the whole reason the helper exists.
        assert_eq!(plural(1, "minute", "minutes"), "1 minute");
        assert_eq!(plural(0, "minute", "minutes"), "0 minutes");
        assert_eq!(plural(2, "session", "sessions"), "2 sessions");
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
