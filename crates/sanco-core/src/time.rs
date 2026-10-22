//! Calendar helpers. Timestamps are UTC milliseconds; the business runs on
//! Asia/Manila time (UTC+8, no daylight saving).

pub const MANILA_OFFSET_MS: i64 = 8 * 60 * 60 * 1000;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// `(year, month, day)` in Manila for a UTC timestamp.
pub fn manila_date(ms: i64) -> (i32, u32, u32) {
    civil_from_days((ms + MANILA_OFFSET_MS).div_euclid(DAY_MS))
}

pub fn manila_year(ms: i64) -> i32 {
    manila_date(ms).0
}

/// `YYYY-MM-DD` in Manila.
pub fn manila_date_string(ms: i64) -> String {
    let (y, m, d) = manila_date(ms);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `YYYYMMDD-HHMMSS` in Manila, for file names.
pub fn manila_stamp(ms: i64) -> String {
    let (y, m, d) = manila_date(ms);
    let secs = (ms + MANILA_OFFSET_MS).rem_euclid(DAY_MS) / 1000;
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

/// UTC ms of 00:00 Manila time on a `YYYY-MM-DD` date.
pub fn manila_day_start_ms(date: &str) -> Option<i64> {
    let mut parts = date.trim().splitn(3, '-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(days_from_civil(y, m, d) * DAY_MS - MANILA_OFFSET_MS)
}

/// Inverse of [`civil_from_days`].
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Days since 1970-01-01 to a proleptic Gregorian date (H. Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y as i32, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manila_dates() {
        assert_eq!(manila_date_string(0), "1970-01-01");
        // 2026-10-21 00:00 UTC is 08:00 in Manila.
        assert_eq!(manila_date_string(1_792_540_800_000), "2026-10-21");
        // 2026-12-31 16:00 UTC is already New Year in Manila.
        let nye = 1_798_732_800_000;
        assert_eq!(manila_date_string(nye - 1), "2026-12-31");
        assert_eq!(manila_date_string(nye), "2027-01-01");
        assert_eq!(manila_year(nye), 2027);
        // Leap day.
        assert_eq!(manila_date_string(1_709_164_800_000), "2024-02-29");
    }

    #[test]
    fn day_starts_and_stamps() {
        // 2026-10-21 00:00 Manila is 2026-10-20 16:00 UTC.
        let start = manila_day_start_ms("2026-10-21").unwrap();
        assert_eq!(start, 1_792_540_800_000 - MANILA_OFFSET_MS);
        assert_eq!(manila_date_string(start), "2026-10-21");
        assert_eq!(manila_date_string(start - 1), "2026-10-20");
        assert_eq!(manila_day_start_ms("2024-02-29").map(manila_date_string).as_deref(), Some("2024-02-29"));
        assert_eq!(manila_day_start_ms("2026-13-01"), None);
        assert_eq!(manila_day_start_ms("junk"), None);
        assert_eq!(manila_stamp(1_792_540_800_000 + 3_723_000), "20261021-090203");
    }
}
