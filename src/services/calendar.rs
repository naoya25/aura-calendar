use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use rrule::{RRule, RRuleSet, Tz};
use std::collections::HashSet;
use std::str::FromStr;
use thiserror::Error;

use crate::config::AppConfig;

const REQUEST_TIMEOUT_SECONDS: u64 = 10;
const USER_AGENT: &str = "aura-calendar/0.1";

pub async fn fetch_next_title(config: &AppConfig) -> Result<Option<String>, CalendarError> {
    if config.calendars.is_empty() {
        return Ok(None);
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
        .user_agent(USER_AGENT)
        .build()?;
    let now = Utc::now();
    let mut first_error: Option<reqwest::Error> = None;
    let mut best_event: Option<CalendarEvent> = None;

    for calendar in &config.calendars {
        let body = match fetch_calendar_body(&client, &calendar.ical_url).await {
            Ok(body) => body,
            Err(e) => {
                first_error.get_or_insert(e);
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
        return Err(CalendarError::Request(error));
    }

    Ok(None)
}

async fn fetch_calendar_body(
    client: &reqwest::Client,
    url: &str,
) -> Result<String, reqwest::Error> {
    client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await
}

#[derive(Debug, Error)]
pub enum CalendarError {
    #[error("failed to build HTTP client: {0}")]
    ClientBuild(#[from] reqwest::Error),
    #[error("calendar request failed: {0}")]
    Request(reqwest::Error),
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

    let parsed_rule = match RRule::from_str(rrule).and_then(|rule| rule.validate(start_tz)) {
        Ok(rule) => rule,
        Err(e) => {
            eprintln!("RRULE parse failed ({rrule}): {e}");
            return events;
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixed_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 1, 12, 0, 0).unwrap()
    }

    fn make_ics(events: &str) -> String {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//Test//EN\r\n{}END:VCALENDAR\r\n",
            events
        )
    }

    fn make_event(dtstart: &str, dtend: &str, summary: &str) -> String {
        format!(
            "BEGIN:VEVENT\r\nDTSTART:{dtstart}\r\nDTEND:{dtend}\r\nSUMMARY:{summary}\r\nEND:VEVENT\r\n"
        )
    }

    // --- unfold_ical_lines ---

    #[test]
    fn unfold_preserves_simple_lines() {
        let result = unfold_ical_lines("LINE1\r\nLINE2\r\n");
        assert_eq!(result, vec!["LINE1", "LINE2"]);
    }

    #[test]
    fn unfold_joins_line_folded_with_space() {
        // iCal の折り返し: 次の行が空白で始まる場合は前の行の続き
        let result = unfold_ical_lines("PROP:val\r\n ue\r\n");
        assert_eq!(result, vec!["PROP:value"]);
    }

    #[test]
    fn unfold_joins_line_folded_with_tab() {
        let result = unfold_ical_lines("PROP:val\r\n\tcontinued\r\n");
        assert_eq!(result, vec!["PROP:valcontinued"]);
    }

    // --- parse_datetime ---

    #[test]
    fn parse_datetime_utc_format() {
        let result = parse_datetime("20260501T150000Z");
        let expected = Utc.with_ymd_and_hms(2026, 5, 1, 15, 0, 0).unwrap();
        assert_eq!(result, Some(expected));
    }

    #[test]
    fn parse_datetime_local_format_returns_some() {
        // ローカルタイムゾーン依存なので Some() であることだけ確認
        assert!(parse_datetime("20260501T150000").is_some());
    }

    #[test]
    fn parse_datetime_date_only_returns_some() {
        assert!(parse_datetime("20260501").is_some());
    }

    #[test]
    fn parse_datetime_invalid_returns_none() {
        assert_eq!(parse_datetime("invalid"), None);
        assert_eq!(parse_datetime(""), None);
        assert_eq!(parse_datetime("20260501T"), None);
    }

    // --- parse_next_event ---

    #[test]
    fn parse_next_event_returns_none_for_empty_calendar() {
        let ics = make_ics("");
        assert!(parse_next_event(&ics, fixed_now()).is_none());
    }

    #[test]
    fn parse_next_event_returns_future_event() {
        let ics = make_ics(&make_event("20260501T150000Z", "20260501T160000Z", "Team Meeting"));
        let event = parse_next_event(&ics, fixed_now()).unwrap();
        assert_eq!(event.title, "Team Meeting");
        assert_eq!(
            event.start,
            Utc.with_ymd_and_hms(2026, 5, 1, 15, 0, 0).unwrap()
        );
    }

    #[test]
    fn parse_next_event_ignores_past_event() {
        // now = 12:00, イベントは 09:00-10:00（過去）→ 返さない
        let ics = make_ics(&make_event("20260501T090000Z", "20260501T100000Z", "Past Event"));
        assert!(parse_next_event(&ics, fixed_now()).is_none());
    }

    #[test]
    fn parse_next_event_returns_currently_active_event() {
        // now = 12:00, イベントは 11:00-13:00（進行中）→ 返す
        let ics = make_ics(&make_event(
            "20260501T110000Z",
            "20260501T130000Z",
            "Active Meeting",
        ));
        let event = parse_next_event(&ics, fixed_now()).unwrap();
        assert_eq!(event.title, "Active Meeting");
    }

    #[test]
    fn parse_next_event_returns_nearest_when_multiple() {
        // 2つある場合は近い方を返す
        let events = format!(
            "{}{}",
            make_event("20260501T160000Z", "20260501T170000Z", "Later"),
            make_event("20260501T150000Z", "20260501T160000Z", "Earlier"),
        );
        let event = parse_next_event(&make_ics(&events), fixed_now()).unwrap();
        assert_eq!(event.title, "Earlier");
    }

    #[test]
    fn parse_next_event_defaults_empty_title_to_yotei_ari() {
        let ics = make_ics(&make_event("20260501T150000Z", "20260501T160000Z", ""));
        let event = parse_next_event(&ics, fixed_now()).unwrap();
        assert_eq!(event.title, "予定あり");
    }

    #[test]
    fn parse_next_event_handles_all_day_event() {
        // VALUE=DATE 形式（終日イベント）
        let ics = make_ics(
            "BEGIN:VEVENT\r\nDTSTART;VALUE=DATE:20260502\r\nSUMMARY:All Day\r\nEND:VEVENT\r\n",
        );
        let event = parse_next_event(&ics, fixed_now());
        assert!(event.is_some());
        assert_eq!(event.unwrap().title, "All Day");
    }

    // --- format_title ---

    #[test]
    fn format_title_90_minutes_away() {
        // デフォルトフォーマット: "{minutes_until}分後 {title}"
        let config = crate::config::AppConfig::default();
        let now = fixed_now();
        let start = now + Duration::minutes(90);
        assert_eq!(format_title(&config, "MTG", start, now), "90分後 MTG");
    }

    #[test]
    fn format_title_active_event_shows_zero() {
        let config = crate::config::AppConfig::default();
        let now = fixed_now();
        // display_time = now なので seconds = 0 → "0分後 MTG"
        assert_eq!(format_title(&config, "MTG", now, now), "0分後 MTG");
    }

    #[test]
    fn format_title_hides_title_when_show_title_false() {
        let mut config = crate::config::AppConfig::default();
        config.display.show_title = false;
        let now = fixed_now();
        let start = now + Duration::minutes(30);
        // "30分後 " → trim → "30分後"
        assert_eq!(format_title(&config, "Secret", start, now), "30分後");
    }
}
