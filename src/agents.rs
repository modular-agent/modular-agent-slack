use std::env;
use std::sync::{Arc, OnceLock};

use im::{Vector, hashmap};
use modular_agent_core::{
    Agent, AgentContext, AgentData, AgentError, AgentOutput, AgentSpec, AgentValue, AsAgent,
    ModularAgent, async_trait, modular_agent,
};
use slack_morphism::prelude::*;
use tokio::sync::mpsc;
use tracing::error;

static CATEGORY: &str = "Slack";

static PORT_MESSAGE: &str = "message";
static PORT_RESULT: &str = "result";
static PORT_TRIGGER: &str = "trigger";
static PORT_MESSAGES: &str = "messages";

static CONFIG_CHANNEL: &str = "channel";
static CONFIG_LIMIT: &str = "limit";
static CONFIG_SLACK_BOT_TOKEN: &str = "slack_bot_token";
static CONFIG_SLACK_APP_TOKEN: &str = "slack_app_token";

type HyperConnector = SlackClientHyperConnector<SlackHyperHttpsConnector>;

static CLIENT: OnceLock<SlackClient<HyperConnector>> = OnceLock::new();

fn get_client() -> &'static SlackClient<HyperConnector> {
    CLIENT.get_or_init(|| {
        rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .expect("Failed to initialize rustls crypto provider");
        SlackClient::new(
            SlackClientHyperConnector::new().expect("Failed to create Slack client HTTP connector"),
        )
    })
}

fn get_token(ma: &ModularAgent) -> Result<SlackApiToken, AgentError> {
    let token_str = if let Some(global_token) = ma
        .get_global_configs(SlackPostAgent::DEF_NAME)
        .and_then(|cfg| cfg.get_string(CONFIG_SLACK_BOT_TOKEN).ok())
        .filter(|key| !key.is_empty())
    {
        global_token
    } else {
        env::var("SLACK_BOT_TOKEN")
            .map_err(|_| AgentError::InvalidValue("SLACK_BOT_TOKEN not set".to_string()))?
    };

    Ok(SlackApiToken::new(SlackApiTokenValue(token_str)))
}

fn get_app_token(ma: &ModularAgent) -> Result<SlackApiToken, AgentError> {
    let token_str = if let Some(global_token) = ma
        .get_global_configs(SlackListenerAgent::DEF_NAME)
        .and_then(|cfg| cfg.get_string(CONFIG_SLACK_APP_TOKEN).ok())
        .filter(|key| !key.is_empty())
    {
        global_token
    } else {
        env::var("SLACK_APP_TOKEN")
            .map_err(|_| AgentError::InvalidValue("SLACK_APP_TOKEN not set".to_string()))?
    };

    Ok(SlackApiToken::new(SlackApiTokenValue(token_str)))
}

/// Agent for posting messages to Slack channels.
///
/// # Configuration
/// - `channel`: The Slack channel name (e.g., "#general") or channel ID
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
    string_global_config(name = CONFIG_SLACK_BOT_TOKEN, title = "Slack Bot Token"),
)]
struct SlackPostAgent {
    data: AgentData,
}

#[async_trait]
impl AsAgent for SlackPostAgent {
    fn new(ma: ModularAgent, id: String, spec: AgentSpec) -> Result<Self, AgentError> {
        Ok(Self {
            data: AgentData::new(ma, id, spec),
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

        let token = get_token(self.ma())?;
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
    integer_config(name = CONFIG_LIMIT),
)]
struct SlackHistoryAgent {
    data: AgentData,
}

#[async_trait]
impl AsAgent for SlackHistoryAgent {
    fn new(ma: ModularAgent, id: String, spec: AgentSpec) -> Result<Self, AgentError> {
        Ok(Self {
            data: AgentData::new(ma, id, spec),
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

        let token = get_token(self.ma())?;
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
    integer_config(name = CONFIG_LIMIT),
)]
struct SlackChannelsAgent {
    data: AgentData,
}

#[async_trait]
impl AsAgent for SlackChannelsAgent {
    fn new(ma: ModularAgent, id: String, spec: AgentSpec) -> Result<Self, AgentError> {
        Ok(Self {
            data: AgentData::new(ma, id, spec),
        })
    }

    async fn process(
        &mut self,
        ctx: AgentContext,
        _port: String,
        _value: AgentValue,
    ) -> Result<(), AgentError> {
        let config = self.configs()?;
        let token = get_token(self.ma())?;
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

/// Agent for listening to Slack messages in real-time via Socket Mode.
///
/// This agent starts listening when activated and outputs messages as they arrive.
///
/// # Configuration
/// - `channel`: Optional channel filter. If empty, listens to all channels.
///
/// # Output
/// - `message`: Message objects containing `text`, `user`, `channel`, `ts`, `thread_ts` fields
///
/// # Required Tokens
/// - `SLACK_BOT_TOKEN`: Bot User OAuth Token (via global config or environment)
/// - `SLACK_APP_TOKEN`: App-Level Token with `connections:write` scope (via global config or environment)
#[modular_agent(
    title = "Listener",
    category = CATEGORY,
    outputs = [PORT_MESSAGE],
    string_config(name = CONFIG_CHANNEL),
    string_global_config(name = CONFIG_SLACK_APP_TOKEN, title = "Slack App Token"),
)]
struct SlackListenerAgent {
    data: AgentData,
    shutdown_tx: Option<mpsc::Sender<()>>,
}

struct SlackListenerUserState {
    ma: ModularAgent,
    id: String,
    channel_filter: Option<String>,
    bot_user_id: SlackUserId,
}

#[async_trait]
impl AsAgent for SlackListenerAgent {
    fn new(ma: ModularAgent, id: String, spec: AgentSpec) -> Result<Self, AgentError> {
        Ok(Self {
            data: AgentData::new(ma, id, spec),
            shutdown_tx: None,
        })
    }

    async fn start(&mut self) -> Result<(), AgentError> {
        let client = Arc::new(get_client().clone());

        let bot_token = get_token(self.ma())?;
        let bot_session = client.open_session(&bot_token);
        let bot_user_id = bot_session
            .auth_test()
            .await
            .map_err(|e| AgentError::IoError(format!("Slack API error during auth_test: {}", e)))?
            .user_id;

        let config = self.configs()?;
        let channel_filter = config.get_string_or_default(CONFIG_CHANNEL);
        let channel_filter = if channel_filter.is_empty() {
            None
        } else {
            Some(channel_filter)
        };

        let app_token = get_app_token(self.ma())?;

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);

        let ma = self.ma().clone();
        let id = self.id().to_string();

        tokio::spawn(async move {
            let user_state = SlackListenerUserState {
                ma,
                id,
                channel_filter,
                bot_user_id,
            };

            let listener_environment = Arc::new(
                SlackClientEventsListenerEnvironment::new(client.clone())
                    .with_user_state(user_state),
            );

            let socket_mode_callbacks =
                SlackSocketModeListenerCallbacks::new().with_push_events(push_events_handler);

            let socket_mode_listener = SlackClientSocketModeListener::new(
                &SlackClientSocketModeConfig::new(),
                listener_environment,
                socket_mode_callbacks,
            );

            if let Err(e) = socket_mode_listener.listen_for(&app_token).await {
                error!("Socket mode listener failed to start: {}", e);
                return;
            }

            socket_mode_listener.start().await;

            // Wait for shutdown signal instead of using serve() which sets its own Ctrl-C handler
            shutdown_rx.recv().await;

            socket_mode_listener.shutdown().await;
        });

        Ok(())
    }

    async fn stop(&mut self) -> Result<(), AgentError> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(()).await;
        }
        Ok(())
    }
}

async fn push_events_handler(
    event: SlackPushEventCallback,
    _client: Arc<SlackHyperClient>,
    states: SlackClientEventsUserState,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let SlackEventCallbackBody::Message(msg_event) = event.event {
        let storage = states.read().await;
        let Some(state) = storage.get_user_state::<SlackListenerUserState>() else {
            error!("SlackListenerUserState not found in storage");
            return Ok(());
        };

        // Apply channel filter if configured
        if let Some(ref filter) = state.channel_filter {
            if let Some(ref channel) = msg_event.origin.channel {
                let channel_str = channel.to_string();
                if channel_str != *filter && !filter.ends_with(&channel_str) {
                    return Ok(());
                }
            }
        }

        if let Some(message) = slack_push_message_to_agent_value(state, &msg_event) {
            if let Err(e) = state.ma.try_send_agent_out(
                state.id.clone(),
                AgentContext::new(),
                PORT_MESSAGE.to_string(),
                message,
            ) {
                error!("Failed to output message: {}", e);
            }
        }
    }

    Ok(())
}

fn slack_push_message_to_agent_value(
    state: &SlackListenerUserState,
    msg: &SlackMessageEvent,
) -> Option<AgentValue> {
    let mut obj = im::HashMap::new();

    if let Some(ref user) = msg.sender.user {
        if user == &state.bot_user_id {
            // Ignore messages sent by the bot itself
            return None;
        }
        obj.insert("user".into(), AgentValue::string(user.to_string()));
    }

    if let Some(ref content) = msg.content {
        if let Some(ref text) = content.text {
            obj.insert("text".into(), AgentValue::string(text.clone()));
        }
    }

    if let Some(ref channel) = msg.origin.channel {
        obj.insert("channel".into(), AgentValue::string(channel.to_string()));
    }

    obj.insert("ts".into(), AgentValue::string(msg.origin.ts.to_string()));

    if let Some(ref thread_ts) = msg.origin.thread_ts {
        obj.insert(
            "thread_ts".into(),
            AgentValue::string(thread_ts.to_string()),
        );
    }

    Some(AgentValue::object(obj))
}
