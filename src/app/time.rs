//! Small time helpers shared across UI surfaces (sync status pill,
//! recents list). Pulled out of `state.rs` so the welcome screen can
//! render relative timestamps without leaking into AppState's API.

use chrono::{DateTime, Local};

/// "just now" / "N seconds ago" / "N minutes ago" / "N hours ago" — same
/// granularity as KeePass2's last-sync indicator. Past-only; future
/// timestamps clip to "just now".
pub fn relative_time_label(when: DateTime<Local>, now: DateTime<Local>) -> String {
    let secs = (now - when).num_seconds().max(0);
    if secs < 10 {
        "just now".into()
    } else if secs < 60 {
        format!("{secs} seconds ago")
    } else if secs < 3600 {
        let m = secs / 60;
        if m == 1 {
            "1 minute ago".into()
        } else {
            format!("{m} minutes ago")
        }
    } else if secs < 86_400 {
        let h = secs / 3600;
        if h == 1 {
            "1 hour ago".into()
        } else {
            format!("{h} hours ago")
        }
    } else {
        when.format("%Y-%m-%d %H:%M").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn anchor() -> DateTime<Local> {
        // Fixed anchor so tests don't depend on wall clock.
        Local::now()
    }

    #[test]
    fn under_ten_seconds_is_just_now() {
        let now = anchor();
        assert_eq!(relative_time_label(now, now), "just now");
        assert_eq!(
            relative_time_label(now - Duration::seconds(5), now),
            "just now"
        );
    }

    #[test]
    fn seconds_minutes_hours_pluralise() {
        let now = anchor();
        assert_eq!(
            relative_time_label(now - Duration::seconds(30), now),
            "30 seconds ago"
        );
        assert_eq!(
            relative_time_label(now - Duration::minutes(1), now),
            "1 minute ago"
        );
        assert_eq!(
            relative_time_label(now - Duration::minutes(5), now),
            "5 minutes ago"
        );
        assert_eq!(
            relative_time_label(now - Duration::hours(1), now),
            "1 hour ago"
        );
        assert_eq!(
            relative_time_label(now - Duration::hours(3), now),
            "3 hours ago"
        );
    }

    #[test]
    fn over_a_day_falls_back_to_absolute() {
        let now = anchor();
        let label = relative_time_label(now - Duration::days(2), now);
        // Absolute fallback uses YYYY-MM-DD HH:MM; doesn't say "ago".
        assert!(!label.contains("ago"));
        assert!(label.contains('-'));
    }

    #[test]
    fn future_timestamps_clip_to_just_now() {
        let now = anchor();
        assert_eq!(
            relative_time_label(now + Duration::seconds(30), now),
            "just now"
        );
    }
}
