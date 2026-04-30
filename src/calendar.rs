use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use rrule::{RRule, RRuleSet, Tz};
use std::collections::HashSet;
use std::str::FromStr;

use crate::config::AppConfig;

pub async fn fetch_next_title(config: &AppConfig) -> Result<Option<String>, reqwest::Error> {
    if config.calendars.is_empty() {
        return Ok(None);
    }

    let now = Utc::now();
    let mut first_error: Option<reqwest::Error> = None;
    let mut best_event: Option<CalendarEvent> = None;

    for calendar in &config.calendars {
        let response = match reqwest::get(&calendar.ical_url).await {
            Ok(response) => response,
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
                continue;
            }
        };

        let body = match response.error_for_status() {
            Ok(ok_response) => match ok_response.text().await {
                Ok(body) => body,
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    continue;
                }
            },
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
                continue;
            }
        };

        if let Some(event) = parse_next_event(&body, now) {
            let should_replace = match &best_event {
                Some(existing) => event.display_time(now) < existing.display_time(now),
                None => true,
            };
            if should_replace {
                best_event = Some(event);
            }
        }
    }

    if let Some(event) = best_event {
        return Ok(Some(format_title(
            config,
            &event.title,
            event.display_time(now),
            now,
        )));
    }

    if let Some(error) = first_error {
        return Err(error);
    }

    Ok(None)
}

#[derive(Debug)]
struct CalendarEvent {
    start: DateTime<Utc>,
    end: Option<DateTime<Utc>>,
    title: String,
}

impl CalendarEvent {
    fn is_active_at(&self, now: DateTime<Utc>) -> bool {
        self.start <= now && self.end.is_some_and(|end| now < end)
    }

    fn display_time(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        if self.is_active_at(now) {
            now
        } else {
            self.start
        }
    }
}

fn parse_next_event(ics: &str, now: DateTime<Utc>) -> Option<CalendarEvent> {
    let mut current_start: Option<DateTime<Utc>> = None;
    let mut current_end: Option<DateTime<Utc>> = None;
    let mut current_is_all_day = false;
    let mut current_title: Option<String> = None;
    let mut current_rrule: Option<String> = None;
    let mut current_exdates: Vec<DateTime<Utc>> = Vec::new();
    let mut events = Vec::new();

    for line in unfold_ical_lines(ics) {
        match line.as_str() {
            "BEGIN:VEVENT" => {
                current_start = None;
                current_end = None;
                current_is_all_day = false;
                current_title = None;
                current_rrule = None;
                current_exdates.clear();
            }
            "END:VEVENT" => {
                if let Some(start) = current_start.take() {
                    let end = current_end
                        .take()
                        .or_else(|| current_is_all_day.then_some(start + Duration::days(1)));
                    let title = current_title
                        .take()
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or_else(|| "予定あり".to_string());
                    if let Some(rrule) = current_rrule.take() {
                        events.extend(expand_recurring_event(
                            start,
                            end,
                            title,
                            &rrule,
                            &current_exdates,
                            now,
                        ));
                    } else {
                        events.push(CalendarEvent { start, end, title });
                    }
                }
            }
            _ => {
                if line.starts_with("DTSTART") {
                    current_is_all_day = line.contains("VALUE=DATE");
                    if let Some((_, raw_value)) = line.split_once(':') {
                        if raw_value.len() == 8 {
                            current_is_all_day = true;
                        }
                        current_start = parse_datetime(raw_value);
                    }
                } else if line.starts_with("DTEND") {
                    if let Some((_, raw_value)) = line.split_once(':') {
                        current_end = parse_datetime(raw_value);
                    }
                } else if line.starts_with("RRULE") {
                    if let Some((_, raw_value)) = line.split_once(':') {
                        current_rrule = Some(raw_value.to_string());
                    }
                } else if line.starts_with("EXDATE") {
                    if let Some((_, raw_values)) = line.split_once(':') {
                        current_exdates.extend(
                            raw_values
                                .split(',')
                                .filter_map(|value| parse_datetime(value.trim())),
                        );
                    }
                } else if line.starts_with("SUMMARY") {
                    if let Some((_, summary)) = line.split_once(':') {
                        current_title = Some(summary.to_string());
                    }
                }
            }
        }
    }

    events
        .into_iter()
        .filter(|event| event.start >= now || event.is_active_at(now))
        .min_by_key(|event| event.display_time(now))
}

fn expand_recurring_event(
    start: DateTime<Utc>,
    end: Option<DateTime<Utc>>,
    title: String,
    rrule: &str,
    exdates: &[DateTime<Utc>],
    now: DateTime<Utc>,
) -> Vec<CalendarEvent> {
    let start_tz = to_tz_datetime(start);
    let until = to_tz_datetime(now + Duration::days(30));
    let exdate_set: HashSet<DateTime<Utc>> = exdates.iter().copied().collect();
    let base_duration = end.map(|end_at| end_at - start);
    let mut events = Vec::new();

    let parsed_rule = match RRule::from_str(rrule)
        .and_then(|rule| rule.validate(start_tz))
    {
        Ok(rule) => rule,
        Err(_) => return events,
    };
    let rule_set = RRuleSet::new(start_tz)
        .rrule(parsed_rule)
        .set_exdates(exdates.iter().copied().map(to_tz_datetime).collect())
        .before(until);

    for occurrence in rule_set.all_unchecked() {
        let start_at = to_utc_datetime(occurrence);
        if exdate_set.contains(&start_at) {
            continue;
        }
        let end_at = base_duration.map(|duration| start_at + duration);
        events.push(CalendarEvent {
            start: start_at,
            end: end_at,
            title: title.clone(),
        });
    }

    events
}

fn to_tz_datetime(value: DateTime<Utc>) -> chrono::DateTime<Tz> {
    value.with_timezone(&Tz::UTC)
}

fn to_utc_datetime(value: chrono::DateTime<Tz>) -> DateTime<Utc> {
    value.with_timezone(&Utc)
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

fn format_title(
    config: &AppConfig,
    title: &str,
    start: DateTime<Utc>,
    now: DateTime<Utc>,
) -> String {
    let seconds = (start - now).num_seconds().max(0);
    let minutes_until = (seconds + 59) / 60;
    let hours_until = minutes_until / 60;
    let remaining_minutes = minutes_until % 60;
    let title_value = if config.display.show_title { title } else { "" };
    let relative_time = format!("{hours_until}h{remaining_minutes:02}m");

    config
        .display
        .normal_format
        .replace("{relative_time}", &relative_time)
        .replace("{hh}", &hours_until.to_string())
        .replace("{mm}", &format!("{remaining_minutes:02}"))
        .replace("{h}", &hours_until.to_string())
        .replace("{m}", &remaining_minutes.to_string())
        .replace("{minutes_until}", &minutes_until.to_string())
        .replace("{title}", title_value)
        .trim()
        .to_string()
}
