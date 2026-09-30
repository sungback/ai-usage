//! 테마 날짜·시각 — 처음 보시는 분을 위한 안내.
//!
//! - "3일 뒤", "2시간 전" 같은 표시를 위한 날짜 조각을 만듭니다.
//! - Windows는 OS 달력 API를 쓰고, 그 외는 계산으로 구합니다.

use super::*;

#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
#[cfg(windows)]
use windows::Win32::Globalization::{
    GetDateFormatEx, GetTimeFormatEx, DATE_LONGDATE, DATE_SHORTDATE, ENUM_DATE_FORMATS_FLAGS,
    TIME_FORMAT_FLAGS, TIME_NOSECONDS,
};
#[cfg(windows)]
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTimeEx};

#[cfg(windows)]
const WINDOWS_TO_UNIX_SECONDS: u64 = 11_644_473_600;
#[cfg(windows)]
const TICKS_PER_SECOND: u64 = 10_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DateTimeParts {
    pub(super) year: u16,
    pub(super) month: u16,
    pub(super) day: u16,
    /// 번들된 테마 예제와 일치하도록 월요일은 0, 일요일은 6으로 표현합니다.
    pub(super) weekday: u16,
    pub(super) hour: u16,
    pub(super) minute: u16,
    pub(super) second: u16,
}

#[cfg(windows)]
pub(super) fn timestamp_parts(unix: f64, local: bool) -> Option<DateTimeParts> {
    let value = timestamp_system_time(unix, local)?;
    Some(DateTimeParts {
        year: value.wYear,
        month: value.wMonth,
        day: value.wDay,
        weekday: (value.wDayOfWeek + 6) % 7,
        hour: value.wHour,
        minute: value.wMinute,
        second: value.wSecond,
    })
}

#[cfg(not(windows))]
pub(super) fn timestamp_parts(unix: f64, _local: bool) -> Option<DateTimeParts> {
    unix_to_parts(unix)
}

#[cfg(not(windows))]
fn unix_to_parts(unix: f64) -> Option<DateTimeParts> {
    if !unix.is_finite() || unix <= 0.0 {
        return None;
    }
    let total_secs = unix.floor() as i64;
    let days = total_secs.div_euclid(86400);
    let rem_secs = total_secs.rem_euclid(86400);
    let hour = (rem_secs / 3600) as u16;
    let minute = ((rem_secs % 3600) / 60) as u16;
    let second = (rem_secs % 60) as u16;
    let weekday = ((days + 3).rem_euclid(7)) as u16;

    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    Some(DateTimeParts {
        year: y as u16,
        month: m as u16,
        day: d as u16,
        weekday,
        hour,
        minute,
        second,
    })
}

#[cfg(windows)]
pub(super) fn format_timestamp(unix: f64, format: &str, context: &DataContext) -> Option<String> {
    let normalized = format.trim().to_ascii_lowercase();
    let (utc, format) = normalized
        .strip_prefix("utc_")
        .map_or((false, normalized.as_str()), |format| (true, format));
    if !is_timestamp_format(format) {
        return None;
    }
    let Some(value) = timestamp_system_time(unix, !utc) else {
        return Some("--".into());
    };
    let locale = context.get_string("i18n.locale").unwrap_or("en");
    let result = match format {
        "weekday_2" => {
            date_pattern(&value, "ddd", locale).map(|value| value.chars().take(2).collect())
        }
        "weekday_short" => date_pattern(&value, "ddd", locale),
        "weekday_long" => date_pattern(&value, "dddd", locale),
        "day" => date_pattern(&value, "d", locale),
        "day_2" => date_pattern(&value, "dd", locale),
        "month" => date_pattern(&value, "M", locale),
        "month_2" => date_pattern(&value, "MM", locale),
        "month_short" => date_pattern(&value, "MMM", locale),
        "month_long" => date_pattern(&value, "MMMM", locale),
        "year_2" => date_pattern(&value, "yy", locale),
        "year" => date_pattern(&value, "yyyy", locale),
        "date" | "date_short" => date_default(&value, DATE_SHORTDATE, locale),
        "date_long" => date_default(&value, DATE_LONGDATE, locale),
        "time" | "time_short" => time_default(&value, TIME_NOSECONDS, locale),
        "time_seconds" => time_default(&value, TIME_FORMAT_FLAGS(0), locale),
        "time_24" => time_pattern(&value, "HH':'mm", locale),
        "time_24_seconds" => time_pattern(&value, "HH':'mm':'ss", locale),
        "time_12" => time_pattern(&value, "h':'mm tt", locale),
        "time_12_seconds" => time_pattern(&value, "h':'mm':'ss tt", locale),
        "am_pm" => time_pattern(&value, "tt", locale),
        "datetime" | "datetime_short" => combine(
            date_default(&value, DATE_SHORTDATE, locale),
            time_default(&value, TIME_NOSECONDS, locale),
        ),
        "datetime_long" => combine(
            date_default(&value, DATE_LONGDATE, locale),
            time_default(&value, TIME_FORMAT_FLAGS(0), locale),
        ),
        "iso_date" => Some(format!(
            "{:04}-{:02}-{:02}",
            value.wYear, value.wMonth, value.wDay
        )),
        "iso_time" => Some(format!(
            "{:02}:{:02}:{:02}",
            value.wHour, value.wMinute, value.wSecond
        )),
        "iso_datetime" => Some(format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}",
            value.wYear,
            value.wMonth,
            value.wDay,
            value.wHour,
            value.wMinute,
            value.wSecond,
            if utc { "Z" } else { "" }
        )),
        _ => None,
    };
    Some(result.unwrap_or_else(|| "--".into()))
}

#[cfg(not(windows))]
pub(super) fn format_timestamp(unix: f64, format: &str, _context: &DataContext) -> Option<String> {
    let normalized = format.trim().to_ascii_lowercase();
    let (utc, format) = normalized
        .strip_prefix("utc_")
        .map_or((false, normalized.as_str()), |format| (true, format));
    if !is_timestamp_format(format) {
        return None;
    }
    let Some(p) = unix_to_parts(unix) else {
        return Some("--".into());
    };
    const WEEKDAYS_SHORT: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const WEEKDAYS_LONG: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
    const MONTHS_SHORT: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    const MONTHS_LONG: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

    let result = match format {
        "weekday_2" => Some(WEEKDAYS_SHORT[p.weekday as usize][..2].to_string()),
        "weekday_short" => Some(WEEKDAYS_SHORT[p.weekday as usize].to_string()),
        "weekday_long" => Some(WEEKDAYS_LONG[p.weekday as usize].to_string()),
        "day" => Some(format!("{}", p.day)),
        "day_2" => Some(format!("{:02}", p.day)),
        "month" => Some(format!("{}", p.month)),
        "month_2" => Some(format!("{:02}", p.month)),
        "month_short" => Some(MONTHS_SHORT[(p.month.saturating_sub(1) as usize).min(11)].to_string()),
        "month_long" => Some(MONTHS_LONG[(p.month.saturating_sub(1) as usize).min(11)].to_string()),
        "year_2" => Some(format!("{:02}", p.year % 100)),
        "year" => Some(format!("{:04}", p.year)),
        "date" | "date_short" => Some(format!("{:04}-{:02}-{:02}", p.year, p.month, p.day)),
        "date_long" => Some(format!("{} {}, {:04}", MONTHS_LONG[(p.month.saturating_sub(1) as usize).min(11)], p.day, p.year)),
        "time" | "time_short" => Some(format!("{:02}:{:02}", p.hour, p.minute)),
        "time_seconds" => Some(format!("{:02}:{:02}:{:02}", p.hour, p.minute, p.second)),
        "time_24" => Some(format!("{:02}:{:02}", p.hour, p.minute)),
        "time_24_seconds" => Some(format!("{:02}:{:02}:{:02}", p.hour, p.minute, p.second)),
        "time_12" => {
            let h12 = match p.hour % 12 { 0 => 12, h => h };
            let am_pm = if p.hour < 12 { "AM" } else { "PM" };
            Some(format!("{:02}:{:02} {}", h12, p.minute, am_pm))
        },
        "time_12_seconds" => {
            let h12 = match p.hour % 12 { 0 => 12, h => h };
            let am_pm = if p.hour < 12 { "AM" } else { "PM" };
            Some(format!("{:02}:{:02}:{:02} {}", h12, p.minute, p.second, am_pm))
        },
        "am_pm" => Some((if p.hour < 12 { "AM" } else { "PM" }).to_string()),
        "datetime" | "datetime_short" => Some(format!("{:04}-{:02}-{:02} {:02}:{:02}", p.year, p.month, p.day, p.hour, p.minute)),
        "datetime_long" => Some(format!("{} {}, {:04} {:02}:{:02}:{:02}", MONTHS_LONG[(p.month.saturating_sub(1) as usize).min(11)], p.day, p.year, p.hour, p.minute, p.second)),
        "iso_date" => Some(format!("{:04}-{:02}-{:02}", p.year, p.month, p.day)),
        "iso_time" => Some(format!("{:02}:{:02}:{:02}", p.hour, p.minute, p.second)),
        "iso_datetime" => Some(format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}",
            p.year,
            p.month,
            p.day,
            p.hour,
            p.minute,
            p.second,
            if utc { "Z" } else { "" }
        )),
        _ => None,
    };
    Some(result.unwrap_or_else(|| "--".into()))
}

fn is_timestamp_format(format: &str) -> bool {
    matches!(
        format,
        "weekday_2"
            | "weekday_short"
            | "weekday_long"
            | "day"
            | "day_2"
            | "month"
            | "month_2"
            | "month_short"
            | "month_long"
            | "year_2"
            | "year"
            | "date"
            | "date_short"
            | "date_long"
            | "time"
            | "time_short"
            | "time_seconds"
            | "time_24"
            | "time_24_seconds"
            | "time_12"
            | "time_12_seconds"
            | "am_pm"
            | "datetime"
            | "datetime_short"
            | "datetime_long"
            | "iso_date"
            | "iso_time"
            | "iso_datetime"
    )
}

#[cfg(windows)]
fn timestamp_system_time(unix: f64, local: bool) -> Option<SYSTEMTIME> {
    if !unix.is_finite() || unix <= 0.0 {
        return None;
    }
    let seconds = unix.floor() as u64;
    let milliseconds = ((unix - seconds as f64) * 1000.0).floor() as u64;
    let ticks = seconds
        .checked_add(WINDOWS_TO_UNIX_SECONDS)?
        .checked_mul(TICKS_PER_SECOND)?
        .checked_add(milliseconds.min(999) * 10_000)?;
    let file_time = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    unsafe { FileTimeToSystemTime(&file_time, &mut utc).ok()? };
    if !local {
        return Some(utc);
    }
    let mut local = SYSTEMTIME::default();
    unsafe { SystemTimeToTzSpecificLocalTimeEx(None, &utc, &mut local).ok()? };
    Some(local)
}

#[cfg(windows)]
fn date_default(
    value: &SYSTEMTIME,
    flags: ENUM_DATE_FORMATS_FLAGS,
    locale: &str,
) -> Option<String> {
    format_date(value, flags, None, locale)
}

#[cfg(windows)]
fn date_pattern(value: &SYSTEMTIME, pattern: &str, locale: &str) -> Option<String> {
    format_date(value, ENUM_DATE_FORMATS_FLAGS(0), Some(pattern), locale)
}

#[cfg(windows)]
fn format_date(
    value: &SYSTEMTIME,
    flags: ENUM_DATE_FORMATS_FLAGS,
    pattern: Option<&str>,
    locale: &str,
) -> Option<String> {
    let locale = wide(locale);
    let pattern = pattern.map(wide);
    let mut output = [0_u16; 128];
    let count = unsafe {
        GetDateFormatEx(
            PCWSTR(locale.as_ptr()),
            flags,
            Some(value),
            pattern
                .as_ref()
                .map_or(PCWSTR::null(), |value| PCWSTR(value.as_ptr())),
            Some(&mut output),
            PCWSTR::null(),
        )
    };
    wide_result(&output, count)
}

#[cfg(windows)]
fn time_default(value: &SYSTEMTIME, flags: TIME_FORMAT_FLAGS, locale: &str) -> Option<String> {
    format_time(value, flags, None, locale)
}

#[cfg(windows)]
fn time_pattern(value: &SYSTEMTIME, pattern: &str, locale: &str) -> Option<String> {
    format_time(value, TIME_FORMAT_FLAGS(0), Some(pattern), locale)
}

#[cfg(windows)]
fn format_time(
    value: &SYSTEMTIME,
    flags: TIME_FORMAT_FLAGS,
    pattern: Option<&str>,
    locale: &str,
) -> Option<String> {
    let locale = wide(locale);
    let pattern = pattern.map(wide);
    let mut output = [0_u16; 128];
    let count = unsafe {
        GetTimeFormatEx(
            PCWSTR(locale.as_ptr()),
            flags,
            Some(value),
            pattern
                .as_ref()
                .map_or(PCWSTR::null(), |value| PCWSTR(value.as_ptr())),
            Some(&mut output),
        )
    };
    wide_result(&output, count)
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn wide_result(value: &[u16], count: i32) -> Option<String> {
    let count = usize::try_from(count).ok()?.checked_sub(1)?;
    Some(String::from_utf16_lossy(value.get(..count)?))
}

#[cfg(windows)]
fn combine(first: Option<String>, second: Option<String>) -> Option<String> {
    Some(format!("{} {}", first?, second?))
}
