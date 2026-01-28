use std::env;
use std::sync::OnceLock;

use im::{Vector, hashmap};
use modular_agent_kit::{
    Agent, AgentContext, AgentData, AgentError, AgentOutput, AgentSpec, AgentValue, AsAgent, MAK,
    async_trait, modular_agent,
};
use slack_morphism::prelude::*;

static CATEGORY: &str = "Slack";

static PORT_MESSAGE: &str = "message";
static PORT_RESULT: &str = "result";
static PORT_TRIGGER: &str = "trigger";
static PORT_MESSAGES: &str = "messages";

static CONFIG_CHANNEL: &str = "channel";
static CONFIG_TOKEN: &str = "token";
static CONFIG_LIMIT: &str = "limit";
static CONFIG_SLACK_BOT_TOKEN: &str = "slack_bot_token";

type HyperConnector = SlackClientHyperConnector<SlackHyperHttpsConnector>;

static CLIENT: OnceLock<SlackClient<HyperConnector>> = OnceLock::new();

fn get_client() -> &'static SlackClient<HyperConnector> {
    CLIENT.get_or_init(|| {
        SlackClient::new(
            SlackClientHyperConnector::new().expect("Failed to create Slack client HTTP connector"),
        )
    })
}

fn get_token(mak: &MAK, config_token: &str) -> Result<SlackApiToken, AgentError> {
    let token_str = if !config_token.is_empty() {
        // 1. Agent-level config takes priority
        if let Some(env_name) = config_token.strip_prefix('$') {
            env::var(env_name).map_err(|_| {
                AgentError::InvalidValue(format!("Environment variable {} not set", env_name))
            })?
        } else {
            config_token.to_string()
        }
    } else if let Some(global_token) = mak
        .get_global_configs(SlackPostAgent::DEF_NAME)
        .and_then(|cfg| cfg.get_string(CONFIG_SLACK_BOT_TOKEN).ok())
        .filter(|key| !key.is_empty())
    {
        // 2. Global config
        global_token
    } else {
        // 3. Environment variable fallback
        env::var("SLACK_BOT_TOKEN")
            .map_err(|_| AgentError::InvalidValue("SLACK_BOT_TOKEN not set".to_string()))?
    };

    Ok(SlackApiToken::new(SlackApiTokenValue(token_str)))
}

/// Agent for posting messages to Slack channels.
///
/// # Configuration
/// - `channel`: The Slack channel name (e.g., "#general") or channel ID
/// - `token`: Bot token. Can be:
///   - Empty: uses SLACK_BOT_TOKEN environment variable
///   - $ENV_NAME: uses the specified environment variable
///   - Direct token value
///
/// # Input
/// - `message`: String message or object with `text`, `blocks`, `thread_ts` fields
///
/// # Output
/// - `result`: Object containing `ok`, `ts`, `channel` on success
#[modular_agent(
    title = "Post",
    category = CATEGORY,
    inputs = [PORT_MESSAGE],
    outputs = [PORT_RESULT],
    string_config(name = CONFIG_CHANNEL),
    string_config(name = CONFIG_TOKEN),
    string_global_config(name = CONFIG_SLACK_BOT_TOKEN, title = "Slack Bot Token"),
)]
struct SlackPostAgent {
    data: AgentData,
}

#[async_trait]
impl AsAgent for SlackPostAgent {
    fn new(mak: MAK, id: String, spec: AgentSpec) -> Result<Self, AgentError> {
        Ok(Self {
            data: AgentData::new(mak, id, spec),
        })
    }

    async fn process(
        &mut self,
        ctx: AgentContext,
        _port: String,
        value: AgentValue,
    ) -> Result<(), AgentError> {
        let config = self.configs()?;
        let channel = config.get_string(CONFIG_CHANNEL)?;
        if channel.is_empty() {
            return Err(AgentError::InvalidValue(
                "Channel not configured".to_string(),
            ));
        }

        let token = get_token(self.mak(), &config.get_string_or_default(CONFIG_TOKEN))?;
        let client = get_client();
        let session = client.open_session(&token);

        let (text, blocks, thread_ts) = extract_message_content(&value)?;

        let channel_id: SlackChannelId = channel.into();
        let content = SlackMessageContent::new().with_text(text);

        let mut request = SlackApiChatPostMessageRequest::new(channel_id, content);

        if let Some(ts) = thread_ts {
            request = request.with_thread_ts(ts.into());
        }

        if let Some(blocks_value) = blocks
            && let Ok(blocks_json) = serde_json::to_string(&blocks_value.to_json())
            && let Ok(slack_blocks) = serde_json::from_str::<Vec<SlackBlock>>(&blocks_json)
        {
            let content_with_blocks = SlackMessageContent::new()
                .with_text(request.content.text.unwrap_or_default())
                .with_blocks(slack_blocks);
            request = SlackApiChatPostMessageRequest::new(request.channel, content_with_blocks);
        }

        let response = session
            .chat_post_message(&request)
            .await
            .map_err(|e| AgentError::IoError(format!("Slack API error: {}", e)))?;

        let result = AgentValue::object(hashmap! {
            "ok".into() => AgentValue::boolean(true),
            "ts".into() => AgentValue::string(response.ts.to_string()),
            "channel".into() => AgentValue::string(response.channel.to_string()),
        });

        self.output(ctx, PORT_RESULT, result).await
    }
}

fn extract_message_content(
    value: &AgentValue,
) -> Result<(String, Option<AgentValue>, Option<String>), AgentError> {
    match value {
        AgentValue::String(s) => Ok((s.to_string(), None, None)),
        AgentValue::Object(obj) => {
            let text = obj
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let blocks = obj.get("blocks").cloned();
            let thread_ts = obj
                .get("thread_ts")
                .and_then(|v| v.as_str())
                .map(String::from);
            Ok((text, blocks, thread_ts))
        }
        AgentValue::Array(arr) => {
            let texts: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str())
                .map(String::from)
                .collect();
            Ok((texts.join("\n"), None, None))
        }
        _ => {
            let json = serde_json::to_string_pretty(&value.to_json()).unwrap_or_default();
            Ok((format!("```\n{}\n```", json), None, None))
        }
    }
}

/// Agent for fetching message history from a Slack channel.
///
/// # Configuration
/// - `channel`: The Slack channel name or ID to fetch history from
/// - `token`: Bot token (same format as SlackPostAgent)
/// - `limit`: Maximum number of messages to fetch (default: 10)
///
/// # Input
/// - `trigger`: Any value triggers fetching the history
///
/// # Output
/// - `messages`: Array of message objects containing `text`, `user`, `ts`, etc.
#[modular_agent(
    title = "History",
    category = CATEGORY,
    inputs = [PORT_TRIGGER],
    outputs = [PORT_MESSAGES],
    string_config(name = CONFIG_CHANNEL),
    string_config(name = CONFIG_TOKEN),
    integer_config(name = CONFIG_LIMIT),
)]
struct SlackHistoryAgent {
    data: AgentData,
}

#[async_trait]
impl AsAgent for SlackHistoryAgent {
    fn new(mak: MAK, id: String, spec: AgentSpec) -> Result<Self, AgentError> {
        Ok(Self {
            data: AgentData::new(mak, id, spec),
        })
    }

    async fn process(
        &mut self,
        ctx: AgentContext,
        _port: String,
        _value: AgentValue,
    ) -> Result<(), AgentError> {
        let config = self.configs()?;
        let channel = config.get_string(CONFIG_CHANNEL)?;
        if channel.is_empty() {
            return Err(AgentError::InvalidValue(
                "Channel not configured".to_string(),
            ));
        }

        let token = get_token(self.mak(), &config.get_string_or_default(CONFIG_TOKEN))?;
        let limit = config.get_integer_or_default(CONFIG_LIMIT);
        let limit = if limit <= 0 { 10 } else { limit as u16 };

        let client = get_client();
        let session = client.open_session(&token);

        let channel_id: SlackChannelId = channel.into();
        let request = SlackApiConversationsHistoryRequest::new()
            .with_channel(channel_id)
            .with_limit(limit);

        let response = session
            .conversations_history(&request)
            .await
            .map_err(|e| AgentError::IoError(format!("Slack API error: {}", e)))?;

        let messages: Vector<AgentValue> = response
            .messages
            .iter()
            .map(slack_message_to_agent_value)
            .collect();

        self.output(ctx, PORT_MESSAGES, AgentValue::array(messages))
            .await
    }
}

fn slack_message_to_agent_value(msg: &SlackHistoryMessage) -> AgentValue {
    let mut obj = im::HashMap::new();

    // SlackHistoryMessage uses #[serde(flatten)] so fields are directly accessible
    if let Some(text) = &msg.content.text {
        obj.insert("text".into(), AgentValue::string(text.clone()));
    }

    if let Some(user) = &msg.sender.user {
        obj.insert("user".into(), AgentValue::string(user.to_string()));
    }

    obj.insert("ts".into(), AgentValue::string(msg.origin.ts.to_string()));

    if let Some(thread_ts) = &msg.origin.thread_ts {
        obj.insert(
            "thread_ts".into(),
            AgentValue::string(thread_ts.to_string()),
        );
    }

    AgentValue::object(obj)
}

/// Agent for listing Slack channels.
///
/// # Configuration
/// - `token`: Bot token (same format as SlackPostAgent)
/// - `limit`: Maximum number of channels to fetch (default: 100)
///
/// # Input
/// - `trigger`: Any value triggers fetching the channel list
///
/// # Output
/// - `channels`: Array of channel objects containing `id`, `name`, `is_private`, etc.
#[modular_agent(
    title = "Channels",
    category = CATEGORY,
    inputs = [PORT_TRIGGER],
    outputs = ["channels"],
    string_config(name = CONFIG_TOKEN),
    integer_config(name = CONFIG_LIMIT),
)]
struct SlackChannelsAgent {
    data: AgentData,
}

#[async_trait]
impl AsAgent for SlackChannelsAgent {
    fn new(mak: MAK, id: String, spec: AgentSpec) -> Result<Self, AgentError> {
        Ok(Self {
            data: AgentData::new(mak, id, spec),
        })
    }

    async fn process(
        &mut self,
        ctx: AgentContext,
        _port: String,
        _value: AgentValue,
    ) -> Result<(), AgentError> {
        let config = self.configs()?;
        let token = get_token(self.mak(), &config.get_string_or_default(CONFIG_TOKEN))?;
        let limit = config.get_integer_or_default(CONFIG_LIMIT);
        let limit = if limit <= 0 { 100 } else { limit as u16 };

        let client = get_client();
        let session = client.open_session(&token);

        let request = SlackApiConversationsListRequest::new().with_limit(limit);

        let response = session
            .conversations_list(&request)
            .await
            .map_err(|e| AgentError::IoError(format!("Slack API error: {}", e)))?;

        let channels: Vector<AgentValue> = response
            .channels
            .iter()
            .map(slack_channel_to_agent_value)
            .collect();

        self.output(ctx, "channels", AgentValue::array(channels))
            .await
    }
}

fn slack_channel_to_agent_value(ch: &SlackChannelInfo) -> AgentValue {
    let mut obj = im::HashMap::new();

    obj.insert("id".into(), AgentValue::string(ch.id.to_string()));

    if let Some(name) = &ch.name {
        obj.insert("name".into(), AgentValue::string(name.clone()));
    }

    // Access flags fields directly (they are Option<bool>)
    if let Some(is_private) = ch.flags.is_private {
        obj.insert("is_private".into(), AgentValue::boolean(is_private));
    }

    if let Some(is_archived) = ch.flags.is_archived {
        obj.insert("is_archived".into(), AgentValue::boolean(is_archived));
    }

    if let Some(is_member) = ch.flags.is_member {
        obj.insert("is_member".into(), AgentValue::boolean(is_member));
    }

    if let Some(num_members) = ch.num_members {
        obj.insert(
            "num_members".into(),
            AgentValue::integer(num_members as i64),
        );
    }

    if let Some(ref topic) = ch.topic {
        obj.insert("topic".into(), AgentValue::string(topic.value.clone()));
    }

    if let Some(ref purpose) = ch.purpose {
        obj.insert("purpose".into(), AgentValue::string(purpose.value.clone()));
    }

    AgentValue::object(obj)
}
