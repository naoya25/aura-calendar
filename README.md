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
- [ ] クリックによる表示モードの切り替えロジック

### Phase 2: カレンダー連携

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

## 配布方針

AuraCalendar は macOS 向けに `.dmg` 形式で配布する予定です。

開発が進んだら、Tauri の bundle 設定を有効化し、以下のように DMG を生成します。

```bash
cargo tauri build --bundles dmg
```

公開配布する場合は、macOS の Gatekeeper 警告を避けるために Apple Developer Program を利用した code signing と notarization を行います。
