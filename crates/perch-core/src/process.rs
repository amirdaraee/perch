//! What a session's process and everything it started are using right now.
//!
//! Read from a table of pids and numbers. The table never holds a command line,
//! so nothing a session is running can reach Perch this way.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProcessStats {
    /// Summed over the process and its descendants, so it can pass 100.
    pub cpu_percent: f64,
    /// Resident memory, summed the same way. Pages shared between processes
    /// are counted once for each, so this can run a little high.
    pub memory_bytes: u64,
    /// How many descendants those sums include.
    pub child_processes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Entry {
    pid: i32,
    ppid: i32,
    cpu_percent: f64,
    rss_kb: u64,
}

/// One `pid ppid %cpu rss-in-kb` per line. A line that doesn't fit is skipped.
fn parse_table(table: &str) -> Vec<Entry> {
    table
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let ppid = fields.next()?.parse().ok()?;
            let cpu_percent = fields.next()?.replace(',', ".").parse().ok()?;
            let rss_kb = fields.next()?.parse().ok()?;
            Some(Entry {
                pid,
                ppid,
                cpu_percent,
                rss_kb,
            })
        })
        .collect()
}

/// The stats for `root` and its descendants, or `None` if `root` isn't in the
/// table.
pub fn stats_from_table(table: &str, root: i32) -> Option<ProcessStats> {
    let entries = parse_table(table);
    let by_pid: HashMap<i32, &Entry> = entries.iter().map(|e| (e.pid, e)).collect();
    let mut children: HashMap<i32, Vec<i32>> = HashMap::new();
    for e in &entries {
        children.entry(e.ppid).or_default().push(e.pid);
    }

    let root_entry = by_pid.get(&root)?;
    let mut stats = ProcessStats {
        cpu_percent: root_entry.cpu_percent,
        memory_bytes: root_entry.rss_kb * 1024,
        child_processes: 0,
    };
    let mut pending = children.get(&root).cloned().unwrap_or_default();
    while let Some(pid) = pending.pop() {
        // Pid 0 lists itself as its own parent, which must not loop if it is
        // ever asked about.
        if pid == root {
            continue;
        }
        if let Some(e) = by_pid.get(&pid) {
            stats.cpu_percent += e.cpu_percent;
            stats.memory_bytes += e.rss_kb * 1024;
            stats.child_processes += 1;
        }
        if let Some(more) = children.get(&pid) {
            pending.extend(more);
        }
    }
    Some(stats)
}

/// Read the stats for `pid` now. `None` when they can't be read, which a caller
/// must show as unknown rather than as zero.
pub fn read_stats(pid: i32) -> Option<ProcessStats> {
    stats_from_table(&crate::platform::process_table()?, pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "    1     0   0.1   1000
  100     1  10.0   2000
  101   100   5.5   3000
  102   101   0.5   4000
  200     1  90.0   9000
";

    #[test]
    fn sums_a_process_and_all_its_descendants_and_nothing_else() {
        let s = stats_from_table(TABLE, 100).unwrap();
        assert_eq!(s.cpu_percent, 16.0);
        assert_eq!(s.memory_bytes, 9000 * 1024);
        assert_eq!(s.child_processes, 2);
    }

    #[test]
    fn a_process_with_no_children_is_just_itself() {
        let s = stats_from_table(TABLE, 200).unwrap();
        assert_eq!(s.cpu_percent, 90.0);
        assert_eq!(s.child_processes, 0);
    }

    #[test]
    fn an_unknown_process_has_no_stats_rather_than_zero() {
        assert_eq!(stats_from_table(TABLE, 999), None);
        assert_eq!(stats_from_table("", 100), None);
    }

    #[test]
    fn a_decimal_comma_and_a_malformed_line_are_both_survivable() {
        let s = stats_from_table("not a row\n100 1 2,5 10\n101 100\n", 100).unwrap();
        assert_eq!(s.cpu_percent, 2.5);
        assert_eq!(s.child_processes, 0);
    }

    #[test]
    fn a_process_listed_as_its_own_parent_does_not_loop() {
        let s = stats_from_table("0 0 1.0 10\n5 0 2.0 10\n", 0).unwrap();
        assert_eq!(s.cpu_percent, 3.0);
        assert_eq!(s.child_processes, 1);
    }
}
