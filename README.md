# AuraCalendar (オーラ・カレンダー)

AuraCalendar は、メニューバーから予定をスマートに確認するための macOS 向けデスクトップアプリです。

## 主な特徴

- ステルス・モード: ショートカットやクリック一つで、予定のタイトルを隠したり（`*`）、アイコンのみの表示に切り替え。背後に人が来た時も安心です。
- カスタム表示: 「あと何分」や「タイトルの一部」など、自分が必要な情報だけをメニューバーに抽出して表示。
- iCal 連携: Google カレンダーなどの非公開URL（iCal形式）を登録するだけで、複雑な認証なしに予定を同期。
- 軽量・高速: Rust + Tauri で構築し、メモリ消費を抑えつつサクサク動作することを目指します。

## 技術スタック

- Native shell: [Tauri 2.0](https://tauri.app/)
- Settings UI: [Dioxus 0.6](https://dioxuslabs.com/) (予定)
- Language: [Rust](https://www.rust-lang.org/)

## 開発ロードマップ

### Phase 1: 基盤構築 (現在)

- [x] プロジェクトの初期化
- [x] メニューバー（システムトレイ）へのタイトル表示
- [x] クリックによる表示モードの切り替えロジック

### Phase 2: カレンダー連携

- [x] ローカル設定ファイルの自動生成
- [ ] iCal 形式の URL 入力・保存機能
- [ ] 定期的なスケジュール・フェッチ機能
- [ ] 「次の予定まであと◯分」の計算ロジック

### Phase 3: カスタマイズ & ブラッシュアップ

- [ ] ステルスモードのショートカットキー実装
- [ ] 設定画面（GUI）の構築
- [ ] アプリアイコンのデザインと適用

## インストールと実行

```bash
# リポジトリのクローン
git clone https://github.com/YOUR_USERNAME/aura-calendar.git
cd aura-calendar

# 実行
cargo run
```

## 開発チェック

Pull Request と `main` への push では、GitHub Actions で以下を実行します。

```bash
cargo fmt --all --check
cargo check --locked
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
```

`cargo check` は Rust の型チェックとコンパイル可能性を高速に確認するコマンドです。React/TypeScript でいう型チェックに近く、成果物の生成までは行いません。

## ローカル設定

AuraCalendar は、ユーザーごとの設定を以下のローカルファイルに保存します。

```text
~/Library/Application Support/AuraCalendar/config.json
```

初回起動時にファイルが存在しない場合は、空のカレンダー設定を自動生成します。

```json
{
  "calendars": [],
  "display": {
    "normal_format": "{minutes_until}分後 {title}",
    "stealth_format": "***",
    "show_title": true
  },
  "refresh_interval_seconds": 300
}
```

## 配布方針

AuraCalendar は macOS 向けに `.dmg` 形式で配布する予定です。

開発が進んだら、Tauri の bundle 設定を有効化し、以下のように DMG を生成します。

```bash
cargo tauri build --bundles dmg
```

公開配布する場合は、macOS の Gatekeeper 警告を避けるために Apple Developer Program を利用した code signing と notarization を行います。
