use chrono::{Datelike, Timelike, Utc};

/// Simple cron expression matcher.
/// Supports: `*/N`, `*`, and literal values for minute, hour, day-of-month, month, day-of-week.
/// Format: "minute hour dom month dow"
pub fn cron_matches(expression: &str, now: &chrono::DateTime<Utc>) -> bool {
    let parts: Vec<&str> = expression.split_whitespace().collect();
    if parts.len() != 5 {
        return false;
    }

    let fields = [
        (parts[0], now.minute() as u32),
        (parts[1], now.hour() as u32),
        (parts[2], now.day()),
        (parts[3], now.month()),
        (parts[4], now.weekday().num_days_from_sunday()),
    ];

    fields.iter().all(|(pattern, value)| field_matches(pattern, *value))
}

fn field_matches(pattern: &str, value: u32) -> bool {
    if pattern == "*" {
        return true;
    }

    // Handle */N (every N)
    if let Some(step) = pattern.strip_prefix("*/") {
        if let Ok(n) = step.parse::<u32>() {
            if n == 0 {
                return false;
            }
            return value % n == 0;
        }
    }

    // Handle comma-separated values
    if pattern.contains(',') {
        return pattern.split(',').any(|p| field_matches(p.trim(), value));
    }

    // Handle range (N-M)
    if pattern.contains('-') {
        let parts: Vec<&str> = pattern.split('-').collect();
        if parts.len() == 2 {
            if let (Ok(start), Ok(end)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                return value >= start && value <= end;
            }
        }
    }

    // Literal value
    if let Ok(n) = pattern.parse::<u32>() {
        return value == n;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_every_5_minutes() {
        let expr = "*/5 * * * *";
        let t = Utc.with_ymd_and_hms(2026, 2, 20, 12, 0, 0).unwrap();
        assert!(cron_matches(expr, &t));

        let t = Utc.with_ymd_and_hms(2026, 2, 20, 12, 5, 0).unwrap();
        assert!(cron_matches(expr, &t));

        let t = Utc.with_ymd_and_hms(2026, 2, 20, 12, 3, 0).unwrap();
        assert!(!cron_matches(expr, &t));
    }

    #[test]
    fn test_specific_time() {
        let expr = "30 14 * * *";
        let t = Utc.with_ymd_and_hms(2026, 2, 20, 14, 30, 0).unwrap();
        assert!(cron_matches(expr, &t));

        let t = Utc.with_ymd_and_hms(2026, 2, 20, 14, 31, 0).unwrap();
        assert!(!cron_matches(expr, &t));
    }

    #[test]
    fn test_wildcard() {
        let expr = "* * * * *";
        let t = Utc.with_ymd_and_hms(2026, 2, 20, 12, 34, 0).unwrap();
        assert!(cron_matches(expr, &t));
    }

    #[test]
    fn test_range() {
        let expr = "0 9-17 * * *";
        let t = Utc.with_ymd_and_hms(2026, 2, 20, 12, 0, 0).unwrap();
        assert!(cron_matches(expr, &t));

        let t = Utc.with_ymd_and_hms(2026, 2, 20, 20, 0, 0).unwrap();
        assert!(!cron_matches(expr, &t));
    }

    #[test]
    fn test_comma_list() {
        let expr = "0,15,30,45 * * * *";
        let t = Utc.with_ymd_and_hms(2026, 2, 20, 12, 15, 0).unwrap();
        assert!(cron_matches(expr, &t));

        let t = Utc.with_ymd_and_hms(2026, 2, 20, 12, 16, 0).unwrap();
        assert!(!cron_matches(expr, &t));
    }

    #[test]
    fn test_invalid_expression() {
        assert!(!cron_matches("invalid", &Utc::now()));
        assert!(!cron_matches("* * *", &Utc::now())); // too few fields
    }
}
