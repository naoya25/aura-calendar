use chrono::{DateTime, Utc};
use minijinja::{context, Environment};

const SINGLE_LIMIT: usize = 20;
const SINGLE_KEEP: usize = 16;
const MULTI_LIMIT: usize = 9;
const MULTI_KEEP: usize = 6;

fn hw_width(c: char) -> usize {
    // Full-width CJK and similar blocks count as 2.
    match c {
        '\u{1100}'..='\u{115F}'   // Hangul Jamo
        | '\u{2E80}'..='\u{303E}' // CJK Radicals / Kangxi
        | '\u{3041}'..='\u{33BF}' // Hiragana, Katakana, Bopomofo, CJK Compatibility
        | '\u{33FF}'..='\u{A4CF}' // Various CJK
        | '\u{AC00}'..='\u{D7AF}' // Hangul Syllables
        | '\u{F900}'..='\u{FAFF}' // CJK Compatibility Ideographs
        | '\u{FE10}'..='\u{FE19}' // Vertical forms
        | '\u{FE30}'..='\u{FE6F}' // CJK Compatibility Forms
        | '\u{FF00}'..='\u{FF60}' // Fullwidth Forms
        | '\u{FFE0}'..='\u{FFE6}' // Fullwidth Signs
        | '\u{20000}'..='\u{2FFFD}' // CJK Unified Ideographs Extension B-F
        | '\u{30000}'..='\u{3FFFD}' => 2,
        _ => 1,
    }
}

fn hw_len(s: &str) -> usize {
    s.chars().map(hw_width).sum()
}

fn truncate_hw(s: &str, limit: usize, keep: usize) -> String {
    if hw_len(s) <= limit {
        return s.to_string();
    }
    let mut acc = 0;
    let mut cut = s.len();
    for (i, c) in s.char_indices() {
        if acc + hw_width(c) > keep {
            cut = i;
            break;
        }
        acc += hw_width(c);
    }
    format!("{}...", &s[..cut])
}

pub struct FormatContext {
    pub d: i64,
    pub h: i64,
    pub hh: String,
    pub m: i64,
    pub s: i64,
    pub ss: String,
    pub mm: String,
    pub total_minutes: i64,
    pub title: String,
    pub active: bool,
    pub count: i64,
}

/// タイトルを半角換算で切り詰める。単一予定と複数予定で上限が異なる。
pub fn truncate_title(title: &str, is_multi: bool) -> String {
    if is_multi {
        truncate_hw(title, MULTI_LIMIT, MULTI_KEEP)
    } else {
        truncate_hw(title, SINGLE_LIMIT, SINGLE_KEEP)
    }
}

pub fn build_context(
    title: String,
    display_time: DateTime<Utc>,
    now: DateTime<Utc>,
    active: bool,
    count: usize,
) -> FormatContext {
    let seconds = (display_time - now).num_seconds().max(0);
    let total_minutes = (seconds + 59) / 60;

    let d = seconds / (24 * 60 * 60);
    let mut rem = seconds - d * 24 * 60 * 60;
    let h = rem / 3600;
    rem = rem - h * 3600;
    let m = rem / 60;
    let s = rem % 60;

    FormatContext {
        d,
        hh: format!("{h:02}"),
        h,
        mm: format!("{m:02}"),
        s,
        ss: format!("{s:02}"),
        m,
        total_minutes,
        title,
        active,
        count: count as i64,
    }
}

fn make_context(ctx: &FormatContext) -> minijinja::Value {
    context! {
        d => ctx.d,
        h => ctx.h,
        hh => ctx.hh,
        s => ctx.s,
        ss => ctx.ss,
        m => ctx.m,
        mm => ctx.mm,
        total_minutes => ctx.total_minutes,
        title => ctx.title,
        active => ctx.active,
        count => ctx.count,
    }
}

fn create_env() -> Environment<'static> {
    let mut env = Environment::new();
    // 設定画面では複数行テンプレートの入力を許可するため、
    // 制御構文由来の不要な改行・インデントを描画時に抑制する。
    env.set_trim_blocks(true);
    env.set_lstrip_blocks(true);
    env
}

fn normalize_single_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn render(template: &str, ctx: FormatContext) -> String {
    let env = create_env();
    match env.render_str(template, make_context(&ctx)) {
        Ok(s) => normalize_single_line(&s),
        Err(e) => {
            eprintln!("format template error: {e}");
            normalize_single_line(&ctx.title)
        }
    }
}

/// 設定画面のプレビュー用。サンプルデータでテンプレートを描画して返す。
pub fn preview(template: &str) -> Result<String, String> {
    let env = create_env();
    env.render_str(
        template,
        context! {
            d => 0_i64,
            h => 1_i64,
            hh => "01",
            m => 30_i64,
            mm => "30",
            s => 5_i64,
            ss => "05",
            total_minutes => 90_i64,
            title => "チームMTG",
            active => false,
            count => 1_i64,
        },
    )
    .map(|s| normalize_single_line(&s))
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn render_should_normalize_multiline_template_to_single_line() {
        let template = r#"
{% if active %}
    now
{% else %}
    {% if d > 0 %}
        {{ d }}:
    {% endif %}

    {% if d > 0 or h > 0 %}
        {{ hh }}:
    {% endif %}

    {% if d > 0 or h > 0 or m > 0 %}
        {{ mm }}
    {% endif %}
{% endif %}

> {{ title }}
{% if count > 1 %}
    ({{ count }})
{% endif %}
"#;

        let now = Utc::now();
        let ctx = build_context(
            "チームMTG".to_string(),
            now + Duration::hours(1),
            now,
            false,
            1,
        );
        let rendered = render(template, ctx);

        assert!(!rendered.contains('\n'));
        assert!(!rendered.contains('\r'));
        assert!(rendered.contains("チームMTG"));
    }

    #[test]
    fn preview_should_normalize_multiline_template_to_single_line() {
        let template = "{% if d > 0 %}{{ d }}:{% endif %}\n{{ hh }}:{{ mm }}\n> {{ title }}";
        let rendered = preview(template).expect("preview should succeed");

        assert!(!rendered.contains('\n'));
        assert!(!rendered.contains('\r'));
        assert!(rendered.contains("チームMTG"));
    }
}
