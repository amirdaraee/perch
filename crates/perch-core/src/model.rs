//! Shared value types.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    /// Subset of `output`. Reported for insight, never added to totals.
    pub thinking: u64,
}

impl TurnUsage {
    pub fn plus(&self, other: &TurnUsage) -> TurnUsage {
        TurnUsage {
            input: self.input + other.input,
            output: self.output + other.output,
            cache_read: self.cache_read + other.cache_read,
            cache_write_5m: self.cache_write_5m + other.cache_write_5m,
            cache_write_1h: self.cache_write_1h + other.cache_write_1h,
            thinking: self.thinking + other.thinking,
        }
    }

    /// Sum of the four billable classes. Excludes `thinking`, which is part of `output`.
    pub fn total_tokens(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write_5m + self.cache_write_1h
    }

    pub fn cache_write_total(&self) -> u64 {
        self.cache_write_5m + self.cache_write_1h
    }
}

/// One assistant message with usage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub ts: i64,
    pub model: String,
    pub usage: TurnUsage,
}

/// Facts about a session gathered while scanning its transcript.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMeta {
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub cc_version: Option<String>,
    pub ai_title: Option<String>,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    pub message_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub project_id: i64,
    pub file_path: String,
    pub file_size: u64,
    pub indexed_offset: u64,
    pub started_at: Option<i64>,
    pub last_activity_at: Option<i64>,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub cc_version: Option<String>,
    pub title: Option<String>,
    pub message_count: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_adds_componentwise() {
        let a = TurnUsage {
            input: 1,
            output: 2,
            cache_read: 3,
            cache_write_5m: 4,
            cache_write_1h: 5,
            thinking: 6,
        };
        let b = TurnUsage {
            input: 10,
            output: 20,
            cache_read: 30,
            cache_write_5m: 40,
            cache_write_1h: 50,
            thinking: 60,
        };
        let sum = a.plus(&b);
        assert_eq!(sum.input, 11);
        assert_eq!(sum.output, 22);
        assert_eq!(sum.cache_read, 33);
        assert_eq!(sum.cache_write_5m, 44);
        assert_eq!(sum.cache_write_1h, 55);
        assert_eq!(sum.thinking, 66);
    }

    #[test]
    fn total_billable_excludes_thinking() {
        // thinking tokens are a subset of output and must not be double counted
        let u = TurnUsage {
            input: 1,
            output: 100,
            cache_read: 10,
            cache_write_5m: 5,
            cache_write_1h: 5,
            thinking: 40,
        };
        assert_eq!(u.total_tokens(), 121);
    }
}
