use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, RwLock,
};
use std::time::Instant;

use chrono::{Datelike, Duration, Local, Timelike, Utc};
use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{App, Manager};
use tauri_plugin_global_shortcut::GlobalShortcutExt;
use tokio::sync::broadcast;

use crate::config::AppConfig;
use crate::services::calendar::{fetch, render_title, CachedEvent};
use crate::ui::icon::menu_bar_icon;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use urlencoding::decode;

use super::commands::{self, ConfigState, RefreshSignal};

pub struct ShutdownHandle(pub broadcast::Sender<()>);

pub struct AllowExit(pub Arc<AtomicBool>);

pub struct StealthState {
    pub is_hidden: Arc<AtomicBool>,
}

/// トレイメニューが現在開いている（NSMenu がトラッキング中）かどうか。
///
/// 実測の結果、`TrayIcon::show_menu()`（内部で `performClick` を呼ぶ）は
/// グローバルショートカット経由で呼んだ場合ブロックしない（呼び出しはすぐ
/// 返り、メニューはそれとは無関係に表示され続ける）ことが分かった。
/// そのため「呼び出しがブロック中かどうか」を自前のフラグで推測するのではなく、
/// NSMenuDidBeginTrackingNotification / NSMenuDidEndTrackingNotification
/// （NSNotificationCenter 経由）を購読し、AppKit が実際に報告する開閉状態を
/// そのまま真偽値として保持する。ESC・外クリックによる dismiss でも
/// DidEndTracking は必ず飛ぶため、クリックイベントの Down/Up を見ていた
/// 旧実装より正確に状態を追跡できる。
pub struct TrayMenuOpenState(pub AtomicBool);

pub struct TrayPresentationState {
    pub render_lock_until: Mutex<Option<Instant>>,
    pub pending_schedule: Mutex<Option<Vec<CachedEvent>>>,
}

pub struct CachedSchedule(pub Mutex<Option<Vec<CachedEvent>>>);

pub struct CachedEvents(pub Mutex<Option<Vec<CachedEvent>>>);

fn is_tray_render_locked(app: &tauri::AppHandle) -> bool {
    let state = app.state::<TrayPresentationState>();
    let lock_result = state.render_lock_until.lock();
    let lock_until = match lock_result {
        Ok(guard) => guard,
        Err(_) => return false,
    };

    (*lock_until).is_some()
}

fn lock_tray_render(app: &tauri::AppHandle) {
    let state = app.state::<TrayPresentationState>();
    let lock_result = state.render_lock_until.lock();
    if let Ok(mut lock_until) = lock_result {
        *lock_until = Some(Instant::now());
    }
}

fn clear_tray_render_lock(app: &tauri::AppHandle) {
    let state = app.state::<TrayPresentationState>();
    let lock_result = state.render_lock_until.lock();
    if let Ok(mut lock_until) = lock_result {
        *lock_until = None;
    }
}

fn store_pending_schedule(app: &tauri::AppHandle, schedule: Vec<CachedEvent>) {
    let state = app.state::<TrayPresentationState>();
    let lock_result = state.pending_schedule.lock();
    if let Ok(mut pending_schedule) = lock_result {
        *pending_schedule = Some(schedule);
    }
}

fn take_pending_schedule(app: &tauri::AppHandle) -> Option<Vec<CachedEvent>> {
    let state = app.state::<TrayPresentationState>();
    let lock_result = state.pending_schedule.lock();
    lock_result.ok().and_then(|mut pending| pending.take())
}

fn store_cached_schedule(app: &tauri::AppHandle, schedule: Vec<CachedEvent>) {
    let state = app.state::<CachedSchedule>();
    let lock_result = state.0.lock();
    if let Ok(mut cached_schedule) = lock_result {
        *cached_schedule = Some(schedule);
    }
}

fn cached_schedule(app: &tauri::AppHandle) -> Option<Vec<CachedEvent>> {
    let state = app.state::<CachedSchedule>();
    state.0.lock().ok().and_then(|guard| guard.clone())
}

fn current_tray_title(app: &tauri::AppHandle) -> String {
    let config = app.state::<ConfigState>();
    let config = config.0.read().map(|g| g.clone()).unwrap_or_default();
    let now = Utc::now();
    let events = app
        .state::<CachedEvents>()
        .0
        .lock()
        .ok()
        .and_then(|guard| guard.clone());

    match events {
        Some(ref evts) => render_title(&config, evts, now),
        None => config.normal_title(),
    }
}

/// ロック確認と実際のトレイ更新（set_title / set_menu）を同一の main スレッド
/// タスクとして実行する。
///
/// 背景スレッドで `is_tray_render_locked` を確認してから `tray.set_title` 等を
/// 呼ぶと、確認から実際に main スレッドへ dispatch されるまでの間にロックの
/// 状態が変わりうる（TOCTOU）。tray-icon crate の `TrayIcon::show_menu()` /
/// `set_menu` / `set_title` はいずれも内部 `Rc<RefCell<..>>` を経由するため、
/// このロックが有効な区間（= NSMenu が実際にトラッキング中の区間。
/// `install_menu_tracking_observer` が NSMenuDidBeginTracking /
/// DidEndTracking 通知を見て張り直す）と set_title / set_menu（= borrow_mut）
/// の呼び出しが重なると "already borrowed" で panic しうる。確認自体を main
/// スレッド上・更新呼び出しの直前（同一クロージャ内、途中で他のタスクに
/// 割り込まれない）で行うことでこの隙間を無くす。
fn run_if_unlocked<F>(app: &tauri::AppHandle, action: F)
where
    F: FnOnce(&tauri::AppHandle) + Send + 'static,
{
    let app_handle = app.clone();
    if let Err(e) = app.run_on_main_thread(move || {
        if is_tray_render_locked(&app_handle) {
            return;
        }
        action(&app_handle);
    }) {
        eprintln!("failed to schedule tray update: {e}");
    }
}

fn refresh_tray_title(app: &tauri::AppHandle) {
    let stealth = app.state::<StealthState>();
    if stealth.is_hidden.load(Ordering::Relaxed) {
        return;
    }

    let title = current_tray_title(app);
    run_if_unlocked(app, move |app| {
        if let Some(tray) = app.tray_by_id("main-tray") {
            let spaced = format!(" {title}");
            if let Err(e) = tray.set_title(Some(spaced.as_str())) {
                eprintln!("failed to refresh tray title: {e}");
            }
        }
    });
}

fn apply_pending_tray_menu(app: &tauri::AppHandle) {
    if let Some(schedule) = take_pending_schedule(app) {
        rebuild_tray_menu(app, &schedule);
    }
}

pub fn toggle_stealth(app: &tauri::AppHandle) {
    let stealth = app.state::<StealthState>();
    let config = app.state::<ConfigState>();

    let next_hidden = !stealth.is_hidden.fetch_xor(true, Ordering::Relaxed);
    let title = if next_hidden {
        config
            .0
            .read()
            .map(|g| g.stealth_title().to_string())
            .unwrap_or_else(|_| "***".to_string())
    } else {
        current_tray_title(app)
    };

    // is_hidden の状態自体は既に更新済みなので、ロック中で下の更新をスキップ
    // しても、ロックが外れたタイミングの refresh_tray_title で表示に反映される。
    run_if_unlocked(app, move |app| {
        if let Some(tray) = app.tray_by_id("main-tray") {
            let spaced = format!(" {title}");
            if let Err(e) = tray.set_title(Some(spaced.as_str())) {
                eprintln!("failed to update tray title: {e}");
            }
        }
    });
}

fn open_menu_shortcut_string(app: &tauri::AppHandle) -> String {
    app.state::<ConfigState>()
        .0
        .read()
        .map(|g| g.open_menu_shortcut.clone())
        .unwrap_or_default()
}

/// メニュー最下部の「閉じる」項目。accelerator に open_menu_shortcut を
/// 割り当てることで、メニュー表示中の同キー押下がネイティブの
/// keyEquivalent として項目選択（= メニューを閉じる）になる。
///
/// メニュー表示中は Carbon がグローバルホットキーを配送しない
/// （閉じた瞬間にまとめて再生され開き直しになる）ため、表示中だけ
/// グローバル登録を外し、閉じる操作はこの keyEquivalent に担わせる。
fn close_menu_item(app: &tauri::AppHandle) -> Option<MenuItem<tauri::Wry>> {
    let shortcut = open_menu_shortcut_string(app);
    let accelerator = (!shortcut.is_empty()).then_some(shortcut);
    MenuItem::with_id(app, "close_menu", "閉じる", true, accelerator).ok()
}

pub fn register_stealth_shortcut(app: &tauri::AppHandle, shortcut: &str) -> Result<(), String> {
    app.global_shortcut()
        .register(shortcut)
        .map_err(|e| e.to_string())
}

pub fn register_open_menu_shortcut(app: &tauri::AppHandle, shortcut: &str) -> Result<(), String> {
    app.global_shortcut()
        .register(shortcut)
        .map_err(|e| e.to_string())
}

pub fn unregister_all_shortcuts(app: &tauri::AppHandle) {
    if let Err(e) = app.global_shortcut().unregister_all() {
        eprintln!("failed to unregister shortcuts: {e}");
    }
}

/// グローバルショートカットでトレイメニューの開閉をトグルする。
///
/// `TrayMenuOpenState`（NSMenuDidBeginTracking / DidEndTracking 通知で維持される
/// 実際の開閉状態）を見て、開いていれば `close_tray_menu`、閉じていれば
/// `show_menu` を呼ぶだけ。show_menu 自体がブロックするかどうかに一切依存しない。
/// クロージャを「きれいな main ループの次のターン」で実行する。
///
/// Carbon のグローバルホットキーコールバックや performClick のスタック内から
/// 同期的にトレイ操作（show_menu / set_title 等）を行うと、AppKit が正規の
/// メニュートラッキングセッションを張れない（Begin/End が空発火してゾンビ
/// メニューが残る）・tray-icon 内部 RefCell の borrow と衝突して panic する、
/// の両方が起きる。いったん別スレッドへ逃がしてから run_on_main_thread で
/// 戻すことで、必ずイベントループ経由の新しいターンで実行させる。
fn defer_on_main<F>(app: &tauri::AppHandle, action: F)
where
    F: FnOnce(&tauri::AppHandle) + Send + 'static,
{
    let app = app.clone();
    std::thread::spawn(move || {
        let inner = app.clone();
        if let Err(e) = app.run_on_main_thread(move || action(&inner)) {
            eprintln!("failed to defer tray action to main thread: {e}");
        }
    });
}

pub fn open_tray_menu(app: &tauri::AppHandle) {
    let open_state = app.state::<TrayMenuOpenState>();
    let is_open = open_state.0.load(Ordering::Acquire);
    if is_open {
        // メニュー表示中は main ループがトラッキングモードでブロックしており
        // 遅延ディスパッチは届かないため、ホットキーコールバックから直接閉じる。
        // close_tray_menu は tray-icon の RefCell borrow を保持しないので安全。
        close_tray_menu(app);
        return;
    }
    defer_on_main(app, |app| {
        let open_state = app.state::<TrayMenuOpenState>();
        if open_state.0.load(Ordering::Acquire) {
            return;
        }
        open_tray_menu_on_main(app);
    });
}

/// トレイメニューを popUpMenuPositioningItem で表示する（main スレッド・
/// きれいなスタックから呼ぶこと）。
///
/// tray-icon の `show_menu()`（NSStatusBarButton.performClick）はホットキー等の
/// 合成コンテキストから呼ぶと正規のメニュートラッキングセッションが張られず、
/// NSMenuDidBegin/EndTracking が瞬時に空発火する「ゾンビメニュー」になる
/// （表示は残るのに cancelTracking も ESC も効かない）。
/// `NSMenu.popUpMenuPositioningItem:atLocation:inView:` はプロセス内で本物の
/// トラッキングセッションを張る公開 API で、閉じるまでブロックし、
/// Begin/End 通知・cancelTracking が正しく機能する。
#[cfg(target_os = "macos")]
fn open_tray_menu_on_main(app: &tauri::AppHandle) {
    let Some(tray) = app.tray_by_id("main-tray") else {
        return;
    };

    // メニュー表示中は Carbon がグローバルホットキーを配送せず、閉じた瞬間に
    // 溜まった押下がまとめて再生されて開き直しになる。表示中はグローバル登録を
    // 外し、押下を素の keyDown としてメニューに流して「閉じる」項目の
    // keyEquivalent に処理させる（メニューが閉じたら下で再登録する）。
    let shortcut = open_menu_shortcut_string(app);
    if !shortcut.is_empty() {
        if let Err(e) = app.global_shortcut().unregister(shortcut.as_str()) {
            eprintln!("failed to suspend open-menu shortcut: {e}");
        }
    }
    // ns_status_item() は tray-icon 内部 RefCell を一時 borrow して owned な
    // Retained を返すため、以降（popUp のブロック中を含め）RefCell の borrow は
    // 保持されず、表示中の set_title（borrow_mut）と衝突しない。
    if let Err(e) = tray.with_inner_tray_icon(|inner| {
        use objc2_foundation::NSPoint;

        let Some(status_item) = inner.ns_status_item() else {
            return;
        };
        let Some(mtm) = objc2::MainThreadMarker::new() else {
            return;
        };
        let (Some(button), Some(menu)) = (status_item.button(mtm), status_item.menu(mtm)) else {
            return;
        };

        let location = NSPoint {
            x: 0.0,
            y: button.bounds().size.height + 4.0,
        };
        let view: &objc2_app_kit::NSView = &button;
        // ここでブロックする（メニューが閉じるまで戻らない）
        menu.popUpMenuPositioningItem_atLocation_inView(None, location, Some(view));
    }) {
        eprintln!("failed to open tray menu: {e}");
    }

    // メニューが閉じたのでグローバルショートカットを復帰させる
    if !shortcut.is_empty() {
        if let Err(e) = register_open_menu_shortcut(app, &shortcut) {
            eprintln!("failed to resume open-menu shortcut: {e}");
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn open_tray_menu_on_main(app: &tauri::AppHandle) {
    if let Some(tray) = app.tray_by_id("main-tray") {
        if let Err(e) = tray.with_inner_tray_icon(|inner| inner.show_menu()) {
            eprintln!("failed to open tray menu: {e}");
        }
    }
}

/// 現在表示中のトレイメニューを `NSMenu.cancelTracking()` で閉じる。
///
/// tray-icon / muda はメニューを programmatic に閉じる公開APIを持たないため、
/// 標準・公開の AppKit API である `NSMenu.cancelTracking()`
/// (https://developer.apple.com/documentation/appkit/nsmenu/canceltracking())
/// を objc2 経由で直接呼ぶ。`NSStatusItem.menu()` / `NSMenu.cancelTracking()` は
/// いずれも `self.tray.borrow()`（immutable）にしかアクセスしない
/// `TrayIcon::ns_status_item()` を経由するため、tray-icon 内部の RefCell が
/// 他所で immutable borrow されていても衝突しない
/// （RefCell は複数の immutable borrow を許容する）。
#[cfg(target_os = "macos")]
fn close_tray_menu(app: &tauri::AppHandle) {
    let Some(tray) = app.tray_by_id("main-tray") else {
        return;
    };
    if let Err(e) = tray.with_inner_tray_icon(|inner| {
        let Some(status_item) = inner.ns_status_item() else {
            return;
        };
        let Some(mtm) = objc2::MainThreadMarker::new() else {
            return;
        };
        if let Some(menu) = status_item.menu(mtm) {
            menu.cancelTracking();
        }
    }) {
        eprintln!("failed to close tray menu: {e}");
    }
}

#[cfg(not(target_os = "macos"))]
fn close_tray_menu(_app: &tauri::AppHandle) {}

/// NSMenuDidBeginTrackingNotification / NSMenuDidEndTrackingNotification を
/// NSNotificationCenter 経由で購読し、`TrayMenuOpenState` とトレイ描画ロック
/// （`TrayPresentationState`）を実際のメニュー開閉に同期させる。
///
/// object（どの NSMenu か）でのフィルタは行わず、プロセス内のあらゆる NSMenu の
/// begin/end tracking をトレイメニューの開閉として扱う。このアプリのネイティブ
/// メニューはトレイのドロップダウンのみなので通常は問題にならない
/// （設定画面は WKWebView で、テキスト欄の右クリックなど WebKit 側のネイティブ
/// context menu が万一巻き込まれても、影響はトグル状態の一時的なズレに留まり
/// クラッシュはしない）。
///
/// `rebuild_tray_menu` は呼び出すたびに新しい NSMenu インスタンスを作って
/// `set_menu` で差し替えるため、特定の NSMenu オブジェクトを覚えておいて
/// フィルタする設計は「差し替え後に古いインスタンスしか監視できていない」
/// 状態を生みやすい。object なしのグローバル購読はその問題を構造的に避けられる。
#[cfg(target_os = "macos")]
fn install_menu_tracking_observer(app: &tauri::AppHandle) {
    use objc2_app_kit::{NSMenuDidBeginTrackingNotification, NSMenuDidEndTrackingNotification};
    use objc2_foundation::NSNotificationCenter;

    let center = NSNotificationCenter::defaultCenter();

    let app_begin = app.clone();
    let begin_block = block2::RcBlock::new(
        move |_notif: std::ptr::NonNull<objc2_foundation::NSNotification>| {
            let open_state = app_begin.state::<TrayMenuOpenState>();
            open_state.0.store(true, Ordering::Release);
            lock_tray_render(&app_begin);
        },
    );
    unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(NSMenuDidBeginTrackingNotification),
            None,
            None,
            &begin_block,
        );
    }
    // NSNotificationCenter が block を内部で保持し続けるため、登録後は
    // Rust 側の RcBlock を明示的に leak してプロセス終了まで生存させる
    // （このオブザーバはアプリ終了まで解除しない前提）。
    std::mem::forget(begin_block);

    let app_end = app.clone();
    let end_block = block2::RcBlock::new(
        move |_notif: std::ptr::NonNull<objc2_foundation::NSNotification>| {
            let open_state = app_end.state::<TrayMenuOpenState>();
            open_state.0.store(false, Ordering::Release);
            // この通知は performClick のスタック内（= tray-icon の RefCell が
            // borrow されたまま）で同期発火するため、set_title / set_menu に
            // つながる後処理はスタックが巻き戻った後のターンに遅延させる。
            // ロック解除も同じ遅延ターンで行い、それまで他の更新を弾く。
            defer_on_main(&app_end, |app| {
                clear_tray_render_lock(app);
                apply_pending_tray_menu(app);
                refresh_tray_title(app);
            });
        },
    );
    unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(NSMenuDidEndTrackingNotification),
            None,
            None,
            &end_block,
        );
    }
    std::mem::forget(end_block);
}

#[cfg(not(target_os = "macos"))]
fn install_menu_tracking_observer(_app: &tauri::AppHandle) {}

/// n日分の予定 + Preferences / Quit を含むトレイメニューを再構築してトレイに適用する。
pub fn rebuild_tray_menu(app: &tauri::AppHandle, schedule: &[CachedEvent]) {
    // If rendering is locked (tray likely open), defer setting the menu and store
    // the schedule as pending to avoid calling `set_menu` while the OS menu is open.
    if is_tray_render_locked(app) {
        store_pending_schedule(app, schedule.to_vec());
        return;
    }
    let config = app.state::<ConfigState>();
    let days_to_show = config.0.read().map(|g| g.tray_days_to_show).unwrap_or(4);

    let now_local = Local::now();
    let today = now_local.date_naive();
    let weekdays = ["日", "月", "火", "水", "木", "金", "土"];

    let mut all_items: Vec<Box<dyn IsMenuItem<tauri::Wry>>> = Vec::new();
    for day_offset in 0..days_to_show {
        if day_offset > 0 {
            if let Ok(sep) = PredefinedMenuItem::separator(app) {
                all_items.push(Box::new(sep));
            }
        }

        let date = today + Duration::days(day_offset as i64);
        let label = if date == today {
            "Today".to_string()
        } else {
            let wd = date.weekday().num_days_from_sunday() as usize;
            format!("{}/{} ({})", date.month(), date.day(), weekdays[wd])
        };
        if let Ok(header) =
            MenuItem::with_id(app, format!("date_{date}"), label, false, None::<&str>)
        {
            all_items.push(Box::new(header));
        }

        let day_events: Vec<(usize, &CachedEvent, chrono::DateTime<Local>)> = schedule
            .iter()
            .enumerate()
            .filter_map(|(i, event)| {
                let local_start = event.start.with_timezone(&Local);
                (local_start.date_naive() == date).then_some((i, event, local_start))
            })
            .collect();

        if day_events.is_empty() {
            if let Ok(item) = MenuItem::with_id(
                app,
                format!("event_none_{date}"),
                "none",
                true,
                None::<&str>,
            ) {
                all_items.push(Box::new(item));
            }
            continue;
        }

        for (i, event, local_start) in day_events {
            let label = format_event_label(event, local_start);
            if let Some(item) = build_schedule_menu_item(app, i, event, label) {
                all_items.push(item);
            }
        }
    }

    if let Ok(sep) = PredefinedMenuItem::separator(app) {
        all_items.push(Box::new(sep));
    }
    if let Ok(pref) = MenuItem::with_id(app, "preferences", "Preferences...", true, None::<&str>) {
        all_items.push(Box::new(pref));
    }
    if let Ok(quit) = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>) {
        all_items.push(Box::new(quit));
    }
    if let Some(close) = close_menu_item(app) {
        all_items.push(Box::new(close));
    }

    let refs: Vec<&dyn IsMenuItem<tauri::Wry>> = all_items.iter().map(|b| b.as_ref()).collect();

    match Menu::with_items(app, &refs) {
        Ok(menu) => {
            run_if_unlocked(app, move |app| {
                if let Some(tray) = app.tray_by_id("main-tray") {
                    if let Err(e) = tray.set_menu(Some(menu)) {
                        eprintln!("failed to set tray menu: {e}");
                    }
                }
            });
        }
        Err(e) => eprintln!("failed to build tray menu: {e}"),
    }
}

fn build_schedule_menu_item(
    app: &tauri::AppHandle,
    event_index: usize,
    event: &CachedEvent,
    label: String,
) -> Option<Box<dyn IsMenuItem<tauri::Wry>>> {
    // Use calendar emoji (stored per-calendar) as a label prefix. When there are
    // no child actions, show a simple menu item prefixed with the emoji.
    let emoji = if event.calendar_emoji.trim().is_empty() {
        "⬛"
    } else {
        &event.calendar_emoji
    };

    if event.actions.is_empty() {
        let display_label = format!("{} {}", emoji, label);
        if let Ok(item) = MenuItem::with_id(
            app,
            format!("event_{event_index}"),
            display_label,
            true,
            None::<&str>,
        ) {
            return Some(Box::new(item));
        }
        return None;
    }

    let mut child_items: Vec<Box<dyn IsMenuItem<tauri::Wry>>> = Vec::new();
    for (action_index, action) in event.actions.iter().enumerate() {
        let id = format!("event_{event_index}_action_{action_index}");
        // Build a more informative display label: prefer a short service key
        // followed by the inner value (URL or decoded map query). Then
        // truncate to `max_display_width` visual width (fullwidth=2).
        // Show only the inner value (URL or decoded map query) without
        // a service prefix like "meet:" or "map:".
        let mut display_label = match action.label.as_str() {
            "Google map" => {
                if let Some(pos) = action.target.find("query=") {
                    let raw = &action.target[pos + "query=".len()..];
                    let end = raw.find('&').unwrap_or(raw.len());
                    let enc = &raw[..end];
                    match decode(enc) {
                        Ok(decoded) => decoded.into_owned(),
                        Err(_) => action.target.clone(),
                    }
                } else {
                    action.target.clone()
                }
            }
            // For Meet/Zoom and other links, show the target (URL or text) only.
            _ => action.target.clone(),
        };

        // Truncate display label to maximum visual width
        fn truncate_display(s: &str, max_width: usize) -> String {
            if s.width() <= max_width {
                return s.to_string();
            }
            let mut acc = 0usize;
            let mut out = String::new();
            for ch in s.chars() {
                let w = ch.width().unwrap_or(0);
                // Reserve width for ellipsis "..." (3)
                if acc + w + 3 > max_width {
                    break;
                }
                out.push(ch);
                acc += w;
            }
            if out.is_empty() {
                "...".to_string()
            } else {
                format!("{}...", out)
            }
        }

        display_label = truncate_display(&display_label, 30);

        if let Ok(item) = MenuItem::with_id(app, id, display_label, true, None::<&str>) {
            child_items.push(Box::new(item));
        }
    }

    if child_items.is_empty() {
        return None;
    }

    let child_refs: Vec<&dyn IsMenuItem<tauri::Wry>> =
        child_items.iter().map(|item| item.as_ref()).collect();
    let display_label = format!("{} {}", emoji, label);
    Submenu::with_items(app, display_label, true, &child_refs)
        .ok()
        .map(|submenu| Box::new(submenu) as Box<dyn IsMenuItem<tauri::Wry>>)
}

fn resolve_event_action_target(app: &tauri::AppHandle, menu_id: &str) -> Option<String> {
    let event_id = menu_id.strip_prefix("event_")?;
    let (event_index_str, action_index_str) = event_id.split_once("_action_")?;
    let event_index = event_index_str.parse::<usize>().ok()?;
    let action_index = action_index_str.parse::<usize>().ok()?;
    let schedule = cached_schedule(app)?;
    let event = schedule.get(event_index)?;
    let action = event.actions.get(action_index)?;
    Some(action.target.clone())
}

pub fn setup(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let config = AppConfig::load_or_create()?;
    let config_arc = Arc::new(RwLock::new(config.clone()));

    app.manage(ConfigState(Arc::clone(&config_arc)));
    let refresh_notify = Arc::new(tokio::sync::Notify::new());
    app.manage(RefreshSignal(Arc::clone(&refresh_notify)));

    let is_hidden = Arc::new(AtomicBool::new(false));

    app.manage(StealthState {
        is_hidden: Arc::clone(&is_hidden),
    });
    app.manage(TrayMenuOpenState(AtomicBool::new(false)));
    app.manage(TrayPresentationState {
        render_lock_until: Mutex::new(None),
        pending_schedule: Mutex::new(None),
    });
    app.manage(CachedSchedule(Mutex::new(None)));
    app.manage(CachedEvents(Mutex::new(None)));

    let (shutdown_tx, _) = broadcast::channel::<()>(1);
    app.manage(ShutdownHandle(shutdown_tx.clone()));

    let allow_exit = Arc::new(AtomicBool::new(false));
    app.manage(AllowExit(Arc::clone(&allow_exit)));

    // 初期メニュー（予定取得前）
    let initial_close = close_menu_item(&app.handle().clone());
    let mut initial_items: Vec<&dyn IsMenuItem<tauri::Wry>> = Vec::new();
    let pref_item = MenuItem::with_id(app, "preferences", "Preferences...", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    initial_items.push(&pref_item);
    initial_items.push(&quit_item);
    if let Some(close) = initial_close.as_ref() {
        initial_items.push(close);
    }
    let initial_menu = Menu::with_items(app, &initial_items)?;

    let initial_title = config.normal_title();

    let app_handle_menu = app.handle().clone();
    app.on_menu_event(move |_, event| match event.id.as_ref() {
        "preferences" => {
            clear_tray_render_lock(&app_handle_menu);
            apply_pending_tray_menu(&app_handle_menu);
            refresh_tray_title(&app_handle_menu);
            commands::open_settings_window(&app_handle_menu)
        }
        "quit" => {
            let allow_exit = app_handle_menu.state::<AllowExit>();
            allow_exit.0.store(true, Ordering::Relaxed);
            app_handle_menu.exit(0);
        }
        menu_id if menu_id.starts_with("event_") => {
            clear_tray_render_lock(&app_handle_menu);
            apply_pending_tray_menu(&app_handle_menu);
            refresh_tray_title(&app_handle_menu);
            if let Some(target) = resolve_event_action_target(&app_handle_menu, menu_id) {
                if let Err(e) = webbrowser::open(&target) {
                    eprintln!("failed to open external target: {e}");
                }
            }
        }
        _ => {
            clear_tray_render_lock(&app_handle_menu);
            apply_pending_tray_menu(&app_handle_menu);
            refresh_tray_title(&app_handle_menu);
        }
    });

    // 実クリックでのトレイ描画ロックの set/clear は、以前はここの
    // on_tray_icon_event（Down/Up クリック）で toggle していたが、
    // install_menu_tracking_observer が購読する NSMenuDidBeginTracking /
    // DidEndTracking 通知に一本化した（ESC・外クリックでの dismiss でも
    // 確実にクリアされるため、クリックイベントのみを見るより正確）。
    TrayIconBuilder::with_id("main-tray")
        .icon(menu_bar_icon())
        .icon_as_template(false)
        .title(format!(" {initial_title}").leak())
        .tooltip("AuraCalendar")
        .menu(&initial_menu)
        .show_menu_on_left_click(true)
        .build(app)?;

    install_menu_tracking_observer(app.handle());

    register_stealth_shortcut(app.handle(), &config.stealth_shortcut)
        .unwrap_or_else(|e| eprintln!("failed to register stealth shortcut: {e}"));
    register_open_menu_shortcut(app.handle(), &config.open_menu_shortcut)
        .unwrap_or_else(|e| eprintln!("failed to register open menu shortcut: {e}"));

    // ── 取得タスク（長周期）──────────────────────────────────────────
    {
        let app_handle = app.handle().clone();
        let fetch_config = Arc::clone(&config_arc);
        let fetch_refresh = Arc::clone(&refresh_notify);
        let mut shutdown_rx = shutdown_tx.subscribe();

        tauri::async_runtime::spawn(async move {
            loop {
                let config = fetch_config.read().map(|g| g.clone()).unwrap_or_default();
                let fetch_duration =
                    std::time::Duration::from_secs(config.refresh_interval_seconds);

                let (tray_events, schedule_events) = match fetch(&config).await {
                    Ok(result) => (result.tray_events, result.schedule_events),
                    Err(e) => {
                        eprintln!("failed to fetch calendar: {e}");
                        (None, Vec::new())
                    }
                };

                if let Ok(mut cached) = app_handle.state::<CachedEvents>().0.lock() {
                    *cached = tray_events;
                }
                store_cached_schedule(&app_handle, schedule_events.clone());

                if is_tray_render_locked(&app_handle) {
                    store_pending_schedule(&app_handle, schedule_events);
                } else {
                    rebuild_tray_menu(&app_handle, &schedule_events);
                }

                tokio::select! {
                    _ = shutdown_rx.recv() => break,
                    _ = fetch_refresh.notified() => {}
                    _ = tokio::time::sleep(fetch_duration) => {}
                }
            }
        });
    }

    // ── 表示タスク（短周期）──────────────────────────────────────────
    {
        let app_handle = app.handle().clone();
        let disp_config = Arc::clone(&config_arc);
        let disp_hidden = Arc::clone(&is_hidden);
        let mut shutdown_rx = shutdown_tx.subscribe();

        tauri::async_runtime::spawn(async move {
            loop {
                let config = disp_config.read().map(|g| g.clone()).unwrap_or_default();
                let disp_duration = std::time::Duration::from_secs(config.display_interval_seconds);
                let now = Utc::now();

                let _next_title = {
                    let events = app_handle
                        .state::<CachedEvents>()
                        .0
                        .lock()
                        .ok()
                        .and_then(|guard| guard.clone());
                    match events {
                        Some(ref evts) => render_title(&config, evts, now),
                        None => config.normal_title(),
                    }
                };

                if !disp_hidden.load(Ordering::Relaxed) {
                    refresh_tray_title(&app_handle);
                }
                if !is_tray_render_locked(&app_handle) {
                    apply_pending_tray_menu(&app_handle);
                }

                tokio::select! {
                    _ = shutdown_rx.recv() => break,
                    _ = tokio::time::sleep(disp_duration) => {}
                }
            }
        });
    }

    Ok(())
}

fn format_event_label(event: &CachedEvent, local_start: chrono::DateTime<Local>) -> String {
    let start_hm = format!("{:02}:{:02}", local_start.hour(), local_start.minute());
    let end_str = match event.end {
        None => String::new(),
        Some(end_utc) => {
            let local_end = end_utc.with_timezone(&Local);
            let day_diff = (local_end.date_naive() - local_start.date_naive()).num_days();
            match day_diff {
                0 => format!("{:02}:{:02}", local_end.hour(), local_end.minute()),
                1 => format!("{:02}:{:02}", local_end.hour() + 24, local_end.minute()),
                _ => String::new(),
            }
        }
    };
    let title = truncate_chars(&event.title, 30);
    if end_str.is_empty() {
        format!("{}~ {}", start_hm, title)
    } else {
        format!("{}~{} {}", start_hm, end_str, title)
    }
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}...", truncated)
    } else {
        truncated
    }
}

// Removed legacy color-dot helper and hex parsing (unused after emoji migration).
