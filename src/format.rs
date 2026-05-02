use chrono::{DateTime, Utc};
use minijinja::{context, Environment};

pub struct FormatContext {
    pub d: i64,
    pub h: i64,
    pub m: i64,
    pub mm: String,
    pub total_minutes: i64,
    pub title: String,
    pub active: bool,
    pub count: i64,
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
    let d = total_minutes / (24 * 60);
    let remaining = total_minutes - d * 24 * 60;
    let h = remaining / 60;
    let m = remaining % 60;

    FormatContext {
        d,
        h,
        m,
        mm: format!("{m:02}"),
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
        m => ctx.m,
        mm => ctx.mm,
        total_minutes => ctx.total_minutes,
        title => ctx.title,
        active => ctx.active,
        count => ctx.count,
    }
}

pub fn render(template: &str, ctx: FormatContext) -> String {
    let env = Environment::new();
    match env.render_str(template, make_context(&ctx)) {
        Ok(s) => s.trim().to_string(),
        Err(e) => {
            eprintln!("format template error: {e}");
            ctx.title
        }
    }
}

/// 設定画面のプレビュー用。サンプルデータでテンプレートを描画して返す。
pub fn preview(template: &str) -> Result<String, String> {
    let env = Environment::new();
    env.render_str(
        template,
        context! {
            d => 0_i64,
            h => 1_i64,
            m => 30_i64,
            mm => "30",
            total_minutes => 90_i64,
            title => "チームMTG",
            active => false,
            count => 1_i64,
        },
    )
    .map(|s| s.trim().to_string())
    .map_err(|e| e.to_string())
}
