use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    error::{AppError, AppResult},
    models::Usage,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientProtocol {
    OpenAiResponses,
    AnthropicMessages,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralOpenAIRequest {
    pub model: String,
    pub messages: Vec<GeneralMessage>,
    pub system: Option<String>,
    pub max_tokens: Option<i64>,
    pub temperature: Option<f64>,
    pub stream: bool,
    pub tools: Option<Value>,
    pub tool_choice: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralMessage {
    pub role: String,
    pub content: String,
    pub tool_calls: Option<Value>,
    pub tool_call_id: Option<String>,
}

impl GeneralOpenAIRequest {
    pub fn estimated_input_tokens(&self) -> i64 {
        estimate_tokens(
            &serde_json::to_string(self).unwrap_or_else(|_| self.model.clone()),
        )
    }
}

pub fn parse_openai_responses(value: Value) -> AppResult<GeneralOpenAIRequest> {
    reject_unsupported(&value, &["input_image", "input_file", "reasoning"])?;
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::BadRequest("responses request requires model".to_string()))?
        .to_string();
    let stream = value.get("stream").and_then(Value::as_bool).unwrap_or(false);
    let max_tokens = value
        .get("max_output_tokens")
        .or_else(|| value.get("max_tokens"))
        .and_then(Value::as_i64);
    let temperature = value.get("temperature").and_then(Value::as_f64);
    let system = value
        .get("instructions")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let messages = parse_responses_input(value.get("input"))?;
    Ok(GeneralOpenAIRequest {
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

pub fn parse_anthropic_messages(value: Value) -> AppResult<GeneralOpenAIRequest> {
    reject_unsupported(&value, &["thinking", "images", "image", "documents"])?;
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::BadRequest("messages request requires model".to_string()))?
        .to_string();
    let stream = value.get("stream").and_then(Value::as_bool).unwrap_or(false);
    let max_tokens = value.get("max_tokens").and_then(Value::as_i64);
    let temperature = value.get("temperature").and_then(Value::as_f64);
    let system = match value.get("system") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(items)) => Some(
            items
                .iter()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
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
            let content = anthropic_content_to_text(message.get("content"))?;
            Ok(GeneralMessage {
                role,
                content,
                tool_calls: message.get("tool_calls").cloned(),
                tool_call_id: None,
            })
        })
        .collect::<AppResult<Vec<_>>>()?;
    Ok(GeneralOpenAIRequest {
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

pub fn general_to_openai_responses(request: &GeneralOpenAIRequest, stream: bool) -> Value {
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
        "stream": stream,
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

pub fn general_to_anthropic_messages(request: &GeneralOpenAIRequest, stream: bool) -> Value {
    let messages = request
        .messages
        .iter()
        .filter(|message| message.role != "system")
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
        "stream": stream,
    });
    if let Some(system) = &request.system {
        body["system"] = Value::String(system.clone());
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

pub fn response_to_openai(value: Value) -> (Value, Usage) {
    let usage = extract_usage(&value);
    if value.get("output").is_some() {
        return (value, usage);
    }
    let content = value
        .get("content")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
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
        .unwrap_or_default();
    (
        json!({
            "id": value.get("id").cloned().unwrap_or_else(|| json!("resp_local")),
            "object": "response",
            "model": value.get("model").cloned().unwrap_or(Value::Null),
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": content}]
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

pub fn response_to_anthropic(value: Value) -> (Value, Usage) {
    let usage = extract_usage(&value);
    if value.get("content").is_some() && value.get("type").and_then(Value::as_str) == Some("message") {
        return (value, usage);
    }
    let text = openai_output_text(&value);
    (
        json!({
            "id": value.get("id").cloned().unwrap_or_else(|| json!("msg_local")),
            "type": "message",
            "role": "assistant",
            "model": value.get("model").cloned().unwrap_or(Value::Null),
            "content": [{"type": "text", "text": text}],
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
                "cache_read_input_tokens": usage.cache_tokens,
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
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let output_tokens = usage
        .get("output_tokens")
        .or_else(|| usage.get("completion_tokens"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let cache_tokens = usage
        .get("cache_read_input_tokens")
        .or_else(|| usage.get("cache_creation_input_tokens"))
        .or_else(|| usage.get("cached_tokens"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    Usage {
        input_tokens,
        output_tokens,
        cache_tokens,
    }
}

pub fn estimate_tokens(text: &str) -> i64 {
    // A conservative local estimate that keeps the gateway dependency-light for Anthropic
    // while still preventing obvious negative-balance abuse before upstream usage arrives.
    ((text.chars().count() as f64) / 3.2).ceil().max(1.0) as i64
}

pub fn openai_output_text(value: &Value) -> String {
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
        .unwrap_or_default()
}

fn parse_responses_input(input: Option<&Value>) -> AppResult<Vec<GeneralMessage>> {
    match input {
        Some(Value::String(text)) => Ok(vec![GeneralMessage {
            role: "user".to_string(),
            content: text.clone(),
            tool_calls: None,
            tool_call_id: None,
        }]),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                if item.get("type").and_then(Value::as_str) == Some("function_call_output") {
                    return Ok(GeneralMessage {
                        role: "tool".to_string(),
                        content: item.get("output").and_then(Value::as_str).unwrap_or("").to_string(),
                        tool_calls: None,
                        tool_call_id: item.get("call_id").and_then(Value::as_str).map(ToString::to_string),
                    });
                }
                if item.get("type").and_then(Value::as_str) == Some("function_call") {
                    return Ok(GeneralMessage {
                        role: "assistant".to_string(),
                        content: String::new(),
                        tool_calls: Some(item.clone()),
                        tool_call_id: item.get("call_id").and_then(Value::as_str).map(ToString::to_string),
                    });
                }
                let role = item
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("user")
                    .to_string();
                let content = openai_input_content_to_text(item.get("content"))?;
                Ok(GeneralMessage {
                    role,
                    content,
                    tool_calls: item.get("tool_calls").cloned(),
                    tool_call_id: item.get("tool_call_id").and_then(Value::as_str).map(ToString::to_string),
                })
            })
            .collect(),
        _ => Err(AppError::BadRequest(
            "responses request requires input string or array".to_string(),
        )),
    }
}

fn openai_input_content_to_text(content: Option<&Value>) -> AppResult<String> {
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
                            "unsupported OpenAI content item type: {other}"
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
                    Some("tool_use") | Some("tool_result") => {
                        text.push_str(&item.to_string());
                    }
                    Some(other) => {
                        return Err(AppError::BadRequest(format!(
                            "unsupported Anthropic content item type: {other}"
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

fn reject_unsupported(value: &Value, needles: &[&str]) -> AppResult<()> {
    let body = value.to_string();
    for needle in needles {
        if body.contains(needle) {
            return Err(AppError::BadRequest(format!(
                "unsupported MVP field or content type: {needle}"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn converts_anthropic_to_openai_responses() {
        let request = parse_anthropic_messages(json!({
            "model": "claude-3-5-sonnet",
            "system": "be terse",
            "messages": [{"role": "user", "content": [{"type": "text", "text": "hello"}]}],
            "max_tokens": 64
        }))
        .unwrap();
        let outbound = general_to_openai_responses(&request, false);
        assert_eq!(outbound["instructions"], "be terse");
        assert_eq!(outbound["max_output_tokens"], 64);
        assert_eq!(outbound["input"][0]["content"][0]["text"], "hello");
    }

    #[test]
    fn converts_openai_response_to_anthropic_message() {
        let (body, usage) = response_to_anthropic(json!({
            "id": "resp_1",
            "model": "gpt-test",
            "output": [{"type": "message", "content": [{"type": "output_text", "text": "hi"}]}],
            "usage": {"input_tokens": 10, "output_tokens": 2}
        }));
        assert_eq!(body["content"][0]["text"], "hi");
        assert_eq!(usage.total(), 12);
    }
}
