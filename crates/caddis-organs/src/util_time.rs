//! util_time.rs — zero-dep civil-time math (split from util.rs under
//! the 280-line law). ISO-8601 <-> unix, Europe/Vilnius rendering
//! with EU DST, Hinnant's civil-day algorithms (public domain).
//! No calendar crate; div_euclid/rem_euclid keep pre-epoch total.

/// ISO-8601 UTC timestamp (seconds precision) from the system clock.
pub fn iso8601_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    iso8601_from_unix(secs)
}

/// ISO-8601 UTC from unix seconds. Civil-from-days per Howard Hinnant's
/// algorithm (public domain) — deterministic, no calendar crate. Handles
/// pre-epoch seconds correctly (div_euclid/rem_euclid keep the math total).
pub fn iso8601_from_unix(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60
    )
}

/// Europe/Vilnius UTC offset in seconds for a UTC instant. EET (+2)
/// in winter, EEST (+3) in summer; EU DST rule — summer starts the
/// LAST SUNDAY of March at 01:00 UTC, ends the LAST SUNDAY of
/// October at 01:00 UTC (std-only civil math, operator order
/// 2026-08-29: the board shows Lithuanian time).
pub fn vilnius_offset_secs(utc_secs: i64) -> i64 {
    let days = utc_secs.div_euclid(86_400);
    let (y, _, _) = civil_from_days(days);
    let start = last_sunday_utc_days(y, 3) * 86_400 + 3_600;
    let end = last_sunday_utc_days(y, 10) * 86_400 + 3_600;
    if utc_secs >= start && utc_secs < end {
        3 * 3_600
    } else {
        2 * 3_600
    }
}

/// Epoch days of the last Sunday of `month` in `year`.
fn last_sunday_utc_days(year: i64, month: u32) -> i64 {
    let (ny, nm) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let first_next = days_from_civil(ny, nm, 1);
    let last_day = first_next - 1;
    // 1970-01-01 (epoch day 0) was a Thursday; wd with 0=Sunday:
    // wd = (day + 4) mod 7; Sunday <=> wd == 0.
    let wd = (last_day + 4).rem_euclid(7);
    last_day - wd
}

/// (year, month, day) -> days since 1970-01-01. Hinnant's days_from_civil.
fn days_from_civil(y: i64, m: u32, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = ((m + 9) % 12) as i64;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// ISO-8601 in Europe/Vilnius local time from UTC unix seconds.
/// Stored rows stay UTC (the sorting law); only RENDERING converts
/// (offset suffix kept; hms() slices positions 11..19).
pub fn iso8601_from_unix_vilnius(utc_secs: i64) -> String {
    let off = vilnius_offset_secs(utc_secs);
    let local = utc_secs + off;
    let days = local.div_euclid(86_400);
    let secs_of_day = local.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}+0{}:00",
        y,
        m,
        d,
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
        off / 3_600
    )
}

/// ISO-8601 UTC ("YYYY-MM-DDTHH:MM:SS..." with any suffix) -> unix
/// seconds. Render-side inverse of iso8601_from_unix.
pub fn unix_from_iso8601(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[10] != b'T' {
        return None;
    }
    let num = |a: usize, z: usize| -> Option<i64> { s.get(a..z)?.parse().ok() };
    let (y, m, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hh, mm, ss) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(days_from_civil(y, m as u32, d) * 86_400 + hh * 3_600 + mm * 60 + ss)
}

/// days since 1970-01-01 -> (year, month, day). Hinnant's civil_from_days.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}
