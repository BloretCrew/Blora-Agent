// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_types::{BloraError, Result};
use chrono::{DateTime, Datelike, Duration, Timelike, Utc};

/// Compute the next UTC time matching a 5-field cron (`m h dom mon dow`).
pub fn next_cron(expr: &str, from: DateTime<Utc>) -> Result<DateTime<Utc>> {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(BloraError::Other(
            "cron must have 5 fields: minute hour day month weekday".to_owned(),
        ));
    }
    let minute = parse_field(fields[0], 0, 59)?;
    let hour = parse_field(fields[1], 0, 23)?;
    let day = parse_field(fields[2], 1, 31)?;
    let month = parse_field(fields[3], 1, 12)?;
    let weekday = parse_field(fields[4], 0, 6)?;
    let mut cursor = from
        .with_second(0)
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(from)
        + Duration::minutes(1);
    for _ in 0..(60 * 24 * 14) {
        if minute.contains(&cursor.minute())
            && hour.contains(&cursor.hour())
            && day.contains(&cursor.day())
            && month.contains(&cursor.month())
            && weekday.contains(&cursor.weekday().num_days_from_sunday())
        {
            return Ok(cursor);
        }
        cursor += Duration::minutes(1);
    }
    Err(BloraError::Other(format!(
        "no cron match in 14 days for {expr}"
    )))
}

fn parse_field(field: &str, min: u32, max: u32) -> Result<Vec<u32>> {
    if field == "*" {
        return Ok((min..=max).collect());
    }
    if field.contains(',') {
        let mut out = Vec::new();
        for part in field.split(',') {
            out.extend(parse_field(part, min, max)?);
        }
        out.sort_unstable();
        out.dedup();
        return Ok(out);
    }
    if let Some((start, end)) = field.split_once('-') {
        let start = parse_single(start, min, max)?;
        let end = parse_single(end, min, max)?;
        if start > end {
            return Err(BloraError::Other(format!("cron range {field} is inverted")));
        }
        return Ok((start..=end).collect());
    }
    if let Some(step) = field.strip_prefix("*/") {
        let step: u32 = step
            .parse()
            .map_err(|_| BloraError::Other(format!("invalid cron step {field}")))?;
        if step == 0 {
            return Err(BloraError::Other("cron step cannot be 0".to_owned()));
        }
        return Ok((min..=max)
            .filter(|value| (*value - min) % step == 0)
            .collect());
    }
    Ok(vec![parse_single(field, min, max)?])
}

fn parse_single(field: &str, min: u32, max: u32) -> Result<u32> {
    let value: u32 = field
        .parse()
        .map_err(|_| BloraError::Other(format!("invalid cron field {field}")))?;
    if value < min || value > max {
        return Err(BloraError::Other(format!(
            "cron field {value} out of range {min}-{max}"
        )));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn every_hour() {
        let from = Utc.with_ymd_and_hms(2026, 9, 13, 10, 15, 0).unwrap();
        let next = next_cron("0 * * * *", from).unwrap();
        assert_eq!(next.minute(), 0);
        assert_eq!(next.hour(), 11);
    }

    #[test]
    fn list_and_range() {
        let from = Utc.with_ymd_and_hms(2026, 9, 13, 10, 10, 0).unwrap();
        let next = next_cron("15,45 * * * *", from).unwrap();
        assert_eq!(next.minute(), 15);
        let next = next_cron("0 9-11 * * *", from).unwrap();
        assert_eq!(next.hour(), 11);
        assert_eq!(next.minute(), 0);
    }
}
