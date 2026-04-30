use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};

use crate::config::AppConfig;

pub async fn fetch_next_title(config: &AppConfig) -> Result<Option<String>, reqwest::Error> {
    let Some(calendar) = config.calendars.first() else {
        return Ok(None);
    };

    let response = reqwest::get(&calendar.ical_url).await?;
    let body = response.error_for_status()?.text().await?;
    let now = Utc::now();
    let next_event = parse_next_event(&body, now);

    Ok(next_event.map(|event| format_title(config, &event.title, event.start, now)))
}

#[derive(Debug)]
struct CalendarEvent {
    start: DateTime<Utc>,
    title: String,
}

fn parse_next_event(ics: &str, now: DateTime<Utc>) -> Option<CalendarEvent> {
    let mut current_start: Option<DateTime<Utc>> = None;
    let mut current_title: Option<String> = None;
    let mut events = Vec::new();

    for line in unfold_ical_lines(ics) {
        match line.as_str() {
            "BEGIN:VEVENT" => {
                current_start = None;
                current_title = None;
            }
            "END:VEVENT" => {
                if let (Some(start), Some(title)) = (current_start.take(), current_title.take()) {
                    events.push(CalendarEvent { start, title });
                }
            }
            _ => {
                if line.starts_with("DTSTART") {
                    if let Some((_, raw_value)) = line.split_once(':') {
                        current_start = parse_datetime(raw_value);
                    }
                } else if let Some((_, summary)) = line.split_once("SUMMARY:") {
                    current_title = Some(summary.to_string());
                }
            }
        }
    }

    events
        .into_iter()
        .filter(|event| event.start >= now)
        .min_by_key(|event| event.start)
}

fn unfold_ical_lines(ics: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();

    for raw_line in ics.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(last) = lines.last_mut() {
                last.push_str(line.trim_start());
            }
            continue;
        }
        lines.push(line.to_string());
    }

    lines
}

fn parse_datetime(value: &str) -> Option<DateTime<Utc>> {
    if let Ok(naive_utc) = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%SZ") {
        return Some(DateTime::from_naive_utc_and_offset(naive_utc, Utc));
    }

    if let Ok(local_naive) = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S") {
        return Local
            .from_local_datetime(&local_naive)
            .single()
            .or_else(|| Local.from_local_datetime(&local_naive).earliest())
            .map(|dt| dt.with_timezone(&Utc));
    }

    if let Ok(local_date) = NaiveDate::parse_from_str(value, "%Y%m%d") {
        let local_naive = local_date.and_hms_opt(0, 0, 0)?;
        return Local
            .from_local_datetime(&local_naive)
            .single()
            .or_else(|| Local.from_local_datetime(&local_naive).earliest())
            .map(|dt| dt.with_timezone(&Utc));
    }

    None
}

fn format_title(config: &AppConfig, title: &str, start: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let seconds = (start - now).num_seconds().max(0);
    let minutes_until = (seconds + 59) / 60;
    let title_value = if config.display.show_title { title } else { "" };

    config
        .display
        .normal_format
        .replace("{minutes_until}", &minutes_until.to_string())
        .replace("{title}", title_value)
        .trim()
        .to_string()
}
