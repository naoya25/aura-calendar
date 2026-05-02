# AuraCalendar

macOS のメニューバーに次の予定をシンプルに表示するアプリです。

Google カレンダーなどの iCal URL を登録するだけで、複雑な認証なしに予定を確認できます。

---

## 機能

- **メニューバー表示** — 次の予定タイトルと開始までの残り時間をメニューバーに常駐表示
- **ステルスモード** — アイコンを左クリックするだけで予定の表示・非表示を瞬時に切り替え（背後に人が来ても安心）
- **複数カレンダー対応** — 複数の iCal URL を登録し、最も直近の予定を自動選択
- **繰り返し予定対応** — RRULE / EXDATE を解釈し、定例会議なども正しく表示
- **GUI 設定画面** — トレイを右クリック → Preferences... から設定を変更可能（config.json の直接編集不要）
- **表示フォーマット自由設定** — `{minutes_until}`, `{hh}`, `{mm}`, `{title}` などのプレースホルダーで表示内容をカスタマイズ

## スクリーンショット

> _(準備中)_

## 動作環境

- macOS 12 Sequoia 以降
- Apple Silicon / Intel 両対応

## インストール

現在はソースからのビルドのみ対応しています。

**必要なもの**

- [Rust](https://www.rust-lang.org/tools/install) (stable)
- [Tauri CLI](https://tauri.app/start/prerequisites/)

```bash
git clone https://github.com/naoya25/aura-calendar.git
cd aura-calendar
cargo run
```

## 使い方

1. アプリを起動するとメニューバーにアイコンが表示されます
2. **右クリック → Preferences...** でカレンダーの iCal URL を登録
3. iCal URL は Google カレンダーの「設定 → カレンダーの統合 → 非公開の iCal 形式の URL」から取得できます
4. **左クリック** で予定の表示・非表示を切り替え（ステルスモード）
5. **右クリック → Quit AuraCalendar** で終了

## 設定

設定は GUI から変更できます。設定ファイルの保存先：

```
~/Library/Application Support/AuraCalendar/config.json
```

**設定項目**

| キー                       | 説明                                          | デフォルト                    |
| -------------------------- | --------------------------------------------- | ----------------------------- |
| `calendars`                | カレンダー名と iCal URL のリスト              | `[]`                          |
| `display.normal_format`    | 通常時の表示フォーマット                      | `{minutes_until}分後 {title}` |
| `display.stealth_format`   | ステルス時の表示文字列                        | `***`                         |
| `display.show_title`       | 予定タイトルを表示するか                      | `true`                        |
| `refresh_interval_seconds` | カレンダー取得・表示更新の間隔（秒、最小 30） | `300`                         |

**フォーマットのプレースホルダー**

| プレースホルダー  | 内容                     |
| ----------------- | ------------------------ |
| `{minutes_until}` | 開始まで何分か（合計分） |
| `{hh}`            | 時間部分                 |
| `{mm}`            | 分部分（2桁）            |
| `{title}`         | 予定のタイトル           |

## 技術スタック

- [Tauri 2](https://tauri.app/) — クロスプラットフォームデスクトップフレームワーク
- [Rust](https://www.rust-lang.org/) — バックエンド全般
- HTML / CSS / JavaScript — 設定画面 UI

## 開発

```bash
# ビルド
cargo build

# テスト
cargo test

# フォーマット・Lint
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
```

Pull Request と `main` への push では GitHub Actions で fmt / check / clippy / test を自動実行します。

## 実装予定

- [ ] 表示更新とカレンダー取得の間隔を分離（表示は短周期、取得は長周期）
- [ ] 右クリック時に予定一覧表示
- [ ] DMG 配布 / Apple Developer コードサイニング対応

## ライセンス

MIT
