use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{
    error::{AppError, AppResult},
    models::Usage,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientProtocol {
    OpenAiChatCompletions,
    OpenAiResponses,
    AnthropicMessages,
    GeminiGenerateContent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderProtocol {
    OpenAiResponses,
    AnthropicMessages,
    GeminiGenerateContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextRequest {
    pub model: String,
    pub messages: Vec<TextMessage>,
    pub system: Option<String>,
    pub max_tokens: Option<i64>,
    pub temperature: Option<f64>,
    pub stream: bool,
    pub tools: Option<Value>,
    pub tool_choice: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextMessage {
    pub role: String,
    pub content: Vec<MessagePart>,
    pub tool_calls: Option<Value>,
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessagePart {
    Text(String),
    Image(ImageInput),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageInput {
    pub source: ImageSource,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ImageSource {
    Url(String),
    Base64 {
        media_type: String,
        data: String,
    },
    FileId(String),
    FileUri {
        file_uri: String,
        media_type: Option<String>,
    },
}

impl TextRequest {
    pub fn estimated_input_tokens(&self) -> i64 {
        crate::tokenizer::estimate_request_tokens(self).tokens
    }
}

pub fn parse_client_request(protocol: ClientProtocol, body: &Value) -> AppResult<TextRequest> {
    match protocol {
        ClientProtocol::OpenAiChatCompletions => parse_openai_chat_completions(body),
        ClientProtocol::OpenAiResponses => parse_openai_responses(body),
        ClientProtocol::AnthropicMessages => parse_anthropic_messages(body),
        ClientProtocol::GeminiGenerateContent => parse_gemini_generate_content(body),
    }
}

pub fn provider_protocol(provider: &crate::models::ProviderKind) -> ProviderProtocol {
    match provider {
        crate::models::ProviderKind::OpenAi => ProviderProtocol::OpenAiResponses,
        crate::models::ProviderKind::Anthropic => ProviderProtocol::AnthropicMessages,
        crate::models::ProviderKind::Gemini => ProviderProtocol::GeminiGenerateContent,
    }
}

pub fn same_wire_protocol(client: ClientProtocol, provider: ProviderProtocol) -> bool {
    matches!(
        (client, provider),
        (
            ClientProtocol::OpenAiResponses,
            ProviderProtocol::OpenAiResponses
        ) | (
            ClientProtocol::AnthropicMessages,
            ProviderProtocol::AnthropicMessages
        ) | (
            ClientProtocol::GeminiGenerateContent,
            ProviderProtocol::GeminiGenerateContent
        )
    )
}

pub fn upstream_path(provider: ProviderProtocol, model: &str, stream: bool) -> String {
    match provider {
        ProviderProtocol::OpenAiResponses => "/v1/responses".to_string(),
        ProviderProtocol::AnthropicMessages => "/v1/messages".to_string(),
        ProviderProtocol::GeminiGenerateContent => {
            let action = if stream {
                "streamGenerateContent?alt=sse"
            } else {
                "generateContent"
            };
            format!("/v1beta/models/{model}:{action}")
        }
    }
}

pub fn upstream_body(
    client_protocol: ClientProtocol,
    provider_protocol: ProviderProtocol,
    raw_body: &Value,
    request: &TextRequest,
) -> AppResult<Value> {
    if same_wire_protocol(client_protocol, provider_protocol) {
        return Ok(strip_internal_fields(raw_body));
    }
    match provider_protocol {
        ProviderProtocol::OpenAiResponses => text_to_openai_responses(request),
        ProviderProtocol::AnthropicMessages => text_to_anthropic_messages(request),
        ProviderProtocol::GeminiGenerateContent => text_to_gemini_generate_content(request),
    }
}

pub fn client_response_body(
    client_protocol: ClientProtocol,
    provider_protocol: ProviderProtocol,
    value: Value,
) -> (Value, Usage) {
    let usage = extract_usage(&value);
    if same_wire_protocol(client_protocol, provider_protocol) {
        return (value, usage);
    }
    match client_protocol {
        ClientProtocol::OpenAiChatCompletions => {
            response_to_chat_completions(value, provider_protocol, usage)
        }
        ClientProtocol::OpenAiResponses => {
            response_to_openai_responses(value, provider_protocol, usage)
        }
        ClientProtocol::AnthropicMessages => response_to_anthropic(value, provider_protocol, usage),
        ClientProtocol::GeminiGenerateContent => {
            response_to_gemini(value, provider_protocol, usage)
        }
    }
}

pub fn response_has_semantic_content(value: &Value, provider_protocol: ProviderProtocol) -> bool {
    semantic_response_text(value, provider_protocol).is_some_and(|text| !text.trim().is_empty())
        || response_has_tool_content(value, provider_protocol)
}

pub fn stream_chunk_has_semantic_content(
    bytes: &bytes::Bytes,
    provider_protocol: ProviderProtocol,
) -> bool {
    let text = String::from_utf8_lossy(bytes);
    text.lines().any(|line| {
        let Some(data) = line.strip_prefix("data:") else {
            return false;
        };
        let data = data.trim();
        if data == "[DONE]" || data.is_empty() {
            return false;
        }
        serde_json::from_str::<Value>(data)
            .ok()
            .is_some_and(|value| stream_event_has_semantic_content(&value, provider_protocol))
    })
}

fn parse_openai_chat_completions(value: &Value) -> AppResult<TextRequest> {
    let model = required_string(value, "model", "chat completions request requires model")?;
    let stream = value
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let max_tokens = value
        .get("max_completion_tokens")
        .or_else(|| value.get("max_tokens"))
        .and_then(Value::as_i64);
    let temperature = value.get("temperature").and_then(Value::as_f64);
    let mut system_parts = Vec::new();
    let mut messages = Vec::new();
    for message in value
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AppError::BadRequest("chat completions request requires messages[]".to_string())
        })?
    {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user")
            .to_string();
        let content = openai_content_to_parts(message.get("content"))?;
        if role == "system" {
            system_parts.push(parts_to_text_only(
                &content,
                "chat completions system message",
            )?);
            continue;
        }
        messages.push(TextMessage {
            role,
            content,
            tool_calls: message.get("tool_calls").cloned(),
            tool_call_id: message
                .get("tool_call_id")
                .and_then(Value::as_str)
                .map(ToString::to_string),
        });
    }
    Ok(TextRequest {
        model,
        messages,
        system: non_empty_join(system_parts),
        max_tokens,
        temperature,
        stream,
        tools: value.get("tools").cloned(),
        tool_choice: value.get("tool_choice").cloned(),
    })
}

fn parse_openai_responses(value: &Value) -> AppResult<TextRequest> {
    let model = required_string(value, "model", "responses request requires model")?;
    let stream = value
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let max_tokens = value
        .get("max_output_tokens")
        .or_else(|| value.get("max_tokens"))
        .and_then(Value::as_i64);
    let temperature = value.get("temperature").and_then(Value::as_f64);
    let messages = parse_responses_input(value.get("input"))?;
    Ok(TextRequest {
        model,
        messages,
        system: value
            .get("instructions")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        max_tokens,
        temperature,
        stream,
        tools: value.get("tools").cloned(),
        tool_choice: value.get("tool_choice").cloned(),
    })
}

fn parse_anthropic_messages(value: &Value) -> AppResult<TextRequest> {
    let model = required_string(value, "model", "messages request requires model")?;
    let stream = value
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let max_tokens = value.get("max_tokens").and_then(Value::as_i64);
    let temperature = value.get("temperature").and_then(Value::as_f64);
    let system = match value.get("system") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(items)) => non_empty_join(
            items
                .iter()
                .map(|item| {
                    let parts = anthropic_content_to_parts(Some(item))?;
                    parts_to_text_only(&parts, "anthropic system message")
                })
                .collect::<AppResult<Vec<_>>>()?,
        ),
        _ => None,
    };
    let messages = value
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::BadRequest("messages request requires messages[]".to_string()))?
        .iter()
        .map(|message| {
            let role = message
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("user")
                .to_string();
            Ok(TextMessage {
                role,
                content: anthropic_content_to_parts(message.get("content"))?,
                tool_calls: None,
                tool_call_id: None,
            })
        })
        .collect::<AppResult<Vec<_>>>()?;
    Ok(TextRequest {
        model,
        messages,
        system,
        max_tokens,
        temperature,
        stream,
        tools: value.get("tools").cloned(),
        tool_choice: value.get("tool_choice").cloned(),
    })
}

fn parse_gemini_generate_content(value: &Value) -> AppResult<TextRequest> {
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("contents")
                .and_then(Value::as_array)
                .and_then(|_| value.get("_model").and_then(Value::as_str))
        })
        .unwrap_or("gemini")
        .to_string();
    let generation_config = value.get("generationConfig");
    let stream = value
        .get("stream")
        .or_else(|| value.get("_stream"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let max_tokens = generation_config
        .and_then(|config| config.get("maxOutputTokens"))
        .and_then(Value::as_i64);
    let temperature = generation_config
        .and_then(|config| config.get("temperature"))
        .and_then(Value::as_f64);
    let system = value
        .get("systemInstruction")
        .and_then(|item| {
            let parts = gemini_parts_to_message_parts(item.get("parts")).ok()?;
            parts_to_text_only(&parts, "gemini system instruction").ok()
        })
        .filter(|text| !text.is_empty());
    let messages = value
        .get("contents")
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::BadRequest("gemini request requires contents[]".to_string()))?
        .iter()
        .map(|content| {
            let role = match content.get("role").and_then(Value::as_str) {
                Some("model") => "assistant",
                Some(role) => role,
                None => "user",
            };
            Ok(TextMessage {
                role: role.to_string(),
                content: gemini_parts_to_message_parts(content.get("parts"))?,
                tool_calls: None,
                tool_call_id: None,
            })
        })
        .collect::<AppResult<Vec<_>>>()?;
    Ok(TextRequest {
        model,
        messages,
        system,
        max_tokens,
        temperature,
        stream,
        tools: value.get("tools").cloned(),
        tool_choice: None,
    })
}

fn strip_internal_fields(value: &Value) -> Value {
    let mut clean = value.clone();
    if let Some(object) = clean.as_object_mut() {
        object.remove("_model");
        object.remove("_stream");
    }
    clean
}

fn text_to_openai_responses(request: &TextRequest) -> AppResult<Value> {
    let mut input = Vec::new();
    for message in &request.messages {
        if message.role == "tool" {
            input.push(json!({
                "type": "function_call_output",
                "call_id": message.tool_call_id.clone().unwrap_or_default(),
                "output": message_text_content(&message.content),
            }));
        } else if let Some(tool_calls) = &message.tool_calls {
            input.push(json!({
                "type": "function_call",
                "role": message.role,
                "call_id": tool_calls.get("id").and_then(Value::as_str).unwrap_or("call"),
                "name": tool_calls.get("function").and_then(|f| f.get("name")).and_then(Value::as_str).unwrap_or("tool"),
                "arguments": tool_calls.get("function").and_then(|f| f.get("arguments")).cloned().unwrap_or(Value::String("{}".to_string())),
            }));
        } else {
            let text_type = if message.role == "assistant" {
                "output_text"
            } else {
                "input_text"
            };
            let content = openai_responses_content_items(&message.content, text_type)?;
            input.push(json!({
                "role": message.role,
                "content": content,
            }));
        }
    }
    let mut body = json!({
        "model": request.model,
        "input": input,
        "stream": request.stream,
    });
    if let Some(system) = &request.system {
        body["instructions"] = Value::String(system.clone());
    }
    if let Some(max_tokens) = request.max_tokens {
        body["max_output_tokens"] = Value::Number(max_tokens.into());
    }
    if let Some(temperature) = request.temperature {
        body["temperature"] = json!(temperature);
    }
    if let Some(tools) = &request.tools {
        body["tools"] = tools.clone();
    }
    if let Some(tool_choice) = &request.tool_choice {
        body["tool_choice"] = tool_choice.clone();
    }
    Ok(body)
}

fn text_to_anthropic_messages(request: &TextRequest) -> AppResult<Value> {
    let messages = request
        .messages
        .iter()
        .map(|message| -> AppResult<Value> {
            let content = anthropic_content_items(&message.content)?;
            Ok(json!({
                "role": if message.role == "assistant" { "assistant" } else { "user" },
                "content": content,
            }))
        })
        .collect::<AppResult<Vec<_>>>()?;
    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "max_tokens": request.max_tokens.unwrap_or(1024),
        "stream": request.stream,
    });
    if let Some(system) = &request.system {
        body["system"] = Value::String(system.clone());
    }
    if let Some(temperature) = request.temperature {
        body["temperature"] = json!(temperature);
    }
    if let Some(tools) = &request.tools {
        body["tools"] = openai_tools_to_anthropic(tools);
    }
    if let Some(tool_choice) = &request.tool_choice {
        body["tool_choice"] = tool_choice.clone();
    }
    Ok(body)
}

fn text_to_gemini_generate_content(request: &TextRequest) -> AppResult<Value> {
    let contents = request
        .messages
        .iter()
        .map(|message| -> AppResult<Value> {
            Ok(json!({
                "role": if message.role == "assistant" { "model" } else { "user" },
                "parts": gemini_content_parts(&message.content)?,
            }))
        })
        .collect::<AppResult<Vec<_>>>()?;
    let mut generation_config = Map::new();
    if let Some(max_tokens) = request.max_tokens {
        generation_config.insert(
            "maxOutputTokens".to_string(),
            Value::Number(max_tokens.into()),
        );
    }
    if let Some(temperature) = request.temperature {
        generation_config.insert("temperature".to_string(), json!(temperature));
    }
    let mut body = json!({
        "contents": contents,
    });
    if !generation_config.is_empty() {
        body["generationConfig"] = Value::Object(generation_config);
    }
    if let Some(system) = &request.system {
        body["systemInstruction"] = json!({
            "parts": [{"text": system}],
        });
    }
    if let Some(tools) = &request.tools {
        body["tools"] = openai_tools_to_gemini(tools);
    }
    Ok(body)
}

fn response_to_chat_completions(
    value: Value,
    provider_protocol: ProviderProtocol,
    usage: Usage,
) -> (Value, Usage) {
    let text = output_text(&value, provider_protocol);
    let prompt_tokens = usage.input_tokens + usage.cache_tokens;
    (
        json!({
            "id": value.get("id").cloned().unwrap_or_else(|| json!("chatcmpl_local")),
            "object": "chat.completion",
            "model": value.get("model").cloned().unwrap_or(Value::Null),
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": text},
                "finish_reason": finish_reason(&value, provider_protocol)
            }],
            "usage": {
                "prompt_tokens": prompt_tokens,
                "completion_tokens": usage.output_tokens,
                "total_tokens": usage.total(),
                "prompt_tokens_details": {
                    "cached_tokens": usage.cache_tokens
                }
            }
        }),
        usage,
    )
}

fn response_to_openai_responses(
    value: Value,
    provider_protocol: ProviderProtocol,
    usage: Usage,
) -> (Value, Usage) {
    let text = output_text(&value, provider_protocol);
    let input_tokens = usage.input_tokens + usage.cache_tokens;
    (
        json!({
            "id": value.get("id").cloned().unwrap_or_else(|| json!("resp_local")),
            "object": "response",
            "model": value.get("model").cloned().unwrap_or(Value::Null),
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": text}]
            }],
            "usage": {
                "input_tokens": input_tokens,
                "output_tokens": usage.output_tokens,
                "total_tokens": usage.total(),
                "input_tokens_details": {
                    "cached_tokens": usage.cache_tokens
                }
            }
        }),
        usage,
    )
}

fn response_to_anthropic(
    value: Value,
    provider_protocol: ProviderProtocol,
    usage: Usage,
) -> (Value, Usage) {
    (
        json!({
            "id": value.get("id").cloned().unwrap_or_else(|| json!("msg_local")),
            "type": "message",
            "role": "assistant",
            "model": value.get("model").cloned().unwrap_or(Value::Null),
            "content": [{"type": "text", "text": output_text(&value, provider_protocol)}],
            "stop_reason": anthropic_stop_reason(&value, provider_protocol),
            "usage": {
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
                "cache_read_input_tokens": usage.cache_tokens,
            }
        }),
        usage,
    )
}

fn response_to_gemini(
    value: Value,
    provider_protocol: ProviderProtocol,
    usage: Usage,
) -> (Value, Usage) {
    let prompt_tokens = usage.input_tokens + usage.cache_tokens;
    (
        json!({
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{"text": output_text(&value, provider_protocol)}]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": prompt_tokens,
                "candidatesTokenCount": usage.output_tokens,
                "totalTokenCount": usage.total(),
                "cachedContentTokenCount": usage.cache_tokens
            }
        }),
        usage,
    )
}

pub fn extract_usage(value: &Value) -> Usage {
    if value.get("usageMetadata").is_some() {
        return parse_usage_fields(
            value.get("usageMetadata").unwrap_or(value),
            CacheTokenShape::IncludedInInput,
        );
    }
    if let Some(response_usage) = value
        .get("response")
        .and_then(|response| response.get("usage"))
    {
        return parse_usage_fields(response_usage, CacheTokenShape::Auto);
    }
    if let Some(message_usage) = value
        .get("message")
        .and_then(|message| message.get("usage"))
    {
        return parse_usage_fields(message_usage, CacheTokenShape::Auto);
    }
    if let Some(usage) = value.get("usage") {
        return parse_usage_fields(usage, CacheTokenShape::Auto);
    }
    parse_usage_fields(value, CacheTokenShape::Auto)
}

#[derive(Debug, Clone, Copy)]
enum CacheTokenShape {
    Auto,
    IncludedInInput,
}

fn parse_usage_fields(usage: &Value, cache_shape: CacheTokenShape) -> Usage {
    let raw_input_tokens = token_field(
        usage,
        &["input_tokens", "prompt_tokens", "promptTokenCount"],
    );
    let output_tokens = token_field(
        usage,
        &["output_tokens", "completion_tokens", "candidatesTokenCount"],
    );
    let detail_cache_tokens = usage
        .get("input_tokens_details")
        .or_else(|| usage.get("prompt_tokens_details"))
        .map(|details| token_field(details, &["cached_tokens"]))
        .unwrap_or(0);
    let direct_cache_tokens = token_field(
        usage,
        &[
            "cache_read_input_tokens",
            "cache_creation_input_tokens",
            "cached_tokens",
            "cachedContentTokenCount",
        ],
    );
    let cache_tokens = detail_cache_tokens.max(direct_cache_tokens);
    let cache_included_in_input = detail_cache_tokens > 0
        || matches!(cache_shape, CacheTokenShape::IncludedInInput)
        || usage.get("cachedContentTokenCount").is_some();
    let input_tokens = if cache_included_in_input {
        raw_input_tokens.saturating_sub(cache_tokens)
    } else {
        raw_input_tokens
    };
    Usage {
        input_tokens,
        output_tokens,
        cache_tokens,
    }
}

fn token_field(value: &Value, keys: &[&str]) -> i64 {
    keys.iter()
        .filter_map(|key| value.get(*key))
        .find_map(|value| {
            value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|tokens| i64::try_from(tokens).ok()))
        })
        .unwrap_or(0)
        .max(0)
}

pub fn translate_stream_chunk(
    bytes: bytes::Bytes,
    provider_protocol: ProviderProtocol,
    client_protocol: ClientProtocol,
    model: &str,
) -> bytes::Bytes {
    if same_wire_protocol(client_protocol, provider_protocol) {
        return bytes;
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut translated = String::new();
    let mut changed = false;
    for line in text.lines() {
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim();
            if data == "[DONE]" || data.is_empty() {
                translated.push_str(line);
                translated.push('\n');
                continue;
            }
            if let Ok(value) = serde_json::from_str::<Value>(data)
                && let Some(mapped) =
                    stream_event_to_client(&value, provider_protocol, client_protocol, model)
            {
                translated.push_str("data: ");
                translated.push_str(&mapped.to_string());
                translated.push_str("\n\n");
                changed = true;
                continue;
            }
        }
        translated.push_str(line);
        translated.push('\n');
    }
    if changed {
        bytes::Bytes::from(translated)
    } else {
        bytes
    }
}

fn stream_event_to_client(
    value: &Value,
    provider_protocol: ProviderProtocol,
    client_protocol: ClientProtocol,
    model: &str,
) -> Option<Value> {
    let delta = stream_text_delta(value, provider_protocol)?;
    match client_protocol {
        ClientProtocol::OpenAiChatCompletions => Some(json!({
            "object": "chat.completion.chunk",
            "model": model,
            "choices": [{"index": 0, "delta": {"content": delta}, "finish_reason": null}]
        })),
        ClientProtocol::OpenAiResponses => Some(json!({
            "type": "response.output_text.delta",
            "delta": delta,
        })),
        ClientProtocol::AnthropicMessages => Some(json!({
            "type": "content_block_delta",
            "delta": {"type": "text_delta", "text": delta}
        })),
        ClientProtocol::GeminiGenerateContent => Some(json!({
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{"text": delta}]
                }
            }]
        })),
    }
}

fn semantic_response_text(value: &Value, provider_protocol: ProviderProtocol) -> Option<String> {
    match provider_protocol {
        ProviderProtocol::OpenAiResponses => {
            openai_output_text(value).or_else(|| openai_refusal_text(value))
        }
        ProviderProtocol::AnthropicMessages => anthropic_output_text(value),
        ProviderProtocol::GeminiGenerateContent => gemini_output_text(value),
    }
}

fn response_has_tool_content(value: &Value, provider_protocol: ProviderProtocol) -> bool {
    match provider_protocol {
        ProviderProtocol::OpenAiResponses => openai_response_has_tool_content(value),
        ProviderProtocol::AnthropicMessages => anthropic_response_has_tool_content(value),
        ProviderProtocol::GeminiGenerateContent => gemini_response_has_tool_content(value),
    }
}

fn stream_event_has_semantic_content(value: &Value, provider_protocol: ProviderProtocol) -> bool {
    stream_text_delta(value, provider_protocol).is_some_and(|delta| !delta.trim().is_empty())
        || stream_event_has_tool_content(value, provider_protocol)
}

fn stream_event_has_tool_content(value: &Value, provider_protocol: ProviderProtocol) -> bool {
    match provider_protocol {
        ProviderProtocol::OpenAiResponses => openai_stream_event_has_tool_content(value),
        ProviderProtocol::AnthropicMessages => anthropic_stream_event_has_tool_content(value),
        ProviderProtocol::GeminiGenerateContent => gemini_response_has_tool_content(value),
    }
}

fn output_text(value: &Value, provider_protocol: ProviderProtocol) -> String {
    match provider_protocol {
        ProviderProtocol::OpenAiResponses => openai_output_text(value),
        ProviderProtocol::AnthropicMessages => anthropic_output_text(value),
        ProviderProtocol::GeminiGenerateContent => gemini_output_text(value),
    }
    .unwrap_or_default()
}

pub fn openai_output_text(value: &Value) -> Option<String> {
    value
        .get("output")
        .and_then(Value::as_array)
        .map(|outputs| {
            outputs
                .iter()
                .flat_map(|output| {
                    output
                        .get("content")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|content| content.get("text").and_then(Value::as_str))
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .filter(|text| !text.is_empty())
        .or_else(|| {
            value
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("message"))
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
}

fn openai_refusal_text(value: &Value) -> Option<String> {
    value
        .get("output")
        .and_then(Value::as_array)
        .map(|outputs| {
            outputs
                .iter()
                .flat_map(|output| {
                    output
                        .get("content")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|content| content.get("refusal").and_then(Value::as_str))
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .filter(|text| !text.is_empty())
}

fn openai_response_has_tool_content(value: &Value) -> bool {
    value
        .get("output")
        .and_then(Value::as_array)
        .is_some_and(|outputs| {
            outputs.iter().any(|output| {
                output.get("type").and_then(Value::as_str) == Some("function_call")
                    || output
                        .get("content")
                        .and_then(Value::as_array)
                        .is_some_and(|items| {
                            items.iter().any(|item| {
                                item.get("type")
                                    .and_then(Value::as_str)
                                    .is_some_and(|kind| {
                                        matches!(kind, "tool_call" | "function_call" | "tool_use")
                                    })
                            })
                        })
            })
        })
        || value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message").or_else(|| choice.get("delta")))
            .is_some_and(openai_message_has_tool_content)
}

fn openai_message_has_tool_content(message: &Value) -> bool {
    message
        .get("tool_calls")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty())
        || message.get("function_call").is_some()
}

fn openai_stream_event_has_tool_content(value: &Value) -> bool {
    value
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| {
            matches!(
                kind,
                "response.output_item.added" | "response.function_call_arguments.delta"
            )
        })
        && value
            .get("item")
            .and_then(|item| item.get("type"))
            .and_then(Value::as_str)
            .map(|kind| kind == "function_call")
            .unwrap_or(true)
        || value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("delta"))
            .is_some_and(openai_message_has_tool_content)
}

fn anthropic_output_text(value: &Value) -> Option<String> {
    value
        .get("content")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .filter(|text| !text.is_empty())
        .or_else(|| openai_output_text(value))
}

fn anthropic_response_has_tool_content(value: &Value) -> bool {
    value
        .get("content")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items
                .iter()
                .any(|item| item.get("type").and_then(Value::as_str) == Some("tool_use"))
        })
        || openai_response_has_tool_content(value)
}

fn anthropic_stream_event_has_tool_content(value: &Value) -> bool {
    match value.get("type").and_then(Value::as_str) {
        Some("content_block_start") => {
            value
                .get("content_block")
                .and_then(|block| block.get("type"))
                .and_then(Value::as_str)
                == Some("tool_use")
        }
        Some("content_block_delta") => value
            .get("delta")
            .and_then(|delta| delta.get("partial_json"))
            .and_then(Value::as_str)
            .is_some_and(|partial| !partial.trim().is_empty()),
        _ => false,
    }
}

fn gemini_output_text(value: &Value) -> Option<String> {
    value
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|candidates| candidates.first())
        .and_then(|candidate| candidate.get("content"))
        .and_then(|content| gemini_parts_to_text(content.get("parts")))
        .filter(|text| !text.is_empty())
}

fn gemini_response_has_tool_content(value: &Value) -> bool {
    value
        .get("candidates")
        .and_then(Value::as_array)
        .is_some_and(|candidates| {
            candidates.iter().any(|candidate| {
                candidate
                    .get("content")
                    .and_then(|content| content.get("parts"))
                    .and_then(Value::as_array)
                    .is_some_and(|parts| {
                        parts.iter().any(|part| {
                            part.get("functionCall").is_some()
                                || part.get("function_call").is_some()
                                || part.get("functionResponse").is_some()
                                || part.get("function_response").is_some()
                        })
                    })
            })
        })
}

fn stream_text_delta(value: &Value, provider_protocol: ProviderProtocol) -> Option<String> {
    match provider_protocol {
        ProviderProtocol::OpenAiResponses => {
            if value.get("type").and_then(Value::as_str) == Some("response.output_text.delta") {
                value
                    .get("delta")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            } else {
                value
                    .get("choices")
                    .and_then(Value::as_array)
                    .and_then(|choices| choices.first())
                    .and_then(|choice| choice.get("delta"))
                    .and_then(|delta| delta.get("content"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            }
        }
        ProviderProtocol::AnthropicMessages => {
            if value.get("type").and_then(Value::as_str) == Some("content_block_delta") {
                value
                    .get("delta")
                    .and_then(|delta| delta.get("text"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            } else {
                None
            }
        }
        ProviderProtocol::GeminiGenerateContent => gemini_output_text(value),
    }
}

fn finish_reason(value: &Value, provider_protocol: ProviderProtocol) -> &'static str {
    match provider_protocol {
        ProviderProtocol::AnthropicMessages => {
            match value.get("stop_reason").and_then(Value::as_str) {
                Some("max_tokens") => "length",
                Some("tool_use") => "tool_calls",
                _ => "stop",
            }
        }
        ProviderProtocol::GeminiGenerateContent => match value
            .get("candidates")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("finishReason"))
            .and_then(Value::as_str)
        {
            Some("MAX_TOKENS") => "length",
            _ => "stop",
        },
        ProviderProtocol::OpenAiResponses => "stop",
    }
}

fn anthropic_stop_reason(value: &Value, provider_protocol: ProviderProtocol) -> String {
    match provider_protocol {
        ProviderProtocol::OpenAiResponses => "end_turn".to_string(),
        ProviderProtocol::AnthropicMessages => value
            .get("stop_reason")
            .and_then(Value::as_str)
            .unwrap_or("end_turn")
            .to_string(),
        ProviderProtocol::GeminiGenerateContent => "end_turn".to_string(),
    }
}

fn parse_responses_input(input: Option<&Value>) -> AppResult<Vec<TextMessage>> {
    match input {
        Some(Value::String(text)) => Ok(vec![TextMessage {
            role: "user".to_string(),
            content: vec![MessagePart::Text(text.clone())],
            tool_calls: None,
            tool_call_id: None,
        }]),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                if item.get("type").and_then(Value::as_str) == Some("function_call_output") {
                    return Ok(TextMessage {
                        role: "tool".to_string(),
                        content: openai_content_to_parts(item.get("output"))?,
                        tool_calls: None,
                        tool_call_id: item
                            .get("call_id")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                    });
                }
                if item.get("type").and_then(Value::as_str) == Some("function_call") {
                    return Ok(TextMessage {
                        role: "assistant".to_string(),
                        content: Vec::new(),
                        tool_calls: Some(item.clone()),
                        tool_call_id: item
                            .get("call_id")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                    });
                }
                let role = item
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("user")
                    .to_string();
                Ok(TextMessage {
                    role,
                    content: openai_content_to_parts(item.get("content"))?,
                    tool_calls: item.get("tool_calls").cloned(),
                    tool_call_id: item
                        .get("tool_call_id")
                        .and_then(Value::as_str)
                        .map(ToString::to_string),
                })
            })
            .collect(),
        _ => Err(AppError::BadRequest(
            "responses request requires input string or array".to_string(),
        )),
    }
}

fn openai_content_to_parts(content: Option<&Value>) -> AppResult<Vec<MessagePart>> {
    match content {
        Some(Value::String(text)) => Ok(vec![MessagePart::Text(text.clone())]),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(openai_content_item_to_part)
            .collect(),
        Some(Value::Null) | None => Ok(Vec::new()),
        Some(other) => Err(AppError::BadRequest(format!(
            "unsupported OpenAI content shape: {other}"
        ))),
    }
}

fn openai_content_item_to_part(item: &Value) -> Option<AppResult<MessagePart>> {
    let item_type = item.get("type").and_then(Value::as_str)?;
    match item_type {
        "text" | "input_text" | "output_text" => item
            .get("text")
            .and_then(Value::as_str)
            .map(|text| Ok(MessagePart::Text(text.to_string()))),
        "image_url" | "input_image" => Some(parse_openai_image_input(item).map(MessagePart::Image)),
        "tool_result" => item
            .get("content")
            .and_then(Value::as_str)
            .map(|text| Ok(MessagePart::Text(text.to_string()))),
        "tool_use" => None,
        "input_audio" | "input_file" | "audio" | "video" | "document" => {
            Some(Err(AppError::BadRequest(format!(
                "unsupported non-text OpenAI content item type: {item_type}"
            ))))
        }
        other => Some(Err(AppError::BadRequest(format!(
            "unsupported non-text OpenAI content item type: {other}"
        )))),
    }
}

fn parse_openai_image_input(item: &Value) -> AppResult<ImageInput> {
    if let Some(file_id) = item.get("file_id").and_then(Value::as_str) {
        return Ok(ImageInput {
            source: ImageSource::FileId(file_id.to_string()),
            detail: item
                .get("detail")
                .and_then(Value::as_str)
                .map(ToString::to_string),
        });
    }
    let detail = item
        .get("detail")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let image_value = item.get("image_url").ok_or_else(|| {
        AppError::BadRequest("OpenAI image content item missing image_url".to_string())
    })?;
    parse_image_input_value(image_value, detail)
}

fn parse_image_input_value(value: &Value, detail: Option<String>) -> AppResult<ImageInput> {
    match value {
        Value::String(url) => Ok(ImageInput {
            source: image_source_from_string(url),
            detail,
        }),
        Value::Object(map) => {
            let detail = detail.or_else(|| {
                map.get("detail")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            });
            if let Some(url) = map.get("url").and_then(Value::as_str) {
                return Ok(ImageInput {
                    source: image_source_from_string(url),
                    detail,
                });
            }
            if let Some(file_id) = map.get("file_id").and_then(Value::as_str) {
                return Ok(ImageInput {
                    source: ImageSource::FileId(file_id.to_string()),
                    detail,
                });
            }
            if let Some(file_uri) = map
                .get("file_uri")
                .or_else(|| map.get("fileUri"))
                .and_then(Value::as_str)
            {
                return Ok(ImageInput {
                    source: ImageSource::FileUri {
                        file_uri: file_uri.to_string(),
                        media_type: map
                            .get("media_type")
                            .or_else(|| map.get("mime_type"))
                            .or_else(|| map.get("mimeType"))
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                    },
                    detail,
                });
            }
            if let Some(data) = map.get("data").and_then(Value::as_str) {
                let media_type = map
                    .get("media_type")
                    .or_else(|| map.get("mime_type"))
                    .or_else(|| map.get("mimeType"))
                    .and_then(Value::as_str)
                    .unwrap_or("image/png");
                return Ok(ImageInput {
                    source: ImageSource::Base64 {
                        media_type: media_type.to_string(),
                        data: data.to_string(),
                    },
                    detail,
                });
            }
            Err(AppError::BadRequest(
                "image content item missing image_url url".to_string(),
            ))
        }
        other => Err(AppError::BadRequest(format!(
            "unsupported OpenAI image source: {other}"
        ))),
    }
}

fn image_source_from_string(url: &str) -> ImageSource {
    if let Some((media_type, data)) = parse_data_uri(url) {
        ImageSource::Base64 { media_type, data }
    } else {
        ImageSource::Url(url.to_string())
    }
}

fn parse_data_uri(value: &str) -> Option<(String, String)> {
    let remainder = value.strip_prefix("data:")?;
    let (media_type, data) = remainder.split_once(";base64,")?;
    Some((
        if media_type.is_empty() {
            "image/png".to_string()
        } else {
            media_type.to_string()
        },
        data.to_string(),
    ))
}

fn anthropic_content_to_parts(content: Option<&Value>) -> AppResult<Vec<MessagePart>> {
    match content {
        Some(Value::String(text)) => Ok(vec![MessagePart::Text(text.clone())]),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(anthropic_content_item_to_part)
            .collect(),
        Some(Value::Null) | None => Ok(Vec::new()),
        Some(other) => Err(AppError::BadRequest(format!(
            "unsupported Anthropic content shape: {other}"
        ))),
    }
}

fn anthropic_content_item_to_part(item: &Value) -> Option<AppResult<MessagePart>> {
    let item_type = item.get("type").and_then(Value::as_str)?;
    match item_type {
        "text" => item
            .get("text")
            .and_then(Value::as_str)
            .map(|text| Ok(MessagePart::Text(text.to_string()))),
        "image" => Some(parse_anthropic_image_input(item).map(MessagePart::Image)),
        "tool_result" => item
            .get("content")
            .and_then(Value::as_str)
            .map(|text| Ok(MessagePart::Text(text.to_string()))),
        "tool_use" => None,
        "input_audio" | "input_file" | "audio" | "video" | "document" => {
            Some(Err(AppError::BadRequest(format!(
                "unsupported non-text Anthropic content item type: {item_type}"
            ))))
        }
        other => Some(Err(AppError::BadRequest(format!(
            "unsupported non-text Anthropic content item type: {other}"
        )))),
    }
}

fn parse_anthropic_image_input(item: &Value) -> AppResult<ImageInput> {
    let detail = item
        .get("detail")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let source = item.get("source").ok_or_else(|| {
        AppError::BadRequest("Anthropic image content item missing source".to_string())
    })?;
    parse_image_input_value(source, detail)
}

fn gemini_parts_to_message_parts(parts: Option<&Value>) -> AppResult<Vec<MessagePart>> {
    let Some(parts) = parts.and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut content = Vec::new();
    for part in parts {
        if let Some(text) = part.get("text").and_then(Value::as_str) {
            content.push(MessagePart::Text(text.to_string()));
            continue;
        }
        if let Some(inline_data) = part.get("inlineData").or_else(|| part.get("inline_data")) {
            content.push(MessagePart::Image(parse_gemini_inline_data(inline_data)?));
            continue;
        }
        if let Some(file_data) = part.get("fileData").or_else(|| part.get("file_data")) {
            content.push(MessagePart::Image(parse_gemini_file_data(file_data)?));
            continue;
        }
        if part.get("functionCall").is_some()
            || part.get("function_call").is_some()
            || part.get("functionResponse").is_some()
            || part.get("function_response").is_some()
            || part.get("executableCode").is_some()
            || part.get("codeExecutionResult").is_some()
            || part.get("thoughtSignature").is_some()
            || part.get("thought").is_some()
        {
            continue;
        }
        if let Some(other) = part.get("type").and_then(Value::as_str) {
            return Err(AppError::BadRequest(format!(
                "unsupported non-text Gemini content item type: {other}"
            )));
        }
    }
    Ok(content)
}

fn parse_gemini_inline_data(value: &Value) -> AppResult<ImageInput> {
    let media_type = value
        .get("mimeType")
        .or_else(|| value.get("mime_type"))
        .and_then(Value::as_str)
        .unwrap_or("image/png")
        .to_string();
    let data = value
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::BadRequest("Gemini inlineData missing data".to_string()))?;
    Ok(ImageInput {
        source: ImageSource::Base64 {
            media_type,
            data: data.to_string(),
        },
        detail: None,
    })
}

fn parse_gemini_file_data(value: &Value) -> AppResult<ImageInput> {
    let file_uri = value
        .get("fileUri")
        .or_else(|| value.get("file_uri"))
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::BadRequest("Gemini fileData missing fileUri".to_string()))?;
    Ok(ImageInput {
        source: ImageSource::FileUri {
            file_uri: file_uri.to_string(),
            media_type: value
                .get("mimeType")
                .or_else(|| value.get("mime_type"))
                .and_then(Value::as_str)
                .map(ToString::to_string),
        },
        detail: None,
    })
}

fn parts_to_text_only(parts: &[MessagePart], field: &str) -> AppResult<String> {
    let mut text = String::new();
    for part in parts {
        match part {
            MessagePart::Text(chunk) => text.push_str(chunk),
            MessagePart::Image(_) => {
                return Err(AppError::BadRequest(format!(
                    "{field} does not support image content"
                )));
            }
        }
    }
    Ok(text)
}

fn message_text_content(parts: &[MessagePart]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Text(text) => Some(text.as_str()),
            MessagePart::Image(_) => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

fn openai_responses_content_items(parts: &[MessagePart], text_type: &str) -> AppResult<Vec<Value>> {
    parts
        .iter()
        .map(|part| match part {
            MessagePart::Text(text) => Ok(json!({
                "type": text_type,
                "text": text,
            })),
            MessagePart::Image(image) => image_input_to_openai_responses_item(image),
        })
        .collect()
}

fn anthropic_content_items(parts: &[MessagePart]) -> AppResult<Vec<Value>> {
    parts
        .iter()
        .map(|part| match part {
            MessagePart::Text(text) => Ok(json!({
                "type": "text",
                "text": text,
            })),
            MessagePart::Image(image) => Ok(json!({
                "type": "image",
                "source": image_input_to_anthropic_source(image)?,
            })),
        })
        .collect()
}

fn gemini_content_parts(parts: &[MessagePart]) -> AppResult<Vec<Value>> {
    parts
        .iter()
        .map(|part| match part {
            MessagePart::Text(text) => Ok(json!({
                "text": text,
            })),
            MessagePart::Image(image) => image_input_to_gemini_part(image),
        })
        .collect()
}

fn image_input_to_openai_responses_item(image: &ImageInput) -> AppResult<Value> {
    let mut item = Map::new();
    item.insert("type".to_string(), Value::String("input_image".to_string()));
    if let Some(detail) = &image.detail {
        item.insert("detail".to_string(), Value::String(detail.clone()));
    }
    match &image.source {
        ImageSource::Url(url) => {
            item.insert("image_url".to_string(), Value::String(url.clone()));
        }
        ImageSource::Base64 { media_type, data } => {
            item.insert(
                "image_url".to_string(),
                Value::String(format!("data:{media_type};base64,{data}")),
            );
        }
        ImageSource::FileId(file_id) => {
            item.insert("file_id".to_string(), Value::String(file_id.clone()));
        }
        ImageSource::FileUri { file_uri, .. } => {
            if file_uri.starts_with("http://") || file_uri.starts_with("https://") {
                item.insert("image_url".to_string(), Value::String(file_uri.clone()));
            } else {
                return Err(AppError::BadRequest(format!(
                    "unsupported image source for OpenAI responses: {file_uri}"
                )));
            }
        }
    }
    Ok(Value::Object(item))
}

fn image_input_to_anthropic_source(image: &ImageInput) -> AppResult<Value> {
    match &image.source {
        ImageSource::Url(url) => Ok(json!({
            "type": "url",
            "url": url,
        })),
        ImageSource::Base64 { media_type, data } => Ok(json!({
            "type": "base64",
            "media_type": media_type,
            "data": data,
        })),
        ImageSource::FileUri { file_uri, .. } => Ok(json!({
            "type": "url",
            "url": file_uri,
        })),
        ImageSource::FileId(file_id) => Err(AppError::BadRequest(format!(
            "unsupported image source for Anthropic image block: {file_id}"
        ))),
    }
}

fn image_input_to_gemini_part(image: &ImageInput) -> AppResult<Value> {
    match &image.source {
        ImageSource::Base64 { media_type, data } => Ok(json!({
            "inlineData": {
                "mimeType": media_type,
                "data": data,
            }
        })),
        ImageSource::Url(url) => Ok(json!({
            "fileData": {
                "fileUri": url,
            }
        })),
        ImageSource::FileUri {
            file_uri,
            media_type,
        } => {
            let mut file_data = Map::new();
            file_data.insert("fileUri".to_string(), Value::String(file_uri.clone()));
            if let Some(media_type) = media_type {
                file_data.insert("mimeType".to_string(), Value::String(media_type.clone()));
            }
            let mut part = Map::new();
            part.insert("fileData".to_string(), Value::Object(file_data));
            Ok(Value::Object(part))
        }
        ImageSource::FileId(file_id) => Err(AppError::BadRequest(format!(
            "unsupported image source for Gemini fileData: {file_id}"
        ))),
    }
}

fn gemini_parts_to_text(parts: Option<&Value>) -> Option<String> {
    let parts = parts?.as_array()?;
    let text = parts
        .iter()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("");
    Some(text)
}

fn openai_tools_to_anthropic(tools: &Value) -> Value {
    let Some(items) = tools.as_array() else {
        return tools.clone();
    };
    Value::Array(
        items
            .iter()
            .filter_map(|tool| {
                if tool.get("type").and_then(Value::as_str) != Some("function") {
                    return None;
                }
                let function = tool.get("function")?;
                Some(json!({
                    "name": function.get("name").cloned().unwrap_or(Value::Null),
                    "description": function.get("description").cloned().unwrap_or(Value::Null),
                    "input_schema": function.get("parameters").cloned().unwrap_or_else(|| json!({"type": "object"})),
                }))
            })
            .collect(),
    )
}

fn openai_tools_to_gemini(tools: &Value) -> Value {
    let Some(items) = tools.as_array() else {
        return tools.clone();
    };
    let function_declarations = items
        .iter()
        .filter_map(|tool| {
            if tool.get("type").and_then(Value::as_str) != Some("function") {
                return None;
            }
            let function = tool.get("function")?;
            Some(json!({
                "name": function.get("name").cloned().unwrap_or(Value::Null),
                "description": function.get("description").cloned().unwrap_or(Value::Null),
                "parameters": function.get("parameters").cloned().unwrap_or_else(|| json!({"type": "object"})),
            }))
        })
        .collect::<Vec<_>>();
    json!([{ "functionDeclarations": function_declarations }])
}

fn required_string(value: &Value, field: &str, message: &str) -> AppResult<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| AppError::BadRequest(message.to_string()))
}

fn non_empty_join(parts: Vec<String>) -> Option<String> {
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn converts_anthropic_to_openai_responses() {
        let request = parse_client_request(
            ClientProtocol::AnthropicMessages,
            &json!({
                "model": "claude-3-5-sonnet",
                "system": "be terse",
                "messages": [{"role": "user", "content": [{"type": "text", "text": "hello"}]}],
                "max_tokens": 64
            }),
        )
        .unwrap();
        let outbound = text_to_openai_responses(&request).unwrap();
        assert_eq!(outbound["instructions"], "be terse");
        assert_eq!(outbound["max_output_tokens"], 64);
        assert_eq!(outbound["input"][0]["content"][0]["text"], "hello");
    }

    #[test]
    fn converts_openai_response_to_anthropic_message() {
        let (body, usage) = client_response_body(
            ClientProtocol::AnthropicMessages,
            ProviderProtocol::OpenAiResponses,
            json!({
                "id": "resp_1",
                "model": "gpt-test",
                "output": [{"type": "message", "content": [{"type": "output_text", "text": "hi"}]}],
                "usage": {"input_tokens": 10, "output_tokens": 2}
            }),
        );
        assert_eq!(body["content"][0]["text"], "hi");
        assert_eq!(usage.total(), 12);
    }

    #[test]
    fn extracts_openai_cached_input_details_without_double_counting() {
        let usage = extract_usage(&json!({
            "usage": {
                "input_tokens": 120,
                "input_tokens_details": {"cached_tokens": 40},
                "output_tokens": 7
            }
        }));

        assert_eq!(usage.input_tokens, 80);
        assert_eq!(usage.cache_tokens, 40);
        assert_eq!(usage.output_tokens, 7);
        assert_eq!(usage.total(), 127);
    }

    #[test]
    fn extracts_chat_prompt_cache_details_without_double_counting() {
        let usage = extract_usage(&json!({
            "usage": {
                "prompt_tokens": 90,
                "prompt_tokens_details": {"cached_tokens": 35},
                "completion_tokens": 11
            }
        }));

        assert_eq!(usage.input_tokens, 55);
        assert_eq!(usage.cache_tokens, 35);
        assert_eq!(usage.output_tokens, 11);
        assert_eq!(usage.total(), 101);
    }

    #[test]
    fn extracts_usage_nested_in_response_completed_event() {
        let usage = extract_usage(&json!({
            "type": "response.completed",
            "response": {
                "usage": {
                    "input_tokens": 200,
                    "input_tokens_details": {"cached_tokens": 125},
                    "output_tokens": 25
                }
            }
        }));

        assert_eq!(usage.input_tokens, 75);
        assert_eq!(usage.cache_tokens, 125);
        assert_eq!(usage.output_tokens, 25);
        assert_eq!(usage.total(), 225);
    }

    #[test]
    fn converts_openai_text_request_to_gemini() {
        let request = parse_client_request(
            ClientProtocol::OpenAiChatCompletions,
            &json!({
                "model": "gemini-test",
                "messages": [{"role": "user", "content": "hello"}],
                "max_tokens": 64
            }),
        )
        .unwrap();
        let outbound = text_to_gemini_generate_content(&request).unwrap();
        assert_eq!(outbound["contents"][0]["parts"][0]["text"], "hello");
        assert_eq!(outbound["generationConfig"]["maxOutputTokens"], 64);
    }

    #[test]
    fn parses_openai_chat_image_input() {
        let request = parse_client_request(
            ClientProtocol::OpenAiChatCompletions,
            &json!({
                "model": "gpt-4o-mini",
                "messages": [{
                    "role": "user",
                    "content": [
                        {"type": "text", "text": "look"},
                        {"type": "image_url", "image_url": {"url": "data:image/png;base64,aGVsbG8=", "detail": "low"}}
                    ]
                }]
            }),
        )
        .unwrap();
        assert_eq!(request.messages[0].content.len(), 2);
        match &request.messages[0].content[1] {
            MessagePart::Image(image) => match &image.source {
                ImageSource::Base64 { media_type, data } => {
                    assert_eq!(media_type, "image/png");
                    assert_eq!(data, "aGVsbG8=");
                }
                _ => panic!("expected base64 image"),
            },
            _ => panic!("expected image part"),
        }
    }

    #[test]
    fn parses_openai_responses_image_input() {
        let request = parse_client_request(
            ClientProtocol::OpenAiResponses,
            &json!({
                "model": "gpt-4o-mini",
                "input": [{
                    "role": "user",
                    "content": [
                        {"type": "input_text", "text": "look"},
                        {"type": "input_image", "image_url": "https://example.com/a.png"}
                    ]
                }]
            }),
        )
        .unwrap();
        assert_eq!(request.messages[0].content.len(), 2);
        match &request.messages[0].content[1] {
            MessagePart::Image(image) => match &image.source {
                ImageSource::Url(url) => assert_eq!(url, "https://example.com/a.png"),
                _ => panic!("expected url image"),
            },
            _ => panic!("expected image part"),
        }
    }

    #[test]
    fn parses_anthropic_image_blocks() {
        let request = parse_client_request(
            ClientProtocol::AnthropicMessages,
            &json!({
                "model": "claude-3-5-sonnet",
                "messages": [{
                    "role": "user",
                    "content": [
                        {"type": "text", "text": "look"},
                        {"type": "image", "source": {"type": "base64", "media_type": "image/jpeg", "data": "aGVsbG8="}}
                    ]
                }]
            }),
        )
        .unwrap();
        assert_eq!(request.messages[0].content.len(), 2);
        match &request.messages[0].content[1] {
            MessagePart::Image(image) => match &image.source {
                ImageSource::Base64 { media_type, data } => {
                    assert_eq!(media_type, "image/jpeg");
                    assert_eq!(data, "aGVsbG8=");
                }
                _ => panic!("expected base64 image"),
            },
            _ => panic!("expected image part"),
        }
    }

    #[test]
    fn parses_gemini_inline_and_file_data() {
        let request = parse_client_request(
            ClientProtocol::GeminiGenerateContent,
            &json!({
                "model": "gemini-test",
                "contents": [{
                    "role": "user",
                    "parts": [
                        {"text": "look"},
                        {"inlineData": {"mimeType": "image/png", "data": "aGVsbG8="}},
                        {"fileData": {"fileUri": "gs://bucket/image.png", "mimeType": "image/png"}}
                    ]
                }]
            }),
        )
        .unwrap();
        assert_eq!(request.messages[0].content.len(), 3);
        assert!(matches!(
            request.messages[0].content[1],
            MessagePart::Image(_)
        ));
        assert!(matches!(
            request.messages[0].content[2],
            MessagePart::Image(_)
        ));
    }

    #[test]
    fn same_protocol_passthrough_keeps_image_payload() {
        let raw = json!({
            "model": "gpt-test",
            "input": [{
                "role": "user",
                "content": [
                    {"type": "input_text", "text": "look"},
                    {"type": "input_image", "image_url": "https://example.com/a.png"}
                ]
            }],
            "_stream": false
        });
        let request = parse_client_request(ClientProtocol::OpenAiResponses, &raw).unwrap();
        let body = upstream_body(
            ClientProtocol::OpenAiResponses,
            ProviderProtocol::OpenAiResponses,
            &raw,
            &request,
        )
        .unwrap();
        assert_eq!(
            body["input"][0]["content"][1]["image_url"],
            "https://example.com/a.png"
        );
        assert!(body.get("_stream").is_none());
    }

    #[test]
    fn extracts_gemini_usage() {
        let usage = extract_usage(&json!({
            "usageMetadata": {
                "promptTokenCount": 7,
                "candidatesTokenCount": 3,
                "cachedContentTokenCount": 2
            }
        }));
        assert_eq!(usage.input_tokens, 5);
        assert_eq!(usage.output_tokens, 3);
        assert_eq!(usage.cache_tokens, 2);
    }
}
