# modular-agent-slack

[Modular Agent Kit](https://github.com/modular-agent/modular-agent-kit) 用の Slack エージェント。

## エージェント

### Slack/Post

Slack チャネルにメッセージを投稿します。

**設定:**
- `channel`: チャネル名（例: `#general`）またはチャネルID
- `token`: ボットトークン。以下の形式に対応:
  - 空: 環境変数 `SLACK_BOT_TOKEN` を使用
  - `$ENV_NAME`: 指定した環境変数を使用
  - トークン値を直接指定

**入力:**
- `message`: 文字列メッセージ、または `text`, `blocks`, `thread_ts` フィールドを持つオブジェクト

**出力:**
- `result`: 成功時に `ok`, `ts`, `channel` を含むオブジェクト

### Slack/History

Slack チャネルのメッセージ履歴を取得します。

**設定:**
- `channel`: チャネル名またはID
- `token`: ボットトークン（Slack/Post と同じ形式）
- `limit`: 取得するメッセージ数（デフォルト: 10）

**入力:**
- `trigger`: 任意の値で履歴取得をトリガー

**出力:**
- `messages`: `text`, `user`, `ts`, `thread_ts` フィールドを持つメッセージオブジェクトの配列

### Slack/Channels

利用可能な Slack チャネルの一覧を取得します。

**設定:**
- `token`: ボットトークン（Slack/Post と同じ形式）
- `limit`: 取得するチャネル数（デフォルト: 100）

**入力:**
- `trigger`: 任意の値でチャネル一覧取得をトリガー

**出力:**
- `channels`: `id`, `name`, `is_private`, `is_archived`, `is_member`, `num_members`, `topic`, `purpose` フィールドを持つチャネルオブジェクトの配列

## セットアップ

### 環境変数

- `SLACK_BOT_TOKEN`: Slack Bot User OAuth Token（`xoxb-` で始まる）

### 必要な Slack アプリ権限

Bot Token Scopes:
- `channels:history` - パブリックチャネルのメッセージを閲覧
- `channels:read` - チャネルの基本情報を閲覧
- `chat:write` - メッセージを送信
- `chat:write.public` - 参加していないチャネルにメッセージを送信
- `groups:read` - プライベートチャネルの基本情報を閲覧（任意）
- `groups:history` - プライベートチャネルのメッセージを閲覧（任意）

## ライセンス

Apache-2.0 OR MIT
