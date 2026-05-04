# 開発ワークフローとブランチ命名規則

このドキュメントでは、aura-calendar の開発におけるブランチ管理とリリース手順について定義します。

## 1. ブランチの種類

| ブランチ名 | 役割                             | 分岐元    | マージ先           |
| :--------- | :------------------------------- | :-------- | :----------------- |
| `main`     | リリース済みの安定コード         | -         | -                  |
| `develop`  | 次期バージョンのための開発ベース | `main`    | `main`             |
| `feature/` | 新機能の開発                     | `develop` | `develop`          |
| `fix/`     | バグ修正                         | `develop` | `develop`          |
| `release/` | リリース直前の最終調整           | `develop` | `main` & `develop` |

## 2. 命名ルール

- 基本は `種類/内容` (例: `feature/add-reminders`)
- 全て小文字、単語間はハイフン `-` で繋ぐ（ケバブケース）
- GitHubのIssueがある場合は番号を含める (例: `feature/12-dark-mode`)

## 3. 基本操作

### 新しい機能を作る時

```bash
# 最新の状態を取得
git switch dev
git pull origin dev

# 機能ブランチを作成して切り替え
git switch -c feature/<your-feature-name>
```

### 開発が終わってプルリクエストを送る時

1. GitHub上で `feature/xxx` -> `dev` へPRを作成
2. テンプレートに従って内容を記載し、マージ

### リリースする時

```bash
# 1. リリースブランチの作成
git switch dev
git switch -c release/vx.x.x

# 2. バージョン番号の更新など、最終調整してコミット
# (XcodeでProjectの設定変更など)

# 3. mainへマージしてタグ付け
git switch main
git merge release/vx.x.x
git tag vx.x.x

# 4. devにも修正を戻す
git switch dev
git merge release/vx.x.x

# 5. 不要になったリリースブランチを削除
git branch -d release/vx.x.x
```
