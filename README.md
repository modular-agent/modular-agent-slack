# modular-agent-slack

Slack Agents for [Modular Agent Kit](https://github.com/modular-agent/modular-agent-kit).

## Agents

### Slack/Post

Posts messages to Slack channels.

**Configuration:**
- `channel`: Channel name (e.g., `#general`) or channel ID
- `token`: Bot token. Can be:
  - Empty: uses `SLACK_BOT_TOKEN` environment variable
  - `$ENV_NAME`: uses the specified environment variable
  - Direct token value

**Input:**
- `message`: String message, or object with `text`, `blocks`, `thread_ts` fields

**Output:**
- `result`: Object containing `ok`, `ts`, `channel` on success

### Slack/History

Fetches message history from a Slack channel.

**Configuration:**
- `channel`: Channel name or ID
- `token`: Bot token (same format as Slack/Post)
- `limit`: Number of messages to fetch (default: 10)

**Input:**
- `trigger`: Any value triggers fetching the history

**Output:**
- `messages`: Array of message objects with `text`, `user`, `ts`, `thread_ts` fields

### Slack/Channels

Lists available Slack channels.

**Configuration:**
- `token`: Bot token (same format as Slack/Post)
- `limit`: Number of channels to fetch (default: 100)

**Input:**
- `trigger`: Any value triggers fetching the channel list

**Output:**
- `channels`: Array of channel objects with `id`, `name`, `is_private`, `is_archived`, `is_member`, `num_members`, `topic`, `purpose` fields

## Setup

### Environment Variables

- `SLACK_BOT_TOKEN`: Slack Bot User OAuth Token (starts with `xoxb-`)

### Required Slack App Permissions

Bot Token Scopes:
- `channels:history` - View messages in public channels
- `channels:read` - View basic channel information
- `chat:write` - Send messages
- `chat:write.public` - Send messages to channels without joining
- `groups:read` - View basic information about private channels (optional)
- `groups:history` - View messages in private channels (optional)

## License

Apache-2.0 OR MIT
