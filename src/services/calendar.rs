use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use rrule::{RRule, RRuleSet, Tz};
use std::collections::HashSet;
use std::str::FromStr;
use thiserror::Error;

use crate::config::AppConfig;

const REQUEST_TIMEOUT_SECONDS: u64 = 10;
const USER_AGENT: &str = "aura-calendar/0.1";

pub struct FetchResult {
    pub tray_events: Option<Vec<CachedEvent>>,
    pub schedule_events: Vec<CachedEvent>,
}

/// カレンダーデータを HTTP 取得・パースし、トレイ用イベントグループと3日分の全予定を返す（長周期で呼ぶ）。
pub async fn fetch(config: &AppConfig) -> Result<FetchResult, CalendarError> {
    if config.calendars.is_empty() {
        return Ok(FetchResult { tray_events: None, schedule_events: Vec::new() });
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
        .user_agent(USER_AGENT)
        .build()?;
    let now = Utc::now();
    let schedule_limit = now + Duration::days(3);
    let mut first_error: Option<reqwest::Error> = None;
    let mut best_display_time: Option<DateTime<Utc>> = None;
    let mut best_events: Vec<CachedEvent> = Vec::new();
    let mut all_schedule: Vec<CachedEvent> = Vec::new();

    for calendar in &config.calendars {
        let body = match fetch_calendar_body(&client, &calendar.ical_url).await {
            Ok(body) => body,
            Err(e) => {
                first_error.get_or_insert(e);
                continue;
            }
        };

        let concurrent = parse_concurrent_events(&body, now);
        if !concurrent.is_empty() {
            let dt = concurrent[0].display_time(now);
            match best_display_time {
                None => {
                    best_display_time = Some(dt);
                    best_events = concurrent;
                }
                Some(best_dt) if dt < best_dt => {
                    best_display_time = Some(dt);
                    best_events = concurrent;
                }
                Some(best_dt) if dt == best_dt => {
                    best_events.extend(concurrent);
                }
                _ => {}
            }
        }

        let schedule: Vec<CachedEvent> = collect_relevant_events(&body, now)
            .into_iter()
            .filter(|e| e.start < schedule_limit)
            .collect();
        all_schedule.extend(schedule);
    }

    all_schedule.sort_by_key(|e| e.start);

    if !best_events.is_empty() {
        return Ok(FetchResult { tray_events: Some(best_events), schedule_events: all_schedule });
    }

    if let Some(error) = first_error {
        return Err(CalendarError::Request(error));
    }

    Ok(FetchResult { tray_events: None, schedule_events: all_schedule })
}

/// キャッシュ済みイベントと現在時刻からタイトルを生成する（短周期で呼ぶ、IO なし）。
pub fn render_title(config: &AppConfig, events: &[CachedEvent], now: DateTime<Utc>) -> String {
    format_title_group(config, events, now)
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

#[derive(Debug, Clone)]
pub struct CachedEvent {
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
    pub title: String,
}

impl CachedEvent {
    pub fn is_active_at(&self, now: DateTime<Utc>) -> bool {
        self.start <= now && self.end.is_some_and(|end| now < end)
    }

    pub fn display_time(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        if self.is_active_at(now) {
            now
        } else {
            self.start
        }
    }
}

fn parse_concurrent_events(ics: &str, now: DateTime<Utc>) -> Vec<CachedEvent> {
    let relevant = collect_relevant_events(ics, now);

    // 未開始の予定を優先し、なければ進行中の予定を使う。
    // 長い予定の中に別の予定が入っている場合に次の予定を常に表示するため。
    let (future, active): (Vec<_>, Vec<_>) = relevant.into_iter().partition(|e| e.start > now);
    let candidates = if !future.is_empty() { future } else { active };

    let min_dt = candidates.iter().map(|e| e.display_time(now)).min();
    match min_dt {
        None => vec![],
        Some(dt) => candidates
            .into_iter()
            .filter(|e| e.display_time(now) == dt)
            .collect(),
    }
}

#[cfg(test)]
fn parse_next_event(ics: &str, now: DateTime<Utc>) -> Option<CachedEvent> {
    parse_concurrent_events(ics, now).into_iter().next()
}

fn collect_relevant_events(ics: &str, now: DateTime<Utc>) -> Vec<CachedEvent> {
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
                        events.push(CachedEvent { start, end, title });
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
        .collect()
}

fn expand_recurring_event(
    start: DateTime<Utc>,
    end: Option<DateTime<Utc>>,
    title: String,
    rrule: &str,
    exdates: &[DateTime<Utc>],
    now: DateTime<Utc>,
) -> Vec<CachedEvent> {
    let start_tz = to_tz_datetime(start);
    let until = to_tz_datetime(now + Duration::days(30));
    let exdate_set: HashSet<DateTime<Utc>> = exdates.iter().copied().collect();
    let base_duration = end.map(|end_at| end_at - start);
    let mut events = Vec::new();

    let normalized_rrule = normalize_rrule_until(rrule);
    // UNTIL < DTSTART はカレンダー側のデータ不整合。既に終了済みなので静かにスキップする。
    if rrule_until_before_start(&normalized_rrule, start) {
        return events;
    }
    let parsed_rule = match RRule::from_str(&normalized_rrule).and_then(|rule| rule.validate(start_tz)) {
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
        events.push(CachedEvent {
            start: start_at,
            end: end_at,
            title: title.clone(),
        });
    }

    events
}

/// UNTIL が DTSTART より前かどうかを確認する。
/// 正規化済み RRULE（UNTIL は UTC 形式）を前提とする。
fn rrule_until_before_start(rrule: &str, start: DateTime<Utc>) -> bool {
    const PREFIX: &str = "UNTIL=";
    let Some(pos) = rrule.find(PREFIX) else { return false };
    let value = &rrule[pos + PREFIX.len()..];
    let end = value.find(';').unwrap_or(value.len());
    let until_str = &value[..end];
    NaiveDateTime::parse_from_str(until_str, "%Y%m%dT%H%M%SZ")
        .map(|naive| DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc) < start)
        .unwrap_or(false)
}

/// `UNTIL=YYYYMMDD`（日付のみ）を `UNTIL=YYYYMMDDTXXXXXXZ`（UTC）に正規化する。
/// Google カレンダーなどが UTC の DTSTART と日付のみの UNTIL を組み合わせた
/// RFC 違反の RRULE を出力することがあるため、パース前に補正する。
fn normalize_rrule_until(rrule: &str) -> String {
    const PREFIX: &str = "UNTIL=";
    let Some(until_pos) = rrule.find(PREFIX) else {
        return rrule.to_string();
    };
    let value_start = until_pos + PREFIX.len();
    let value = &rrule[value_start..];

    // 日付のみ形式: 8桁の数字の後が ';'・末尾・スペースのどれか
    let is_date_only = value.len() >= 8
        && value[..8].chars().all(|c| c.is_ascii_digit())
        && matches!(value.as_bytes().get(8), None | Some(b';') | Some(b' '));

    if !is_date_only {
        return rrule.to_string();
    }

    let date = &value[..8];
    let rest = &value[8..];
    format!("{}{}{}T235959Z{}", &rrule[..until_pos], PREFIX, date, rest)
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

fn format_title_group(config: &AppConfig, events: &[CachedEvent], now: DateTime<Utc>) -> String {
    if events.len() == 1 {
        let e = &events[0];
        return format_title(config, &e.title, e.display_time(now), now);
    }

    let display_time = events[0].display_time(now);
    let active = display_time == now;
    let title = if config.display.show_title {
        events
            .iter()
            .map(|e| crate::format::truncate_title(&e.title, true))
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        String::new()
    };
    let ctx = crate::format::build_context(title, display_time, now, active, events.len());
    crate::format::render(&config.display.normal_format, ctx)
}

fn format_title(
    config: &AppConfig,
    title: &str,
    display_time: DateTime<Utc>,
    now: DateTime<Utc>,
) -> String {
    let title_value = if config.display.show_title {
        crate::format::truncate_title(title, false)
    } else {
        String::new()
    };
    let active = display_time == now;
    let ctx = crate::format::build_context(title_value, display_time, now, active, 1);
    crate::format::render(&config.display.normal_format, ctx)
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

    // --- normalize_rrule_until ---

    #[test]
    fn normalize_rrule_until_converts_date_only() {
        let input = "FREQ=WEEKLY;UNTIL=20211128";
        assert_eq!(normalize_rrule_until(input), "FREQ=WEEKLY;UNTIL=20211128T235959Z");
    }

    #[test]
    fn normalize_rrule_until_converts_date_only_with_trailing_params() {
        let input = "FREQ=MONTHLY;UNTIL=20230614;BYMONTHDAY=15";
        assert_eq!(normalize_rrule_until(input), "FREQ=MONTHLY;UNTIL=20230614T235959Z;BYMONTHDAY=15");
    }

    #[test]
    fn normalize_rrule_until_leaves_utc_datetime_unchanged() {
        let input = "FREQ=WEEKLY;UNTIL=20211227T145959Z";
        assert_eq!(normalize_rrule_until(input), input);
    }

    #[test]
    fn normalize_rrule_until_leaves_no_until_unchanged() {
        let input = "FREQ=DAILY;COUNT=10";
        assert_eq!(normalize_rrule_until(input), input);
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
        let config = crate::config::AppConfig::default();
        let now = fixed_now();
        let start = now + Duration::minutes(90);
        assert_eq!(format_title(&config, "MTG", start, now), "1h30m|MTG");
    }

    #[test]
    fn format_title_active_event_shows_zero() {
        let config = crate::config::AppConfig::default();
        let now = fixed_now();
        // d=0, h=0, m=0 → "now"
        assert_eq!(format_title(&config, "MTG", now, now), "now|MTG");
    }

    #[test]
    fn format_title_hides_title_when_show_title_false() {
        let mut config = crate::config::AppConfig::default();
        config.display.show_title = false;
        let now = fixed_now();
        let start = now + Duration::minutes(30);
        assert_eq!(format_title(&config, "Secret", start, now), "30m|");
    }

    // --- parse_concurrent_events ---

    #[test]
    fn concurrent_events_future_preferred_over_active() {
        // 長い予定（進行中）の中に別の予定（将来）がある → 将来の予定を優先して返す
        let events = format!(
            "{}{}",
            make_event("20260501T090000Z", "20260501T180000Z", "終日MTG"),
            make_event("20260501T140000Z", "20260501T143000Z", "短いMTG"),
        );
        let now = fixed_now(); // 12:00 UTC
        let concurrent = parse_concurrent_events(&make_ics(&events), now);
        assert_eq!(concurrent.len(), 1);
        assert_eq!(concurrent[0].title, "短いMTG");
    }

    #[test]
    fn concurrent_events_active_shown_when_no_future() {
        // 将来の予定がない場合は進行中の予定を表示する
        let events = make_event("20260501T090000Z", "20260501T180000Z", "終日MTG");
        let now = fixed_now(); // 12:00 UTC
        let concurrent = parse_concurrent_events(&make_ics(&events), now);
        assert_eq!(concurrent.len(), 1);
        assert_eq!(concurrent[0].title, "終日MTG");
    }

    #[test]
    fn concurrent_events_same_start_time_are_grouped() {
        // 同時刻に始まる2つの将来予定はグループ化される
        let events = format!(
            "{}{}",
            make_event("20260501T150000Z", "20260501T160000Z", "MTG-A"),
            make_event("20260501T150000Z", "20260501T153000Z", "MTG-B"),
        );
        let now = fixed_now(); // 12:00 UTC
        let concurrent = parse_concurrent_events(&make_ics(&events), now);
        assert_eq!(concurrent.len(), 2);
    }

    #[test]
    fn concurrent_events_multiple_active_are_grouped() {
        // 両方進行中（display_time = now）→ グループ化される
        let events = format!(
            "{}{}",
            make_event("20260501T110000Z", "20260501T130000Z", "長いMTG"),
            make_event("20260501T113000Z", "20260501T120000Z", "短いMTG"), // 11:30-12:00, now=12:00 → 終了直前で進行中
        );
        let now = Utc.with_ymd_and_hms(2026, 5, 1, 11, 45, 0).unwrap();
        let concurrent = parse_concurrent_events(&make_ics(&events), now);
        assert_eq!(concurrent.len(), 2);
    }

    // --- format_title_group ---

    #[test]
    fn format_title_group_single_event_uses_normal_format() {
        let config = crate::config::AppConfig::default();
        let now = fixed_now();
        let events = vec![CachedEvent {
            start: now + Duration::minutes(30),
            end: Some(now + Duration::minutes(60)),
            title: "MTG".to_string(),
        }];
        let result = format_title_group(&config, &events, now);
        assert_eq!(result, "30m|MTG");
    }

    #[test]
    fn format_title_group_multiple_future_events_shows_count() {
        let config = crate::config::AppConfig::default();
        let now = fixed_now();
        let start = now + Duration::minutes(15);
        let events = vec![
            CachedEvent { start, end: Some(start + Duration::hours(1)), title: "MTG-A".to_string() },
            CachedEvent { start, end: Some(start + Duration::minutes(30)), title: "MTG-B".to_string() },
        ];
        let result = format_title_group(&config, &events, now);
        // 各タイトルを個別に切り詰め: "MTG-A"(5)≤9, "MTG-B"(5)≤9 → そのまま結合
        assert_eq!(result, "15m|MTG-A, MTG-B :2");
    }

    #[test]
    fn format_title_group_multiple_active_events_shows_zero_minutes() {
        let config = crate::config::AppConfig::default();
        let now = fixed_now();
        let events = vec![
            CachedEvent { start: now - Duration::hours(1), end: Some(now + Duration::hours(1)), title: "終日MTG".to_string() },
            CachedEvent { start: now - Duration::minutes(10), end: Some(now + Duration::minutes(20)), title: "朝会".to_string() },
        ];
        let result = format_title_group(&config, &events, now);
        // 各タイトルを個別に切り詰め: "終日MTG"(7)≤9, "朝会"(4)≤9 → そのまま結合
        assert_eq!(result, "now|終日MTG, 朝会 :2");
    }
}
