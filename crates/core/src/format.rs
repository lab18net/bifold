//! Small formatting helpers shared by both front ends.

use chrono::{DateTime, Datelike, Local, Timelike};
use std::time::SystemTime;

/// `1.4M`, `812K`, `37 B` — compact, right-aligned friendly.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    if bytes < 1024 {
        return format!("{bytes}B");
    }
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 10.0 {
        format!("{value:.0}{}", UNITS[unit])
    } else {
        format!("{value:.1}{}", UNITS[unit])
    }
}

/// Full timestamp for details panels: `2026-11-18 10:20`.
pub fn fmt_time(time: SystemTime) -> String {
    let local: DateTime<Local> = time.into();
    local.format("%Y-%m-%d %H:%M").to_string()
}

/// Narrow timestamp for list rows: today shows `10:20`, this year `Nov 18 10:20`,
/// older `2024-03-01`.
pub fn short_time(time: SystemTime) -> String {
    let local: DateTime<Local> = time.into();
    let now = Local::now();
    if local.date_naive() == now.date_naive() {
        format!("{:02}:{:02}", local.hour(), local.minute())
    } else if local.year() == now.year() {
        local.format("%b %e %H:%M").to_string().replace("  ", " ")
    } else {
        local.format("%Y-%m-%d").to_string()
    }
}

/// Truncate from the left, keeping the tail — paths are more useful that way.
pub fn ellipsis_left(text: &str, max_chars: usize) -> String {
    let len = text.chars().count();
    if len <= max_chars || max_chars < 2 {
        return text.to_string();
    }
    let skip = len - (max_chars - 1);
    let tail: String = text.chars().skip(skip).collect();
    format!("…{tail}")
}

/// Truncate from the right, keeping the head.
pub fn ellipsis_right(text: &str, max_chars: usize) -> String {
    let len = text.chars().count();
    if len <= max_chars || max_chars == 0 {
        return text.to_string();
    }
    let head: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn sizes() {
        assert_eq!(human_size(0), "0B");
        assert_eq!(human_size(512), "512B");
        assert_eq!(human_size(1024), "1.0K");
        assert_eq!(human_size(1536), "1.5K");
        assert_eq!(human_size(10 * 1024 * 1024), "10M");
        assert_eq!(human_size(5 * 1024 * 1024 * 1024), "5.0G");
    }

    #[test]
    fn ellipsis() {
        assert_eq!(ellipsis_left("/very/long/path/nope", 6), "…/nope");
        assert_eq!(ellipsis_left("abc", 0), "abc");
        assert_eq!(ellipsis_right("abcdefgh", 4), "abc…");
        assert_eq!(ellipsis_right("abc", 10), "abc");
        assert_eq!(ellipsis_right("abc", 0), "abc");
    }

    #[test]
    fn timestamps_are_formatted() {
        let t = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert_eq!(fmt_time(t).len(), 16);
        assert!(!short_time(t).is_empty());
    }
}
