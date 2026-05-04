# aura-calendar agents.md

このファイルは、AI がこのリポジトリを触るときの最小限の案内です。迷ったら、まず既存の実装と `README.md`、`docs/guide.md`、`docs/git-workflow.md` を確認してください。

## 目的

- macOS のメニューバーに予定を表示する AuraCalendar を、安全に小さく直す
- 既存の設計や運用に合わせて、必要最小限の変更で収める

## まず把握すること

- 作業ブランチは原則 `dev`
- Git の運用は [docs/git-workflow.md](docs/git-workflow.md) に従う
- ユーザー向けの使い方は [docs/guide.md](docs/guide.md) が正
- アプリ概要と開発コマンドは [README.md](README.md) を見る

## 主要な編集ポイント

- バックエンドの入口は [src/app/mod.rs](src/app/mod.rs)
- トレイメニューや常駐処理は [src/app/tray.rs](src/app/tray.rs)
- Tauri コマンドは [src/app/commands.rs](src/app/commands.rs)
- 設定画面 UI は [dist/settings.html](dist/settings.html)
- アイコンや画像は [icons/](icons/) と [docs/images/](docs/images/) を確認する

## 変更の考え方

- まずは既存の実装パターンに寄せる
- 1つの修正で複数箇所を触る場合は、呼び出し元と受け側の両方を揃える
- UI を変えるときは、表示だけでなく保存・キャンセル・閉じる動作まで確認する
- 余計なリファクタリングはしない

## このリポジトリで特に注意すること

- `dist/settings.html` は設定画面の実体なので、UI の修正はここを直接直す
- `target/` は生成物なので触らない
- 設定画面の操作を Rust 側で制御する場合は、`invoke_handler` にコマンド登録が必要
- トレイの Quit はアプリ終了、設定画面の Cancel は設定画面を閉じる、というように役割を分ける

## 変更後の確認

- Rust を触ったら `cargo check` を優先して実行する
- 挙動変更がある場合は、関連する画面やトレイ操作を実機で確認する
- フォーマットが崩れたら `cargo fmt --all` を使う

## Git の進め方

- 新しい作業は `dev` から切る
- ブランチ名は `feature/` か `fix/` を基本にする
- 変更がまとまったら commit し、必要なら push して PR を作る
