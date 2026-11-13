use std::time::SystemTime;

use anyhow::{Context, Result};
use jiff::{
    Timestamp,
    civil::DateTime,
    tz::{Offset, TimeZone},
};
use remove_except::{ModifiedRange, PlanOptions};

use crate::InputError;

#[derive(Debug)]
struct ParsedBoundary {
    datetime: DateTime,
    offset: Option<Offset>,
    explicit_z: bool,
}

#[derive(Debug)]
pub(crate) struct Boundary {
    instant: SystemTime,
    pub(crate) human: String,
    pub(crate) utc: String,
}

#[derive(Debug)]
pub(crate) struct Selection {
    pub(crate) since: Option<Boundary>,
    pub(crate) before: Option<Boundary>,
    pub(crate) options: PlanOptions,
}

fn invalid(message: impl Into<String>) -> anyhow::Error {
    InputError(message.into()).into()
}

fn parse_boundary(value: &str) -> Result<ParsedBoundary> {
    let b = value.as_bytes();
    let bad = || {
        invalid(format!(
            "Invalid modification time: {value:?}; expected YYYY-MM-DD or YYYY-MM-DDTHH:MM[:SS] with optional Z or ±HH:MM"
        ))
    };
    if !value.is_ascii() || b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return Err(bad());
    }
    let number = |start: usize, end: usize| -> Result<i32> {
        let digits = b.get(start..end).ok_or_else(bad)?;
        if !digits.iter().all(u8::is_ascii_digit) {
            return Err(bad());
        }
        std::str::from_utf8(digits)?.parse().map_err(|_| bad())
    };
    let year = number(0, 4)?;
    let month = number(5, 7)?;
    let day = number(8, 10)?;
    if year == 0 {
        return Err(bad());
    }
    let mut hour = 0;
    let mut minute = 0;
    let mut second = 0;
    let mut offset = None;
    let mut explicit_z = false;
    if b.len() != 10 {
        if b.len() < 16 || !matches!(b[10], b'T' | b' ') || b[13] != b':' {
            return Err(bad());
        }
        hour = number(11, 13)?;
        minute = number(14, 16)?;
        let mut end = 16;
        if b.get(end) == Some(&b':') {
            second = number(end + 1, end + 3)?;
            end += 3;
        }
        if hour > 23 || minute > 59 || second > 59 {
            return Err(bad());
        }
        match b.get(end..) {
            Some([]) => {}
            Some([b'Z']) => {
                offset = Some(Offset::UTC);
                explicit_z = true;
            }
            Some([sign @ (b'+' | b'-'), a, c, b':', d, e])
                if [a, c, d, e].iter().all(|v| v.is_ascii_digit()) =>
            {
                let hours = i32::from(*a - b'0') * 10 + i32::from(*c - b'0');
                let minutes = i32::from(*d - b'0') * 10 + i32::from(*e - b'0');
                if hours > 23 || minutes > 59 || (*sign == b'-' && hours == 0 && minutes == 0) {
                    return Err(bad());
                }
                let seconds = (hours * 60 + minutes) * 60 * if *sign == b'-' { -1 } else { 1 };
                offset = Some(Offset::from_seconds(seconds).map_err(|_| bad())?);
            }
            _ => return Err(bad()),
        }
    }
    let datetime = DateTime::new(
        year as i16,
        month as i8,
        day as i8,
        hour as i8,
        minute as i8,
        second as i8,
        0,
    )
    .map_err(|_| bad())?;
    Ok(ParsedBoundary {
        datetime,
        offset,
        explicit_z,
    })
}

fn resolve_boundary(parsed: ParsedBoundary, zone: Option<&TimeZone>) -> Result<Boundary> {
    let (timestamp, offset) = if let Some(offset) = parsed.offset {
        (
            offset
                .to_timestamp(parsed.datetime)
                .map_err(|error| invalid(format!("Invalid modification time: {error}")))?,
            offset,
        )
    } else {
        let zone = zone.context("Local time zone was not resolved")?;
        let zoned = zone.to_ambiguous_zoned(parsed.datetime).unambiguous()
            .map_err(|error| invalid(format!("Local modification time is ambiguous or nonexistent: {error}; specify an explicit UTC offset")))?;
        (zoned.timestamp(), zoned.offset())
    };
    let min: Timestamp = "1601-01-01T00:00:00Z".parse().expect("valid constant");
    let max: Timestamp = "9999-01-01T00:00:00Z".parse().expect("valid constant");
    if timestamp < min || timestamp >= max {
        return Err(invalid(
            "Modification time is outside the supported UTC range",
        ));
    }
    let instant = SystemTime::from(timestamp);
    if Timestamp::try_from(instant).ok() != Some(timestamp) {
        return Err(invalid(
            "Modification time cannot be represented exactly on this platform",
        ));
    }
    let seconds = offset.seconds();
    let magnitude = seconds.unsigned_abs();
    let suffix = if parsed.explicit_z {
        "Z".to_owned()
    } else if magnitude % 60 == 0 {
        format!(
            "{}{:02}:{:02}",
            if seconds < 0 { '-' } else { '+' },
            magnitude / 3600,
            magnitude % 3600 / 60
        )
    } else {
        format!(
            "{}{:02}:{:02}:{:02}",
            if seconds < 0 { '-' } else { '+' },
            magnitude / 3600,
            magnitude % 3600 / 60,
            magnitude % 60
        )
    };
    Ok(Boundary {
        instant,
        human: format!("{}{suffix}", parsed.datetime),
        utc: timestamp.to_string(),
    })
}

pub(crate) fn resolve(since: Option<&str>, before: Option<&str>) -> Result<Selection> {
    let since = since.map(parse_boundary).transpose()?;
    let before = before.map(parse_boundary).transpose()?;
    let local = since.as_ref().is_some_and(|b| b.offset.is_none())
        || before.as_ref().is_some_and(|b| b.offset.is_none());
    let zone = if local {
        if std::env::var_os("TZ").is_some_and(|value| value.is_empty()) {
            anyhow::bail!("The TZ environment variable is empty");
        }
        Some(TimeZone::try_system().context("Failed to determine the local time zone")?)
    } else {
        None
    };
    let since = since
        .map(|value| resolve_boundary(value, zone.as_ref()))
        .transpose()?;
    let before = before
        .map(|value| resolve_boundary(value, zone.as_ref()))
        .transpose()?;
    let options = if since.is_some() || before.is_some() {
        PlanOptions::new(ModifiedRange::new(
            since.as_ref().map(|b| b.instant),
            before.as_ref().map(|b| b.instant),
        )?)
    } else {
        PlanOptions::default()
    };
    Ok(Selection {
        since,
        before,
        options,
    })
}

impl Selection {
    pub(crate) fn is_active(&self) -> bool {
        self.options.modified_range().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historical_offsets_keep_second_precision() {
        let zone = TimeZone::get("Asia/Tokyo").unwrap();
        let boundary =
            resolve_boundary(parse_boundary("1880-01-01").unwrap(), Some(&zone)).unwrap();
        assert_eq!(boundary.human, "1880-01-01T00:00:00+09:18:59");
        assert_eq!(boundary.utc, "1879-12-31T14:41:01Z");
    }

    #[test]
    fn explicit_boundaries_are_exact_and_range_checked() {
        for value in ["1601-01-01T00:00:00Z", "9998-12-31T23:59:59Z"] {
            assert!(resolve(Some(value), None).is_ok(), "{value}");
        }
        for value in ["1600-12-31T23:59:59Z", "9999-01-01T00:00:00Z"] {
            assert!(resolve(Some(value), None).is_err(), "{value}");
        }
        assert!(resolve(Some("2026-01-01T09:00+09:00"), Some("2026-01-01T00:00Z")).is_err());
    }
}
