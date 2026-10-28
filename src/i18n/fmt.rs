//! Numbers, dates and times, written the way the reader expects them.
//!
//! Counts follow the UI language: 1,234 in English, 1.234 in German. Dates
//! and times come from macOS, which applies the user's region and their 12-
//! or 24-hour setting. Unit tests get a fixed formatter instead, so what they
//! expect does not depend on the Mac they run on.

use chrono::{DateTime, TimeZone};

use super::Language;

/// `n` with thousands separators for `language`.
pub fn integer(n: u64, language: Language) -> String {
    let separator = match language {
        Language::English => ',',
        Language::German => '.',
    };
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(separator);
        }
        out.push(digit);
    }
    out
}

/// The time of day, short: "13:40", or "1:40 PM" with the 12-hour clock.
pub fn time<Tz: TimeZone>(at: &DateTime<Tz>) -> String {
    format(at, Style::Time)
}

/// The date at medium length: "28.10.2026" or "Oct 28, 2026".
pub fn date<Tz: TimeZone>(at: &DateTime<Tz>) -> String {
    format(at, Style::Date)
}

/// Date and time, both short: "28.10.26, 13:40" or "10/28/26, 1:40 PM".
pub fn date_time<Tz: TimeZone>(at: &DateTime<Tz>) -> String {
    format(at, Style::DateTime)
}

#[derive(Clone, Copy, Debug)]
enum Style {
    Date,
    Time,
    DateTime,
}

#[cfg(all(target_os = "macos", not(test)))]
fn format<Tz: TimeZone>(at: &DateTime<Tz>, style: Style) -> String {
    system(at, style)
}

#[cfg(any(not(target_os = "macos"), test))]
fn format<Tz: TimeZone>(at: &DateTime<Tz>, style: Style) -> String {
    fixed(at, style)
}

/// What macOS writes for the user's region and clock setting.
#[cfg(target_os = "macos")]
fn system<Tz: TimeZone>(at: &DateTime<Tz>, style: Style) -> String {
    use objc2_foundation::{NSDate, NSDateFormatter, NSDateFormatterStyle};

    let (date_style, time_style) = match style {
        Style::Date => (
            NSDateFormatterStyle::MediumStyle,
            NSDateFormatterStyle::NoStyle,
        ),
        Style::Time => (
            NSDateFormatterStyle::NoStyle,
            NSDateFormatterStyle::ShortStyle,
        ),
        Style::DateTime => (
            NSDateFormatterStyle::ShortStyle,
            NSDateFormatterStyle::ShortStyle,
        ),
    };
    let date = NSDate::dateWithTimeIntervalSince1970(at.timestamp_millis() as f64 / 1000.0);
    NSDateFormatter::localizedStringFromDate_dateStyle_timeStyle(&date, date_style, time_style)
        .to_string()
}

/// The stand-in for unit tests: ISO dates and 24-hour times in UTC, the same
/// on every machine.
#[cfg(any(not(target_os = "macos"), test))]
fn fixed<Tz: TimeZone>(at: &DateTime<Tz>, style: Style) -> String {
    let pattern = match style {
        Style::Date => "%Y-%m-%d",
        Style::Time => "%H:%M",
        Style::DateTime => "%Y-%m-%d %H:%M",
    };
    at.with_timezone(&chrono::Utc).format(pattern).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn integers_get_the_separator_of_their_language() {
        for (n, english, german) in [
            (0, "0", "0"),
            (7, "7", "7"),
            (999, "999", "999"),
            (1_000, "1,000", "1.000"),
            (12_345, "12,345", "12.345"),
            (1_234_567, "1,234,567", "1.234.567"),
            (
                u64::MAX,
                "18,446,744,073,709,551,615",
                "18.446.744.073.709.551.615",
            ),
        ] {
            assert_eq!(integer(n, Language::English), english);
            assert_eq!(integer(n, Language::German), german);
        }
    }

    #[test]
    fn tests_see_the_fixed_formatter() {
        let at = Utc.with_ymd_and_hms(2026, 10, 28, 13, 40, 5).unwrap();
        assert_eq!(time(&at), "13:40");
        assert_eq!(date(&at), "2026-10-28");
        assert_eq!(date_time(&at), "2026-10-28 13:40");

        // The same instant in another zone reads the same.
        let berlin = at.with_timezone(&chrono::FixedOffset::east_opt(3600).unwrap());
        assert_eq!(date_time(&berlin), "2026-10-28 13:40");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_system_formatter_answers_for_every_style() {
        // Region, clock and digits differ from Mac to Mac, so only check that
        // macOS wrote something and that the styles differ.
        let at = Utc.with_ymd_and_hms(2026, 10, 28, 13, 40, 5).unwrap();
        let date = system(&at, Style::Date);
        let time = system(&at, Style::Time);
        let both = system(&at, Style::DateTime);
        for (style, text) in [("date", &date), ("time", &time), ("date and time", &both)] {
            assert!(!text.trim().is_empty(), "no {style} from NSDateFormatter");
        }
        assert_ne!(date, time);
        assert!(both.len() > time.len());
    }
}
