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
    pub content: String,
    pub tool_calls: Option<Value>,
    pub tool_call_id: Option<String>,
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
) -> Value {
    if same_wire_protocol(client_protocol, provider_protocol) {
        return strip_internal_fields(raw_body);
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

fn parse_openai_chat_completions(value: &Value) -> AppResult<TextRequest> {
    reject_non_text_payload(value)?;
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
        let content = openai_content_to_text(message.get("content"))?;
        if role == "system" {
            system_parts.push(content);
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
    reject_non_text_payload(value)?;
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
    reject_non_text_payload(value)?;
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
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .map(ToString::to_string)
                .collect(),
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
                content: anthropic_content_to_text(message.get("content"))?,
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
    reject_non_text_payload(value)?;
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
        .and_then(|item| gemini_parts_to_text(item.get("parts")))
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
                content: gemini_parts_to_text(content.get("parts")).unwrap_or_default(),
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

fn text_to_openai_responses(request: &TextRequest) -> Value {
    let mut input = Vec::new();
    for message in &request.messages {
        if message.role == "tool" {
            input.push(json!({
                "type": "function_call_output",
                "call_id": message.tool_call_id.clone().unwrap_or_default(),
                "output": message.content,
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
            input.push(json!({
                "role": message.role,
                "content": [{"type": "input_text", "text": message.content}],
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
    body
}

fn text_to_anthropic_messages(request: &TextRequest) -> Value {
    let messages = request
        .messages
        .iter()
        .map(|message| {
            json!({
                "role": if message.role == "assistant" { "assistant" } else { "user" },
                "content": [{"type": "text", "text": message.content}],
            })
        })
        .collect::<Vec<_>>();
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
    body
}

fn text_to_gemini_generate_content(request: &TextRequest) -> Value {
    let contents = request
        .messages
        .iter()
        .map(|message| {
            json!({
                "role": if message.role == "assistant" { "model" } else { "user" },
                "parts": [{"text": message.content}],
            })
        })
        .collect::<Vec<_>>();
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
    body
}

fn response_to_chat_completions(
    value: Value,
    provider_protocol: ProviderProtocol,
    usage: Usage,
) -> (Value, Usage) {
    let text = output_text(&value, provider_protocol);
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
                "prompt_tokens": usage.input_tokens,
                "completion_tokens": usage.output_tokens,
                "total_tokens": usage.total()
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
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
                "total_tokens": usage.total()
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
                "promptTokenCount": usage.input_tokens,
                "candidatesTokenCount": usage.output_tokens,
                "totalTokenCount": usage.total()
            }
        }),
        usage,
    )
}

pub fn extract_usage(value: &Value) -> Usage {
    let usage = value.get("usage").unwrap_or(value);
    let input_tokens = usage
        .get("input_tokens")
        .or_else(|| usage.get("prompt_tokens"))
        .or_else(|| usage.get("promptTokenCount"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let output_tokens = usage
        .get("output_tokens")
        .or_else(|| usage.get("completion_tokens"))
        .or_else(|| usage.get("candidatesTokenCount"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let cache_tokens = usage
        .get("cache_read_input_tokens")
        .or_else(|| usage.get("cache_creation_input_tokens"))
        .or_else(|| usage.get("cached_tokens"))
        .or_else(|| usage.get("cachedContentTokenCount"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if value.get("usageMetadata").is_some() {
        return extract_usage(value.get("usageMetadata").unwrap_or(value));
    }
    Usage {
        input_tokens,
        output_tokens,
        cache_tokens,
    }
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

fn gemini_output_text(value: &Value) -> Option<String> {
    value
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|candidates| candidates.first())
        .and_then(|candidate| candidate.get("content"))
        .and_then(|content| gemini_parts_to_text(content.get("parts")))
        .filter(|text| !text.is_empty())
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
            content: text.clone(),
            tool_calls: None,
            tool_call_id: None,
        }]),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                if item.get("type").and_then(Value::as_str) == Some("function_call_output") {
                    return Ok(TextMessage {
                        role: "tool".to_string(),
                        content: item
                            .get("output")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
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
                        content: String::new(),
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
                    content: openai_content_to_text(item.get("content"))?,
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

fn openai_content_to_text(content: Option<&Value>) -> AppResult<String> {
    match content {
        Some(Value::String(text)) => Ok(text.clone()),
        Some(Value::Array(items)) => {
            let mut text = String::new();
            for item in items {
                match item.get("type").and_then(Value::as_str) {
                    Some("input_text") | Some("output_text") | Some("text") => {
                        if let Some(part) = item.get("text").and_then(Value::as_str) {
                            text.push_str(part);
                        }
                    }
                    Some(other) => {
                        return Err(AppError::BadRequest(format!(
                            "unsupported non-text OpenAI content item type: {other}"
                        )));
                    }
                    None => {}
                }
            }
            Ok(text)
        }
        _ => Ok(String::new()),
    }
}

fn anthropic_content_to_text(content: Option<&Value>) -> AppResult<String> {
    match content {
        Some(Value::String(text)) => Ok(text.clone()),
        Some(Value::Array(items)) => {
            let mut text = String::new();
            for item in items {
                match item.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(part) = item.get("text").and_then(Value::as_str) {
                            text.push_str(part);
                        }
                    }
                    Some("tool_result") => {
                        if let Some(part) = item.get("content").and_then(Value::as_str) {
                            text.push_str(part);
                        }
                    }
                    Some("tool_use") => {}
                    Some(other) => {
                        return Err(AppError::BadRequest(format!(
                            "unsupported non-text Anthropic content item type: {other}"
                        )));
                    }
                    None => {}
                }
            }
            Ok(text)
        }
        _ => Ok(String::new()),
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

fn reject_non_text_payload(value: &Value) -> AppResult<()> {
    fn visit(value: &Value) -> Option<String> {
        match value {
            Value::Object(map) => {
                for (key, value) in map {
                    if matches!(
                        key.as_str(),
                        "image_url"
                            | "input_audio"
                            | "input_image"
                            | "input_file"
                            | "inline_data"
                            | "file_data"
                            | "audio"
                            | "video"
                            | "document"
                    ) {
                        return Some(key.clone());
                    }
                    if let Some(reason) = visit(value) {
                        return Some(reason);
                    }
                }
                None
            }
            Value::Array(items) => items.iter().find_map(visit),
            _ => None,
        }
    }
    if let Some(field) = visit(value) {
        return Err(AppError::BadRequest(format!(
            "unsupported non-text payload field: {field}"
        )));
    }
    Ok(())
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
        let outbound = text_to_openai_responses(&request);
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
        let outbound = text_to_gemini_generate_content(&request);
        assert_eq!(outbound["contents"][0]["parts"][0]["text"], "hello");
        assert_eq!(outbound["generationConfig"]["maxOutputTokens"], 64);
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
        assert_eq!(usage.input_tokens, 7);
        assert_eq!(usage.output_tokens, 3);
        assert_eq!(usage.cache_tokens, 2);
    }
}
