mod stream;
mod wire;

use serde::de::DeserializeOwned;
use serde_json::value::RawValue;

use super::image::{decode_openai_image_url, encode_openai_image_url};
use crate::codec::ProtocolCodec;
use crate::ir::{
    AssistantOutput, Choice, Conversation, Finish, ImageRef, Part, RawJson, Reasoning,
    ReasoningEffort, Role, Sampling, StopReason, ToolChoice, ToolDef, ToolId, ToolResult,
    ToolResultContent, ToolUse, ToolUseKind, Turn, Usage,
};
use crate::report::{Report, Severity, UnmappedReason};
use crate::Error;
use wire::*;

/// OpenAI Chat Completions 协议 codec。
#[derive(Debug, Default, Clone, Copy)]
pub struct Chat;

impl ProtocolCodec for Chat {
    fn decode_request(&self, body: &[u8]) -> Result<Conversation, Error> {
        self.decode_request_with_report(body, &mut Report::new())
    }

    fn decode_request_with_report(
        &self,
        body: &[u8],
        report: &mut Report,
    ) -> Result<Conversation, Error> {
        let request: ChatRequestIn = parse_json(body)?;
        let ChatRequestIn {
            messages,
            tools,
            tool_choice,
            max_completion_tokens,
            max_tokens,
            temperature,
            top_p,
            stop,
            seed,
            n,
            presence_penalty,
            frequency_penalty,
            reasoning_effort,
            extra,
        } = request;

        for key in extra.keys() {
            report.unmapped(
                format!("request.{key}"),
                UnmappedReason::UnsupportedByTarget,
                Severity::Degraded,
            );
        }
        if max_tokens.is_some() && max_completion_tokens.is_none() {
            report.warn(
                "request.max_tokens",
                "max_tokens is a deprecated alias; mapped as max_output_tokens",
                Severity::Degraded,
            );
        }

        let (system, turns) = decode_messages(messages, report)?;
        Ok(Conversation {
            system,
            turns,
            tools: decode_tools(tools)?,
            tool_choice: decode_tool_choice(tool_choice)?,
            sampling: Sampling {
                temperature,
                top_p,
                max_output_tokens: max_completion_tokens.or(max_tokens),
                stop: decode_stop(stop),
                seed,
                n,
                presence_penalty,
                frequency_penalty,
                ..Sampling::default()
            },
            reasoning: decode_reasoning(reasoning_effort)?,
        })
    }

    fn encode_request(&self, conversation: &Conversation) -> Result<Vec<u8>, Error> {
        let mut messages = Vec::new();

        for part in &conversation.system {
            match part {
                Part::Text(text) => messages.push(MessageOut {
                    role: "system",
                    content: Some(ContentOut::Text(text.clone())),
                    tool_calls: None,
                    tool_call_id: None,
                }),
                _ => return Err(unsupported("chat system part")),
            }
        }

        for turn in &conversation.turns {
            match turn.role {
                Role::User => encode_user_turn(turn, &mut messages)?,
                Role::Assistant => encode_assistant_turn(turn, &mut messages)?,
            }
        }

        let tools = if conversation.tools.is_empty() {
            None
        } else {
            Some(
                conversation
                    .tools
                    .iter()
                    .map(encode_tool)
                    .collect::<Result<Vec<_>, _>>()?,
            )
        };

        let output = ChatRequestOut {
            messages,
            tools,
            tool_choice: encode_tool_choice(&conversation.tool_choice)?,
            max_completion_tokens: conversation.sampling.max_output_tokens,
            temperature: conversation.sampling.temperature,
            top_p: conversation.sampling.top_p,
            stop: encode_stop(&conversation.sampling.stop),
            seed: conversation.sampling.seed,
            n: conversation.sampling.n,
            presence_penalty: conversation.sampling.presence_penalty,
            frequency_penalty: conversation.sampling.frequency_penalty,
            reasoning_effort: conversation
                .reasoning
                .effort
                .map(|effort| reasoning_effort_name(effort).to_owned()),
        };

        serialize_json(&output)
    }

    fn decode_response(&self, body: &[u8]) -> Result<AssistantOutput, Error> {
        self.decode_response_with_report(body, &mut Report::new())
    }

    fn decode_response_with_report(
        &self,
        body: &[u8],
        report: &mut Report,
    ) -> Result<AssistantOutput, Error> {
        let response: ChatResponseIn = parse_json(body)?;
        let provider_finish = response
            .extensions
            .as_ref()
            .and_then(|extensions| extensions.provider_finish_reason.clone());

        let mut choices = Vec::new();
        for choice in response.choices {
            let finish_path = format!("choices[{}].finish_reason", choice.index);
            choices.push(Choice {
                index: choice.index,
                parts: decode_message_parts(choice.message)?,
                finish: decode_finish_with_report(
                    choice.finish_reason,
                    provider_finish.clone(),
                    &finish_path,
                    report,
                ),
            });
        }

        Ok(AssistantOutput {
            choices,
            usage: response.usage.map(decode_usage).unwrap_or_default(),
        })
    }

    fn encode_response(&self, output: &AssistantOutput) -> Result<Vec<u8>, Error> {
        let choices = output
            .choices
            .iter()
            .map(encode_response_choice)
            .collect::<Result<Vec<_>, _>>()?;

        let provider_finish_reason = output
            .choices
            .first()
            .map(|choice| choice.finish.provider_raw.to_string())
            .unwrap_or_default();

        let response = ChatResponseOut {
            id: "chatcmpl-llmwire",
            object: "chat.completion",
            created: 0,
            model: "llmwire",
            choices,
            usage: encode_usage(output.usage),
            extensions: ResponseExtensionsOut {
                provider_finish_reason,
            },
        };

        serialize_json(&response)
    }
    fn decode_stream_frame(
        &self,
        frame: &crate::framing::SseFrame,
        state: &crate::ir::StreamState,
    ) -> Result<crate::codec::StreamDecode, Error> {
        stream::decode_stream_frame(frame, state)
    }

    fn encode_stream_event(
        &self,
        event: &crate::ir::Event,
        state: &crate::ir::StreamState,
    ) -> Result<Vec<crate::framing::SseFrame>, Error> {
        stream::encode_stream_event(event, state)
    }
}
fn decode_messages(
    messages: Vec<MessageIn>,
    report: &mut Report,
) -> Result<(Vec<Part>, Vec<Turn>), Error> {
    let mut system = Vec::new();
    let mut turns = Vec::new();
    let mut messages = messages.into_iter().enumerate().peekable();

    while let Some((message_index, message)) = messages.next() {
        match message.role.as_str() {
            "user" => turns.push(Turn {
                role: Role::User,
                parts: decode_message_parts(message)?,
            }),
            "assistant" => turns.push(Turn {
                role: Role::Assistant,
                parts: decode_message_parts(message)?,
            }),
            "tool" => {
                let mut parts = vec![Part::ToolResult(decode_tool_result(message)?)];
                while matches!(
                    messages.peek().map(|(_, item)| item.role.as_str()),
                    Some("tool")
                ) {
                    let (_, message) = messages.next().expect("peeked message");
                    parts.push(Part::ToolResult(decode_tool_result(message)?));
                }
                if matches!(
                    messages.peek().map(|(_, item)| item.role.as_str()),
                    Some("user")
                ) {
                    let (_, message) = messages.next().expect("peeked message");
                    parts.extend(decode_message_parts(message)?);
                }
                turns.push(Turn {
                    role: Role::User,
                    parts,
                });
            }
            "system" | "developer" => {
                let parts = decode_message_parts(message)?;
                if !parts.iter().all(|part| matches!(part, Part::Text(_))) {
                    return Err(unsupported("chat system content"));
                }
                if !turns.is_empty() {
                    report.warn(
                        format!("request.messages[{message_index}].role"),
                        "late system message promoted to top-level system; position lost",
                        Severity::Degraded,
                    );
                }
                system.extend(parts);
            }
            role => return Err(unsupported(format!("chat role {role}"))),
        }
    }

    Ok((system, turns))
}

fn decode_message_parts(message: MessageIn) -> Result<Vec<Part>, Error> {
    let mut parts = decode_content(message.content)?;
    if let Some(refusal) = message.refusal {
        parts.push(Part::Text(refusal));
    }
    if let Some(tool_calls) = message.tool_calls {
        for tool_call in tool_calls {
            parts.push(Part::ToolUse(decode_tool_call(tool_call)?));
        }
    }
    Ok(parts)
}

fn decode_content(content: Option<ContentIn>) -> Result<Vec<Part>, Error> {
    match content {
        None => Ok(Vec::new()),
        Some(ContentIn::Text(text)) => Ok(vec![Part::Text(text)]),
        Some(ContentIn::Parts(parts)) => decode_content_parts(parts),
    }
}

fn decode_content_parts(parts: Vec<ContentPartIn>) -> Result<Vec<Part>, Error> {
    parts
        .into_iter()
        .map(|part| match part {
            ContentPartIn::Text { text } => Ok(Part::Text(text)),
            ContentPartIn::ImageUrl { image_url, file_id } => {
                if file_id.is_some() {
                    return Err(unsupported("chat image file_id"));
                }
                let image_url = image_url.ok_or_else(|| {
                    Error::InvalidInput("chat image_url part missing image_url".to_owned())
                })?;
                Ok(Part::Image(ImageRef {
                    source: decode_openai_image_url(image_url.url)?,
                    detail: image_url.detail.map(String::into_boxed_str),
                }))
            }
        })
        .collect()
}

fn decode_tool_call(tool_call: ToolCallIn) -> Result<ToolUse, Error> {
    if tool_call.kind != "function" {
        return Err(unsupported(format!(
            "chat tool call type {}",
            tool_call.kind
        )));
    }
    Ok(ToolUse {
        id: ToolId(tool_call.id.into()),
        name: tool_call.function.name.into(),
        arguments: RawJson::from_raw(tool_call.function.arguments),
        kind: ToolUseKind::Client,
    })
}

fn decode_tool_result(message: MessageIn) -> Result<ToolResult, Error> {
    let tool_use_id = message
        .tool_call_id
        .ok_or_else(|| Error::InvalidInput("chat tool message missing tool_call_id".to_owned()))?;
    let content = match message.content {
        Some(ContentIn::Text(text)) => ToolResultContent::Text(text.into()),
        Some(ContentIn::Parts(parts)) => ToolResultContent::Parts(decode_content_parts(parts)?),
        None => ToolResultContent::Text(Box::from("")),
    };
    Ok(ToolResult {
        tool_use_id: ToolId(tool_use_id.into()),
        content,
    })
}

fn decode_tools(tools: Option<Vec<ToolIn>>) -> Result<Vec<ToolDef>, Error> {
    tools
        .unwrap_or_default()
        .into_iter()
        .map(|tool| {
            if let Some(kind) = tool.kind.as_deref() {
                if kind != "function" {
                    return Err(unsupported(format!("chat tool type {kind}")));
                }
            }
            let parameters = tool
                .function
                .parameters
                .map(|raw| RawJson::from_raw(raw.get().to_owned()))
                .unwrap_or_else(|| RawJson::from_raw("{}"));
            Ok(ToolDef {
                name: tool.function.name.into(),
                description: tool.function.description.map(Into::into),
                parameters,
                strict: tool.function.strict,
            })
        })
        .collect()
}

fn decode_tool_choice(raw: Option<Box<RawValue>>) -> Result<ToolChoice, Error> {
    let Some(raw) = raw else {
        return Ok(ToolChoice::Auto);
    };
    match raw.get() {
        "\"auto\"" => Ok(ToolChoice::Auto),
        "\"required\"" => Ok(ToolChoice::Required),
        "\"none\"" => Ok(ToolChoice::None),
        _ => {
            let value: serde_json::Value = parse_str(raw.get())?;
            if value.get("type").and_then(serde_json::Value::as_str) == Some("function") {
                if let Some(name) = value
                    .get("function")
                    .and_then(|function| function.get("name"))
                    .and_then(serde_json::Value::as_str)
                {
                    return Ok(ToolChoice::Named(name.into()));
                }
            }
            Ok(ToolChoice::Other(RawJson::from_raw(raw.get().to_owned())))
        }
    }
}

fn decode_stop(stop: Option<StopIn>) -> Vec<Box<str>> {
    match stop {
        None => Vec::new(),
        Some(StopIn::Text(text)) => vec![text.into()],
        Some(StopIn::List(values)) => values.into_iter().map(Into::into).collect(),
    }
}

fn decode_reasoning(effort: Option<String>) -> Result<Reasoning, Error> {
    match effort {
        None => Ok(Reasoning::default()),
        Some(effort) => Ok(Reasoning {
            enabled: effort != "none",
            effort: Some(reasoning_effort_from_name(&effort)?),
            budget_tokens: None,
        }),
    }
}

fn reasoning_effort_from_name(value: &str) -> Result<ReasoningEffort, Error> {
    match value {
        "minimal" => Ok(ReasoningEffort::Minimal),
        "low" => Ok(ReasoningEffort::Low),
        "medium" => Ok(ReasoningEffort::Medium),
        "high" => Ok(ReasoningEffort::High),
        "none" => Ok(ReasoningEffort::None),
        _ => Err(unsupported(format!("chat reasoning_effort {value}"))),
    }
}

fn reasoning_effort_name(effort: ReasoningEffort) -> &'static str {
    match effort {
        ReasoningEffort::Minimal => "minimal",
        ReasoningEffort::Low => "low",
        ReasoningEffort::Medium => "medium",
        ReasoningEffort::High => "high",
        ReasoningEffort::None => "none",
    }
}

fn decode_finish(finish_reason: Option<String>, provider_finish: Option<String>) -> Finish {
    decode_finish_with_report(
        finish_reason,
        provider_finish,
        "finish_reason",
        &mut Report::new(),
    )
}

fn decode_finish_with_report(
    finish_reason: Option<String>,
    provider_finish: Option<String>,
    field: &str,
    report: &mut Report,
) -> Finish {
    let provider_raw = provider_finish
        .or_else(|| finish_reason.clone())
        .unwrap_or_default();
    let canonical = match finish_reason.as_deref() {
        Some("stop") => StopReason::EndTurn,
        Some("length") => StopReason::MaxTokens,
        Some("tool_calls") => StopReason::ToolUse,
        Some("content_filter") => StopReason::ContentFilter,
        Some("function_call") => {
            report.unmapped(field, UnmappedReason::NotRepresentable, Severity::Degraded);
            StopReason::ToolUse
        }
        Some(value) => {
            report.unmapped(field, UnmappedReason::NotRepresentable, Severity::Degraded);
            StopReason::Other(value.into())
        }
        None => StopReason::Other(Box::from("")),
    };
    Finish {
        canonical,
        provider_raw: provider_raw.into(),
    }
}

fn decode_usage(usage: UsageIn) -> Usage {
    Usage {
        input: usage.prompt_tokens,
        output: usage.completion_tokens,
        cached: usage
            .prompt_tokens_details
            .and_then(|details| details.cached_tokens)
            .unwrap_or(0),
        cache_creation: 0,
        reasoning: usage
            .completion_tokens_details
            .and_then(|details| details.reasoning_tokens)
            .unwrap_or(0),
    }
}

fn encode_user_turn(turn: &Turn, messages: &mut Vec<MessageOut>) -> Result<(), Error> {
    let mut pending = Vec::new();
    for part in &turn.parts {
        match part {
            Part::ToolResult(result) => {
                flush_user_content(&mut pending, messages);
                messages.push(encode_tool_result(result)?);
            }
            Part::Text(text) => pending.push(ContentPartOut::Text { text: text.clone() }),
            Part::Image(image) => pending.push(ContentPartOut::ImageUrl {
                image_url: ImageUrlOut {
                    url: encode_openai_image_url(&image.source),
                    detail: image.detail.as_deref().map(str::to_owned),
                },
            }),
            _ => return Err(unsupported("chat user part")),
        }
    }
    flush_user_content(&mut pending, messages);
    Ok(())
}

fn flush_user_content(pending: &mut Vec<ContentPartOut>, messages: &mut Vec<MessageOut>) {
    if pending.is_empty() {
        return;
    }
    let parts = std::mem::take(pending);
    let content = if let [ContentPartOut::Text { text }] = parts.as_slice() {
        ContentOut::Text(text.clone())
    } else {
        ContentOut::Parts(parts)
    };
    messages.push(MessageOut {
        role: "user",
        content: Some(content),
        tool_calls: None,
        tool_call_id: None,
    });
}

fn encode_tool_result(result: &ToolResult) -> Result<MessageOut, Error> {
    let content = match &result.content {
        ToolResultContent::Text(text) => ContentOut::Text(text.to_string()),
        ToolResultContent::Parts(parts) => ContentOut::Parts(encode_content_parts(parts)?),
        ToolResultContent::Object(_) => return Err(unsupported("chat tool result object")),
    };
    Ok(MessageOut {
        role: "tool",
        content: Some(content),
        tool_calls: None,
        tool_call_id: Some(result.tool_use_id.0.to_string()),
    })
}

fn encode_assistant_turn(turn: &Turn, messages: &mut Vec<MessageOut>) -> Result<(), Error> {
    let mut text = String::new();
    let mut has_text = false;
    let mut tool_calls = Vec::new();

    for part in &turn.parts {
        match part {
            Part::Text(value) => {
                text.push_str(value);
                has_text = true;
            }
            Part::ToolUse(tool_use) => tool_calls.push(encode_tool_call(tool_use)?),
            _ => return Err(unsupported("chat assistant part")),
        }
    }

    messages.push(MessageOut {
        role: "assistant",
        content: has_text.then_some(ContentOut::Text(text)),
        tool_calls: (!tool_calls.is_empty()).then_some(tool_calls),
        tool_call_id: None,
    });
    Ok(())
}

fn encode_tool_call(tool_use: &ToolUse) -> Result<ToolCallOut, Error> {
    if tool_use.kind != ToolUseKind::Client {
        return Err(unsupported("chat non-client tool use"));
    }
    Ok(ToolCallOut {
        id: tool_use.id.0.to_string(),
        kind: "function",
        function: FunctionCallOut {
            name: tool_use.name.to_string(),
            arguments: tool_use.arguments.raw().to_owned(),
        },
    })
}

fn encode_content_parts(parts: &[Part]) -> Result<Vec<ContentPartOut>, Error> {
    parts
        .iter()
        .map(|part| match part {
            Part::Text(text) => Ok(ContentPartOut::Text { text: text.clone() }),
            Part::Image(image) => Ok(ContentPartOut::ImageUrl {
                image_url: ImageUrlOut {
                    url: encode_openai_image_url(&image.source),
                    detail: image.detail.as_deref().map(str::to_owned),
                },
            }),
            _ => Err(unsupported("chat content part")),
        })
        .collect()
}

fn encode_tool(tool: &ToolDef) -> Result<ToolOut, Error> {
    Ok(ToolOut {
        kind: "function",
        function: ToolFunctionOut {
            name: tool.name.to_string(),
            description: tool.description.as_ref().map(ToString::to_string),
            parameters: raw_value(&tool.parameters)?,
            strict: tool.strict,
        },
    })
}

fn encode_tool_choice(tool_choice: &ToolChoice) -> Result<Option<ToolChoiceOut>, Error> {
    match tool_choice {
        ToolChoice::Auto => Ok(Some(ToolChoiceOut::Keyword("auto"))),
        ToolChoice::Required => Ok(Some(ToolChoiceOut::Keyword("required"))),
        ToolChoice::None => Ok(Some(ToolChoiceOut::Keyword("none"))),
        ToolChoice::Named(name) => Ok(Some(ToolChoiceOut::Named(NamedToolChoiceOut {
            kind: "function",
            function: NamedFunctionOut {
                name: name.to_string(),
            },
        }))),
        ToolChoice::Other(raw) => Ok(Some(ToolChoiceOut::Other(raw_value(raw)?))),
    }
}

fn encode_stop(stop: &[Box<str>]) -> Option<StopOut> {
    match stop {
        [] => None,
        [value] => Some(StopOut::Text(value.to_string())),
        values => Some(StopOut::List(
            values.iter().map(ToString::to_string).collect(),
        )),
    }
}

fn encode_response_choice(choice: &Choice) -> Result<ResponseChoiceOut, Error> {
    let mut text = String::new();
    let mut has_text = false;
    let mut tool_calls = Vec::new();

    for part in &choice.parts {
        match part {
            Part::Text(value) => {
                text.push_str(value);
                has_text = true;
            }
            Part::ToolUse(tool_use) => tool_calls.push(encode_tool_call(tool_use)?),
            _ => return Err(unsupported("chat response part")),
        }
    }

    Ok(ResponseChoiceOut {
        index: choice.index,
        message: MessageOut {
            role: "assistant",
            content: has_text.then_some(ContentOut::Text(text)),
            tool_calls: (!tool_calls.is_empty()).then_some(tool_calls),
            tool_call_id: None,
        },
        finish_reason: stop_reason_name(&choice.finish),
    })
}

fn stop_reason_name(finish: &Finish) -> String {
    match &finish.canonical {
        StopReason::EndTurn => "stop".to_owned(),
        StopReason::MaxTokens => "length".to_owned(),
        StopReason::ToolUse => "tool_calls".to_owned(),
        StopReason::ContentFilter => "content_filter".to_owned(),
        StopReason::StopSequence => "stop".to_owned(),
        StopReason::Pause => "stop".to_owned(),
        StopReason::Cancelled => "stop".to_owned(),
        StopReason::Other(value) if value.is_empty() => "stop".to_owned(),
        StopReason::Other(value) => value.to_string(),
    }
}

fn encode_usage(usage: Usage) -> UsageOut {
    UsageOut {
        prompt_tokens: usage.input,
        completion_tokens: usage.output,
        total_tokens: usage
            .input
            .zip(usage.output)
            .map(|(input, output)| input + output),
        prompt_tokens_details: Some(PromptTokensDetailsOut {
            cached_tokens: usage.cached,
        }),
        completion_tokens_details: Some(CompletionTokensDetailsOut {
            reasoning_tokens: usage.reasoning,
        }),
    }
}

fn raw_value(raw: &RawJson) -> Result<Box<RawValue>, Error> {
    RawValue::from_string(raw.raw().to_owned())
        .map_err(|error| Error::InvalidInput(error.to_string()))
}

fn parse_json<T: DeserializeOwned>(body: &[u8]) -> Result<T, Error> {
    serde_json::from_slice(body).map_err(|error| Error::InvalidInput(error.to_string()))
}

fn parse_str<T: DeserializeOwned>(body: &str) -> Result<T, Error> {
    serde_json::from_str(body).map_err(|error| Error::InvalidInput(error.to_string()))
}

fn serialize_json<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, Error> {
    serde_json::to_vec(value).map_err(|error| Error::Protocol(error.to_string()))
}

fn unsupported(value: impl ToString) -> Error {
    Error::Unsupported(value.to_string())
}
