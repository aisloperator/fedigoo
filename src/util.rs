//! Small formatting helpers.

use gtk::glib;

fn parse_time(iso: &str) -> Option<glib::DateTime> {
    glib::DateTime::from_iso8601(iso, Some(&glib::TimeZone::utc())).ok()
}

/// "12s", "5m", "3h", "6d", or a date for anything older than a week.
pub fn relative_time(iso: &str) -> String {
    let Some(t) = parse_time(iso) else {
        return String::new();
    };
    let Ok(now) = glib::DateTime::now_utc() else {
        return String::new();
    };
    let secs = now.difference(&t).as_seconds().max(0);
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86400 => format!("{}h", s / 3600),
        s if s < 7 * 86400 => format!("{}d", s / 86400),
        _ => t
            .to_local()
            .ok()
            .and_then(|l| l.format("%b %-d, %Y").ok())
            .map(|s| s.to_string())
            .unwrap_or_default(),
    }
}

pub fn absolute_time(iso: &str) -> String {
    parse_time(iso)
        .and_then(|t| t.to_local().ok())
        .and_then(|l| l.format("%Y-%m-%d %H:%M").ok())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// Remaining time until `iso`, e.g. "2 hours left"; "closed" when past.
pub fn time_left(iso: &str) -> String {
    let (Some(t), Ok(now)) = (parse_time(iso), glib::DateTime::now_utc()) else {
        return String::new();
    };
    let secs = t.difference(&now).as_seconds();
    let plural = |n: i64, unit: &str| format!("{n} {unit}{} left", if n == 1 { "" } else { "s" });
    match secs {
        s if s <= 0 => "closed".into(),
        s if s < 3600 => plural(s / 60 + 1, "minute"),
        s if s < 86400 => plural(s / 3600, "hour"),
        s => plural(s / 86400, "day"),
    }
}

pub fn count(n: i64) -> String {
    match n {
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1e6),
        n if n >= 10_000 => format!("{}K", n / 1000),
        n if n >= 1_000 => format!("{:.1}K", n as f64 / 1e3),
        n => n.to_string(),
    }
}

/// Length of a post as Mastodon counts it: every URL counts as 23
/// characters and remote mentions count only the local part.
pub fn post_length(text: &str) -> usize {
    text.split_inclusive(char::is_whitespace)
        .map(|word| {
            let trimmed = word.trim_end();
            let trailing = word.chars().count() - trimmed.chars().count();
            let n = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                23
            } else if trimmed.starts_with('@') && trimmed[1..].contains('@') {
                trimmed[1..]
                    .split('@')
                    .next()
                    .map_or(0, |u| u.chars().count() + 1)
            } else {
                trimmed.chars().count()
            };
            n + trailing
        })
        .sum()
}

pub fn visibility_icon(vis: &str) -> &'static str {
    match vis {
        "public" => "web-browser-symbolic",
        "unlisted" => "system-lock-screen-symbolic",
        "private" => "system-users-symbolic",
        _ => "mail-unread-symbolic",
    }
}

pub fn visibility_label(vis: &str) -> &'static str {
    match vis {
        "public" => "Public",
        "unlisted" => "Quiet public",
        "private" => "Followers only",
        _ => "Mentioned people only",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths() {
        assert_eq!(post_length("hello"), 5);
        assert_eq!(
            post_length("see https://example.com/a/very/long/url/indeed ok"),
            4 + 23 + 3
        );
        assert_eq!(post_length("@bob@example.social hi"), 4 + 3);
    }
}
