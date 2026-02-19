use std::env;
#[cfg(feature = "image")]
use std::sync::Arc;
use std::sync::OnceLock;

use im::{Vector, hashmap};
use modular_agent_core::photon_rs::PhotonImage;
use modular_agent_core::{
    Agent, AgentContext, AgentData, AgentError, AgentOutput, AgentSpec, AgentValue, AsAgent,
    Message, ModularAgent, async_trait, modular_agent,
};
use slack_morphism::prelude::*;
use tokio::sync::mpsc;
use tracing::error;

use crate::mrkdwn;

static CATEGORY: &str = "Slack";

static PORT_RESULT: &str = "result";
static PORT_TRIGGER: &str = "trigger";
static PORT_MESSAGE: &str = "message";
static PORT_VALUE: &str = "value";
static PORT_VALUES: &str = "values";
static PORT_CHANNELS: &str = "channels";

static CONFIG_CHANNEL: &str = "channel";
static CONFIG_LIMIT: &str = "limit";
static CONFIG_CONVERT_MARKDOWN: &str = "convert_markdown";
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
    boolean_config(name = CONFIG_CONVERT_MARKDOWN, default = true),
    custom_global_config(name = CONFIG_SLACK_BOT_TOKEN, type_ = "password", default = AgentValue::string(""), title = "Slack Bot Token"),
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
        let convert = config.get_bool_or(CONFIG_CONVERT_MARKDOWN, true);

        let token = get_token(self.ma())?;
        let client = get_client();
        let session = client.open_session(&token);
        let channel_id: SlackChannelId = channel.clone().into();

        // Handle image upload
        #[cfg(feature = "image")]
        if let Some(image) = value.as_image() {
            let result = upload_image_to_slack(&session, image, &channel_id, None, None).await?;
            return self.output(ctx, PORT_RESULT, result).await;
        }

        // Handle Message with image
        #[cfg(feature = "image")]
        if let Some(msg) = value.as_message()
            && let Some(ref image) = msg.image
        {
            let initial_comment = if msg.content.is_empty() {
                None
            } else if convert {
                Some(mrkdwn::md_to_mrkdwn(&msg.content))
            } else {
                Some(msg.content.clone())
            };
            let result =
                upload_image_to_slack(&session, image, &channel_id, initial_comment, None).await?;
            return self.output(ctx, PORT_RESULT, result).await;
        }

        let (text, blocks, thread_ts) = extract_message_content(&value)?;
        let text = if convert {
            mrkdwn::md_to_mrkdwn(&text)
        } else {
            text
        };

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

#[cfg(feature = "image")]
async fn upload_image_to_slack(
    session: &SlackClientSession<'_, HyperConnector>,
    image: &PhotonImage,
    channel_id: &SlackChannelId,
    initial_comment: Option<String>,
    thread_ts: Option<String>,
) -> Result<AgentValue, AgentError> {
    use slack_morphism::api::{
        SlackApiFilesComplete, SlackApiFilesCompleteUploadExternalRequest,
        SlackApiFilesGetUploadUrlExternalRequest, SlackApiFilesUploadViaUrlRequest,
    };

    // Convert image to PNG bytes
    let png_bytes = image.get_bytes();
    let filename = format!("image_{}.png", chrono::Utc::now().timestamp_millis());

    // Step 1: Get upload URL
    let upload_url_request =
        SlackApiFilesGetUploadUrlExternalRequest::new(filename.clone(), png_bytes.len());

    let upload_url_response = session
        .get_upload_url_external(&upload_url_request)
        .await
        .map_err(|e| AgentError::IoError(format!("Failed to get upload URL: {}", e)))?;

    // Step 2: Upload file content
    let upload_request = SlackApiFilesUploadViaUrlRequest::new(
        upload_url_response.upload_url,
        png_bytes,
        "image/png".to_string(),
    );

    session
        .files_upload_via_url(&upload_request)
        .await
        .map_err(|e| AgentError::IoError(format!("Failed to upload file: {}", e)))?;

    // Step 3: Complete upload
    let file_complete = SlackApiFilesComplete::new(upload_url_response.file_id.clone());
    let mut complete_request = SlackApiFilesCompleteUploadExternalRequest::new(vec![file_complete])
        .with_channel_id(channel_id.clone());

    if let Some(comment) = initial_comment {
        complete_request = complete_request.with_initial_comment(comment);
    }

    if let Some(ts) = thread_ts {
        complete_request = complete_request.with_thread_ts(ts.into());
    }

    let complete_response = session
        .files_complete_upload_external(&complete_request)
        .await
        .map_err(|e| AgentError::IoError(format!("Failed to complete upload: {}", e)))?;

    let file_id = complete_response
        .files
        .first()
        .map(|f| f.id.to_string())
        .unwrap_or_default();

    Ok(AgentValue::object(hashmap! {
        "ok".into() => AgentValue::boolean(true),
        "file_id".into() => AgentValue::string(file_id),
        "channel".into() => AgentValue::string(channel_id.to_string()),
    }))
}

fn extract_message_content(
    value: &AgentValue,
) -> Result<(String, Option<AgentValue>, Option<String>), AgentError> {
    match value {
        AgentValue::String(s) => Ok((s.to_string(), None, None)),
        AgentValue::Message(msg) => Ok((msg.content.clone(), None, None)),
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
                .filter_map(|v| {
                    v.as_str()
                        .map(String::from)
                        .or_else(|| v.as_message().map(|m| m.content.clone()))
                })
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
/// - `values`: Array of Slack message objects containing `text`, `user`, `ts`, etc.
#[modular_agent(
    title = "History",
    category = CATEGORY,
    inputs = [PORT_TRIGGER],
    outputs = [PORT_VALUES],
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

        self.output(ctx, PORT_VALUES, AgentValue::array(messages))
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
    outputs = [PORT_CHANNELS],
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

        self.output(ctx, PORT_CHANNELS, AgentValue::array(channels))
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
/// - `value`: Slack Message objects containing `text`, `user`, `channel`, `ts`, `thread_ts` fields
///
/// # Required Tokens
/// - `SLACK_BOT_TOKEN`: Bot User OAuth Token (via global config or environment)
/// - `SLACK_APP_TOKEN`: App-Level Token with `connections:write` scope (via global config or environment)
#[modular_agent(
    title = "Listener",
    category = CATEGORY,
    outputs = [PORT_VALUE],
    string_config(name = CONFIG_CHANNEL),
    custom_global_config(name = CONFIG_SLACK_APP_TOKEN, type_ = "password", default = AgentValue::string(""), title = "Slack App Token"),
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
    bot_token: String,
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
        let bot_token_str = bot_token.token_value.0.clone();

        tokio::spawn(async move {
            let user_state = SlackListenerUserState {
                ma,
                id,
                channel_filter,
                bot_user_id,
                bot_token: bot_token_str,
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

        // Clone necessary data before releasing the lock
        let bot_user_id = state.bot_user_id.clone();
        let bot_token = state.bot_token.clone();
        let ma = state.ma.clone();
        let id = state.id.clone();

        // Check if bot's own message
        if let Some(ref user) = msg_event.sender.user {
            if user == &bot_user_id {
                return Ok(());
            }
        }

        // Download image if present
        #[cfg(feature = "image")]
        let image = download_first_image(&msg_event, &bot_token).await;
        #[cfg(not(feature = "image"))]
        let image: Option<PhotonImage> = None;

        if let Some(message) = slack_push_message_to_agent_value(&msg_event, image) {
            if let Err(e) = ma.try_send_agent_out(
                id,
                AgentContext::new(),
                PORT_VALUE.to_string(),
                message,
            ) {
                error!("Failed to output message: {}", e);
            }
        }
    }

    Ok(())
}

#[cfg(feature = "image")]
async fn download_first_image(msg: &SlackMessageEvent, bot_token: &str) -> Option<PhotonImage> {
    let files = msg.content.as_ref()?.files.as_ref()?;

    for file in files {
        let mimetype = file.mimetype.as_ref()?;
        if !mimetype.0.starts_with("image/") {
            continue;
        }

        let url = file.url_private_download.as_ref().or(file.url_private.as_ref())?;

        match download_slack_file(url.as_str(), bot_token).await {
            Ok(bytes) => {
                let image = PhotonImage::new_from_byteslice(bytes);
                return Some(image);
            }
            Err(e) => {
                error!("Failed to download image: {}", e);
                continue;
            }
        }
    }

    None
}

#[cfg(feature = "image")]
async fn download_slack_file(url: &str, bot_token: &str) -> Result<Vec<u8>, AgentError> {
    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .header("Authorization", format!("Bearer {}", bot_token))
        .send()
        .await
        .map_err(|e| AgentError::IoError(format!("Failed to fetch file: {}", e)))?;

    if !response.status().is_success() {
        return Err(AgentError::IoError(format!(
            "Failed to download file: HTTP {}",
            response.status()
        )));
    }

    response
        .bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| AgentError::IoError(format!("Failed to read file bytes: {}", e)))
}

fn slack_push_message_to_agent_value(
    msg: &SlackMessageEvent,
    #[allow(unused_variables)] image: Option<PhotonImage>,
) -> Option<AgentValue> {
    let text = msg
        .content
        .as_ref()
        .and_then(|c| c.text.clone())
        .unwrap_or_default();

    let channel = msg
        .origin
        .channel
        .as_ref()
        .map(|c| c.to_string())
        .unwrap_or_default();

    let ts = msg.origin.ts.to_string();

    let thread_ts = msg.origin.thread_ts.as_ref().map(|t| t.to_string());

    let user = msg.sender.user.as_ref().map(|u| u.to_string());

    #[cfg(feature = "image")]
    {
        let mut message = Message::user(text);
        message.image = image.map(Arc::new);

        let mut obj = im::HashMap::new();
        obj.insert("message".into(), AgentValue::message(message));
        if let Some(user) = user {
            obj.insert("user".into(), AgentValue::string(user));
        }
        obj.insert("channel".into(), AgentValue::string(channel));
        obj.insert("ts".into(), AgentValue::string(ts));
        if let Some(thread_ts) = thread_ts {
            obj.insert("thread_ts".into(), AgentValue::string(thread_ts));
        }
        Some(AgentValue::object(obj))
    }

    #[cfg(not(feature = "image"))]
    {
        let message = Message::user(text);

        let mut obj = im::HashMap::new();
        obj.insert("message".into(), AgentValue::message(message));
        if let Some(user) = user {
            obj.insert("user".into(), AgentValue::string(user));
        }
        obj.insert("channel".into(), AgentValue::string(channel));
        obj.insert("ts".into(), AgentValue::string(ts));
        if let Some(thread_ts) = thread_ts {
            obj.insert("thread_ts".into(), AgentValue::string(thread_ts));
        }
        Some(AgentValue::object(obj))
    }
}

/// Agent for converting Slack messages to LLM Message format.
///
/// Converts Slack message objects (with `text`, `user`, `channel`, `ts` fields)
/// into AgentValue::Message format suitable for LLM agents.
///
/// # Input
/// - `value`: Single Slack message object or array of Slack message objects
///
/// # Output
/// - `message`: AgentValue::Message or array of AgentValue::Message
#[modular_agent(
    title = "ToMessage",
    category = CATEGORY,
    inputs = [PORT_VALUE],
    outputs = [PORT_MESSAGE],
)]
struct SlackToMessageAgent {
    data: AgentData,
}

#[async_trait]
impl AsAgent for SlackToMessageAgent {
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
        if value.is_array() {
            let arr = value.as_array().unwrap();
            let messages: im::Vector<AgentValue> = arr
                .iter()
                .filter_map(|v| slack_value_to_message(v).ok())
                .map(AgentValue::message)
                .collect();
            self.output(ctx, PORT_MESSAGE, AgentValue::array(messages))
                .await
        } else {
            let message = slack_value_to_message(&value)?;
            self.output(ctx, PORT_MESSAGE, AgentValue::message(message))
                .await
        }
    }
}

fn slack_value_to_message(value: &AgentValue) -> Result<Message, AgentError> {
    match value {
        AgentValue::String(s) => Ok(Message::user(s.to_string())),
        AgentValue::Message(msg) => Ok(Message::clone(msg)),
        AgentValue::Object(obj) => {
            // New format: check for "message" field first
            if let Some(msg) = obj.get("message").and_then(|v| v.as_message()) {
                return Ok(Message::clone(msg));
            }
            // Legacy format: use "text" field
            let text = obj
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(Message::user(text))
        }
        _ => Err(AgentError::InvalidValue(
            "Expected string, message, or object for Slack message".to_string(),
        )),
    }
}
