//! Timestamps are stored as Unix seconds and shown in the local time zone.
use jiff::{Timestamp, tz::TimeZone};

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
fn zoned(seconds: i64) -> jiff::Zoned {
    Timestamp::from_second(seconds)
        .unwrap_or(Timestamp::UNIX_EPOCH)
        .to_zoned(TimeZone::system())
}
/// `2026-10-08 21:29`
pub fn local(seconds: i64) -> String {
    zoned(seconds).strftime("%Y-%m-%d %H:%M").to_string()
}
/// `8 Oct 2026`
pub fn date(seconds: i64) -> String {
    zoned(seconds).strftime("%-d %b %Y").to_string()
}
/// `2026-10-08T21:29:00Z`, for EPUB and DOCX metadata.
pub fn iso(seconds: i64) -> String {
    Timestamp::from_second(seconds)
        .unwrap_or(Timestamp::UNIX_EPOCH)
        .strftime("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}
/// The local calendar day, to thin out old automatic snapshots.
pub fn day(seconds: i64) -> i64 {
    let z = zoned(seconds);
    z.date().to_zoned(TimeZone::UTC).map_or(seconds / 86400, |d| d.timestamp().as_second() / 86400)
}
/// A rough "5 minutes ago".
pub fn ago(seconds: i64, now: i64) -> String {
    let d = (now - seconds).max(0);
    match d {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", d / 60),
        3600..=86399 => format!("{} h ago", d / 3600),
        _ => format!("{} days ago", d / 86400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn formats_known_instants() {
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso(1_760_000_000), "2025-10-09T08:53:20Z");
        assert!(local(1_760_000_000).starts_with("2025-10-"));
        assert_eq!(ago(100, 100), "just now");
        assert_eq!(ago(0, 7200), "2 h ago");
        assert_eq!(day(86400 * 5 + 3), day(86400 * 5 + 7));
    }
}
