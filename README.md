# modular-agent-slack

Slack Agents for [Modular Agent](https://github.com/modular-agent/modular-agent-core).

## Agents

### Slack/Post

Posts messages to Slack channels.

**Configuration:**
- `channel`: Channel name (e.g., `#general`) or channel ID

**Input:**
- `message`: String message, or object with `text`, `blocks`, `thread_ts` fields

**Output:**
- `result`: Object containing `ok`, `ts`, `channel` on success

### Slack/History

Fetches message history from a Slack channel.

**Configuration:**
- `channel`: Channel name or ID
- `limit`: Number of messages to fetch (default: 10)

**Input:**
- `trigger`: Any value triggers fetching the history

**Output:**
- `messages`: Array of message objects with `text`, `user`, `ts`, `thread_ts` fields

### Slack/Channels

Lists available Slack channels.

**Configuration:**
- `limit`: Number of channels to fetch (default: 100)

**Input:**
- `trigger`: Any value triggers fetching the channel list

**Output:**
- `channels`: Array of channel objects with `id`, `name`, `is_private`, `is_archived`, `is_member`, `num_members`, `topic`, `purpose` fields

### Slack/Listener

Listens to Slack messages in real-time via Socket Mode. Outputs messages as they arrive.

**Configuration:**
- `channel`: Optional channel filter. If empty, listens to all channels.

**Output:**

- `message`: Message objects with `text`, `user`, `channel`, `ts`, `thread_ts` fields

## Setup

### Global Config or Environment Variables

- `SLACK_BOT_TOKEN`: Slack Bot User OAuth Token (starts with `xoxb-`)
- `SLACK_APP_TOKEN`: Slack App-Level Token with `connections:write` scope (starts with `xapp-`, required for Slack/Listener)

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
