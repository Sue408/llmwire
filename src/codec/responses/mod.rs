mod stream;
mod wire;

use std::str;

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::value::RawValue;

use crate::codec::ProtocolCodec;
use crate::ids::OpaqueKind;
use crate::ir::{
    AssistantOutput, Choice, Conversation, Finish, ImageRef, Opaque, Part, RawJson, Reasoning,
    ReasoningEffort, Role, Sampling, StopReason, Thinking, ToolChoice, ToolDef, ToolId, ToolResult,
    ToolResultContent, ToolUse, ToolUseKind, Turn, Usage,
};
use crate::Error;
use wire::*;

const RESPONSES_ITEM: &str = "responses_item";
const RESPONSES_CONTENT_PART: &str = "responses_content_part";

#[derive(Debug, Default, Clone, Copy)]
pub struct Responses;

impl ProtocolCodec for Responses {
    fn decode_request(&self, body: &[u8]) -> Result<Conversation, Error> {
        let request: ResponsesRequestIn = parse_json(body)?;
        if request.store == Some(true) {
            return Err(unsupported(
                "responses store=true is not supported in stateless mode",
            ));
        }
        if request.previous_response_id.is_some() {
            return Err(unsupported(
                "responses previous_response_id is not supported in stateless mode",
            ));
        }
        if let Some(max_output_tokens) = request.max_output_tokens {
            if max_output_tokens < 16 {
                return Err(Error::InvalidInput(
                    "responses max_output_tokens must be at least 16".to_owned(),
                ));
            }
        }

        let (mut system, turns) = decode_input(request.input)?;
        if let Some(instructions) = request.instructions {
            system.insert(0, Part::Text(instructions));
        }

        Ok(Conversation {
            system,
            turns,
            tools: decode_tools(request.tools)?,
            tool_choice: decode_tool_choice(request.tool_choice)?,
            sampling: Sampling {
                temperature: request.temperature,
                top_p: request.top_p,
                max_output_tokens: request.max_output_tokens,
                ..Sampling::default()
            },
            reasoning: decode_reasoning(request.reasoning)?,
        })
    }

    fn encode_request(&self, conversation: &Conversation) -> Result<Vec<u8>, Error> {
        if let Some(max_output_tokens) = conversation.sampling.max_output_tokens {
            if max_output_tokens < 16 {
                return Err(Error::InvalidInput(
                    "responses max_output_tokens must be at least 16".to_owned(),
                ));
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

        let output = ResponsesRequestOut {
            model: "llmwire",
            instructions: encode_instructions(&conversation.system)?,
            input: encode_input(&conversation.turns)?,
            tools,
            tool_choice: Some(encode_tool_choice(&conversation.tool_choice)?),
            reasoning: encode_reasoning(&conversation.reasoning),
            max_output_tokens: conversation.sampling.max_output_tokens,
            temperature: conversation.sampling.temperature,
            top_p: conversation.sampling.top_p,
        };

        serialize_json(&output)
    }

    fn decode_response(&self, body: &[u8]) -> Result<AssistantOutput, Error> {
        let response: ResponsesResponseIn = parse_json(body)?;
        if response.status == "failed" {
            let message = response
                .error
                .and_then(|error| error.message)
                .unwrap_or_else(|| "responses failed".to_owned());
            return Err(Error::Protocol(message));
        }

        let last_item = response.output.last();
        let last_kind = last_item
            .map(|raw| parse_value::<ItemMetaIn>(raw).map(|meta| meta.kind))
            .transpose()?;
        let mut parts = Vec::new();
        for item in response.output {
            parts.extend(decode_item(item)?.into_parts());
        }

        let finish = decode_finish(
            &response.status,
            response.incomplete_details,
            last_kind.as_deref() == Some("function_call"),
        )?;

        Ok(AssistantOutput {
            choices: vec![Choice {
                index: 0,
                parts,
                finish,
            }],
            usage: response.usage.map(decode_usage).unwrap_or_default(),
        })
    }

    fn encode_response(&self, output: &AssistantOutput) -> Result<Vec<u8>, Error> {
        if output.choices.len() > 1 {
            return Err(unsupported("responses multiple choices"));
        }

        let choice = output.choices.first();
        let mut items = Vec::new();
        if let Some(choice) = choice {
            for part in &choice.parts {
                items.extend(encode_response_part(part)?);
            }
        }

        let (status, incomplete_details) = choice
            .map(|choice| encode_status(&choice.finish))
            .unwrap_or_else(|| ("completed".to_owned(), None));

        let response = ResponsesResponseOut {
            id: "resp_llmwire",
            object: "response",
            created_at: 0,
            status,
            model: "llmwire",
            output: items,
            incomplete_details,
            usage: encode_usage(output.usage),
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

enum DecodedItem {
    User(Vec<Part>),
    Assistant(Vec<Part>),
    System(Vec<Part>),
}

impl DecodedItem {
    fn into_parts(self) -> Vec<Part> {
        match self {
            Self::User(parts) | Self::Assistant(parts) | Self::System(parts) => parts,
        }
    }
}

fn decode_input(input: Option<ResponsesInputIn>) -> Result<(Vec<Part>, Vec<Turn>), Error> {
    let Some(input) = input else {
        return Ok((Vec::new(), Vec::new()));
    };

    match input {
        ResponsesInputIn::Text(text) => Ok((
            Vec::new(),
            vec![Turn {
                role: Role::User,
                parts: vec![Part::Text(text)],
            }],
        )),
        ResponsesInputIn::Items(items) => {
            let mut system = Vec::new();
            let mut turns = Vec::new();
            for item in items {
                match decode_item(item)? {
                    DecodedItem::User(parts) => push_turn_parts(&mut turns, Role::User, parts),
                    DecodedItem::Assistant(parts) => {
                        push_turn_parts(&mut turns, Role::Assistant, parts)
                    }
                    DecodedItem::System(parts) => system.extend(parts),
                }
            }
            Ok((system, turns))
        }
    }
}

fn push_turn_parts(turns: &mut Vec<Turn>, role: Role, parts: Vec<Part>) {
    if let Some(turn) = turns.last_mut() {
        if turn.role == role {
            turn.parts.extend(parts);
            return;
        }
    }
    turns.push(Turn { role, parts });
}

fn decode_item(raw: serde_json::Value) -> Result<DecodedItem, Error> {
    let meta: ItemMetaIn = parse_value(&raw)?;
    match meta.kind.as_str() {
        "message" => {
            let item: MessageItemIn = parse_value(&raw)?;
            let parts = decode_message_content(item.content)?;
            match item.role.as_str() {
                "user" => Ok(DecodedItem::User(parts)),
                "assistant" => Ok(DecodedItem::Assistant(parts)),
                "system" | "developer" => Ok(DecodedItem::System(parts)),
                role => Err(unsupported(format!("responses message role {role}"))),
            }
        }
        "function_call" => {
            let item: FunctionCallItemIn = parse_value(&raw)?;
            Ok(DecodedItem::Assistant(vec![Part::ToolUse(ToolUse {
                id: ToolId(item.call_id.into_boxed_str()),
                name: item.name.into_boxed_str(),
                arguments: RawJson::from_raw(item.arguments),
                kind: ToolUseKind::Client,
            })]))
        }
        "function_call_output" => {
            let item: FunctionCallOutputItemIn = parse_value(&raw)?;
            Ok(DecodedItem::User(vec![Part::ToolResult(ToolResult {
                tool_use_id: ToolId(item.call_id.into_boxed_str()),
                content: decode_tool_result_content(&item.output)?,
            })]))
        }
        "reasoning" => {
            let item: ReasoningItemIn = parse_value(&raw)?;
            let mut parts = Vec::new();
            if let Some(summary) = item.summary {
                for part in summary {
                    if !part.text.is_empty() {
                        parts.push(Part::Thinking(Thinking {
                            text: part.text,
                            signature: None,
                        }));
                    }
                }
            }
            if let Some(encrypted_content) = item.encrypted_content {
                parts.push(Part::Opaque(Opaque {
                    kind: OpaqueKind::ResponsesEncryptedReasoning,
                    bytes: encrypted_content.into_bytes().into_boxed_slice(),
                }));
            }
            if parts.is_empty() {
                parts.push(opaque_item(&raw));
            }
            Ok(DecodedItem::Assistant(parts))
        }
        _ => Ok(DecodedItem::Assistant(vec![opaque_item(&raw)])),
    }
}

fn decode_message_content(content: Option<MessageContentIn>) -> Result<Vec<Part>, Error> {
    match content {
        None => Ok(Vec::new()),
        Some(MessageContentIn::Text(text)) => Ok(vec![Part::Text(text)]),
        Some(MessageContentIn::Parts(parts)) => parts
            .into_iter()
            .map(|raw| {
                let meta: ItemMetaIn = parse_value(&raw)?;
                match meta.kind.as_str() {
                    "input_text" | "output_text" | "text" => {
                        Ok(Part::Text(parse_value::<TextPartIn>(&raw)?.text))
                    }
                    "refusal" => Ok(Part::Text(parse_value::<RefusalPartIn>(&raw)?.refusal)),
                    "input_image" => {
                        let part: ImagePartIn = parse_value(&raw)?;
                        Ok(Part::Image(ImageRef::Url(match part.image_url {
                            ImageUrlIn::Url(url) => url.into_boxed_str(),
                            ImageUrlIn::Object { url } => url.into_boxed_str(),
                        })))
                    }
                    _ => Ok(Part::Opaque(Opaque {
                        kind: OpaqueKind::ProviderSpecific(RESPONSES_CONTENT_PART),
                        bytes: value_bytes(&raw),
                    })),
                }
            })
            .collect(),
    }
}

fn decode_tool_result_content(raw: &serde_json::Value) -> Result<ToolResultContent, Error> {
    if let Some(text) = raw.as_str() {
        Ok(ToolResultContent::Text(text.into()))
    } else {
        Ok(ToolResultContent::Object(RawJson::from_raw(
            raw.to_string(),
        )))
    }
}

fn decode_tools(tools: Option<Vec<ResponsesToolIn>>) -> Result<Vec<ToolDef>, Error> {
    tools
        .unwrap_or_default()
        .into_iter()
        .map(|tool| {
            if let Some(kind) = tool.kind.as_deref() {
                if kind != "function" {
                    return Err(unsupported(format!("responses tool type {kind}")));
                }
            }
            Ok(ToolDef {
                name: tool.name.into_boxed_str(),
                description: tool.description.map(String::into_boxed_str),
                parameters: tool
                    .parameters
                    .map(|raw| RawJson::from_raw(raw.get().to_owned()))
                    .unwrap_or_else(|| RawJson::from_raw("{}")),
                strict: tool.strict,
            })
        })
        .collect()
}

fn decode_tool_choice(raw: Option<Box<RawValue>>) -> Result<ToolChoice, Error> {
    let Some(raw) = raw else {
        return Ok(ToolChoice::Auto);
    };
    let value: serde_json::Value = parse_str(raw.get())?;
    if let Some(keyword) = value.as_str() {
        return match keyword {
            "auto" => Ok(ToolChoice::Auto),
            "required" => Ok(ToolChoice::Required),
            "none" => Ok(ToolChoice::None),
            _ => Ok(ToolChoice::Other(RawJson::from_raw(raw.get().to_owned()))),
        };
    }
    if value.get("type").and_then(serde_json::Value::as_str) == Some("function") {
        if let Some(name) = value.get("name").and_then(serde_json::Value::as_str) {
            return Ok(ToolChoice::Named(name.into()));
        }
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

fn decode_reasoning(config: Option<ResponsesReasoningIn>) -> Result<Reasoning, Error> {
    let Some(config) = config else {
        return Ok(Reasoning::default());
    };
    let Some(effort) = config.effort else {
        return Ok(Reasoning::default());
    };
    Ok(Reasoning {
        enabled: true,
        effort: Some(reasoning_effort_from_name(&effort)?),
        budget_tokens: None,
    })
}

fn decode_finish(
    status: &str,
    incomplete: Option<ResponsesIncompleteDetailsIn>,
    last_is_function_call: bool,
) -> Result<Finish, Error> {
    let provider_raw = incomplete
        .as_ref()
        .and_then(|details| details.reason.clone())
        .unwrap_or_else(|| status.to_owned());
    let canonical = match status {
        "completed" if last_is_function_call => StopReason::ToolUse,
        "completed" => StopReason::EndTurn,
        "incomplete" => match incomplete.and_then(|details| details.reason).as_deref() {
            Some("max_output_tokens") => StopReason::MaxTokens,
            Some("content_filter") => StopReason::ContentFilter,
            Some(reason) => StopReason::Other(reason.into()),
            None => StopReason::Other("".into()),
        },
        "cancelled" => StopReason::Cancelled,
        "queued" | "in_progress" => {
            return Err(Error::Protocol(format!(
                "responses status {status} is not terminal"
            )));
        }
        other => StopReason::Other(other.into()),
    };
    Ok(Finish {
        canonical,
        provider_raw: provider_raw.into_boxed_str(),
    })
}

fn decode_usage(usage: ResponsesUsageIn) -> Usage {
    Usage {
        input: usage.input_tokens,
        output: usage.output_tokens,
        cached: usage
            .input_tokens_details
            .and_then(|details| details.cached_tokens)
            .unwrap_or(0),
        cache_creation: 0,
        reasoning: usage
            .output_tokens_details
            .and_then(|details| details.reasoning_tokens)
            .unwrap_or(0),
    }
}

fn encode_instructions(system: &[Part]) -> Result<Option<String>, Error> {
    if system.is_empty() {
        return Ok(None);
    }
    let mut instructions = Vec::new();
    for part in system {
        match part {
            Part::Text(text) => instructions.push(text.clone()),
            _ => return Err(unsupported("responses instructions part")),
        }
    }
    Ok(Some(instructions.join("\n\n")))
}

fn encode_input(turns: &[Turn]) -> Result<Vec<Box<RawValue>>, Error> {
    let mut items = Vec::new();
    for turn in turns {
        for part in &turn.parts {
            items.extend(encode_input_part(turn.role, part)?);
        }
    }
    Ok(items)
}

fn encode_input_part(role: Role, part: &Part) -> Result<Vec<Box<RawValue>>, Error> {
    let value = match part {
        Part::Text(text) => {
            let content_type = match role {
                Role::User => "input_text",
                Role::Assistant => "output_text",
            };
            let role = match role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            vec![raw_from_serializable(&serde_json::json!({
                "type": "message",
                "role": role,
                "content": [{"type": content_type, "text": text}],
            }))?]
        }
        Part::Image(ImageRef::Url(url)) => vec![raw_from_serializable(&serde_json::json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_image", "image_url": url}],
        }))?],
        Part::Image(ImageRef::Base64 { media_type, data }) => {
            let url = format!("data:{media_type};base64,{data}");
            vec![raw_from_serializable(&serde_json::json!({
                "type": "message",
                "role": "user",
                "content": [{"type": "input_image", "image_url": url}],
            }))?]
        }
        Part::ToolUse(tool_use) => vec![raw_from_serializable(&serde_json::json!({
            "type": "function_call",
            "call_id": tool_use.id.0,
            "name": tool_use.name,
            "arguments": tool_use.arguments.raw(),
        }))?],
        Part::ToolResult(result) => vec![raw_from_serializable(&serde_json::json!({
            "type": "function_call_output",
            "call_id": result.tool_use_id.0,
            "output": encode_tool_result_content(&result.content)?,
        }))?],
        Part::Opaque(opaque) => match &opaque.kind {
            OpaqueKind::ResponsesEncryptedReasoning => {
                vec![raw_from_serializable(&serde_json::json!({
                    "type": "reasoning",
                    "encrypted_content": bytes_to_string(&opaque.bytes)?,
                }))?]
            }
            OpaqueKind::ProviderSpecific(kind) if *kind == RESPONSES_ITEM => {
                vec![raw_from_bytes(&opaque.bytes)?]
            }
            _ => return Err(unsupported("responses input opaque")),
        },
        Part::Thinking(_) => return Err(unsupported("responses input thinking")),
    };
    Ok(value)
}

fn encode_tool_result_content(content: &ToolResultContent) -> Result<serde_json::Value, Error> {
    match content {
        ToolResultContent::Text(text) => Ok(serde_json::Value::String(text.to_string())),
        ToolResultContent::Object(raw) => parse_str(raw.raw()),
        ToolResultContent::Parts(_) => Err(unsupported("responses tool result parts")),
    }
}

fn encode_tool(tool: &ToolDef) -> Result<ResponsesToolOut, Error> {
    Ok(ResponsesToolOut {
        kind: "function",
        name: tool.name.to_string(),
        description: tool.description.as_ref().map(ToString::to_string),
        parameters: raw_value(&tool.parameters)?,
        strict: tool.strict,
    })
}

fn encode_tool_choice(tool_choice: &ToolChoice) -> Result<Box<RawValue>, Error> {
    match tool_choice {
        ToolChoice::Auto => raw_from_serializable(&"auto"),
        ToolChoice::Required => raw_from_serializable(&"required"),
        ToolChoice::None => raw_from_serializable(&"none"),
        ToolChoice::Named(name) => raw_from_serializable(&serde_json::json!({
            "type": "function",
            "name": name,
        })),
        ToolChoice::Other(raw) => raw_value(raw),
    }
}

fn encode_reasoning(reasoning: &Reasoning) -> Option<ResponsesReasoningOut> {
    let effort = reasoning.effort?;
    Some(ResponsesReasoningOut {
        effort: reasoning_effort_name(effort).to_owned(),
    })
}

fn encode_response_part(part: &Part) -> Result<Vec<Box<RawValue>>, Error> {
    let value = match part {
        Part::Text(text) => vec![raw_from_serializable(&serde_json::json!({
            "type": "message",
            "id": "msg_llmwire",
            "role": "assistant",
            "status": "completed",
            "content": [{"type": "output_text", "text": text, "annotations": []}],
        }))?],
        Part::ToolUse(tool_use) => vec![raw_from_serializable(&serde_json::json!({
            "type": "function_call",
            "id": format!("fc_llmwire_{}", tool_use.id.0),
            "call_id": tool_use.id.0,
            "name": tool_use.name,
            "arguments": tool_use.arguments.raw(),
            "status": "completed",
        }))?],
        Part::Thinking(thinking) => {
            if thinking.signature.is_some() {
                return Err(unsupported("responses thinking signature"));
            }
            vec![raw_from_serializable(&serde_json::json!({
                "type": "reasoning",
                "id": "rs_llmwire",
                "summary": [{"type": "summary_text", "text": thinking.text}],
                "status": "completed",
            }))?]
        }
        Part::Opaque(opaque) => match &opaque.kind {
            OpaqueKind::ResponsesEncryptedReasoning => {
                vec![raw_from_serializable(&serde_json::json!({
                    "type": "reasoning",
                    "id": "rs_llmwire",
                    "summary": [],
                    "encrypted_content": bytes_to_string(&opaque.bytes)?,
                    "status": "completed",
                }))?]
            }
            OpaqueKind::ProviderSpecific(kind) if *kind == RESPONSES_ITEM => {
                vec![raw_from_bytes(&opaque.bytes)?]
            }
            _ => return Err(unsupported("responses response opaque")),
        },
        Part::ToolResult(_) => return Err(unsupported("responses response tool result")),
        Part::Image(_) => return Err(unsupported("responses response image")),
    };
    Ok(value)
}

fn encode_status(finish: &Finish) -> (String, Option<ResponsesIncompleteDetailsOut>) {
    match &finish.canonical {
        StopReason::MaxTokens => (
            "incomplete".to_owned(),
            Some(ResponsesIncompleteDetailsOut {
                reason: "max_output_tokens".to_owned(),
            }),
        ),
        StopReason::ContentFilter => (
            "incomplete".to_owned(),
            Some(ResponsesIncompleteDetailsOut {
                reason: "content_filter".to_owned(),
            }),
        ),
        StopReason::Cancelled => ("cancelled".to_owned(), None),
        StopReason::Other(reason) => (
            "incomplete".to_owned(),
            Some(ResponsesIncompleteDetailsOut {
                reason: reason.to_string(),
            }),
        ),
        _ => ("completed".to_owned(), None),
    }
}

fn encode_usage(usage: Usage) -> ResponsesUsageOut {
    ResponsesUsageOut {
        input_tokens: usage.input,
        output_tokens: usage.output,
        total_tokens: match (usage.input, usage.output) {
            (Some(input), Some(output)) => Some(input + output),
            _ => None,
        },
        input_tokens_details: (usage.cached > 0).then_some(InputTokensDetailsOut {
            cached_tokens: usage.cached,
        }),
        output_tokens_details: (usage.reasoning > 0).then_some(OutputTokensDetailsOut {
            reasoning_tokens: usage.reasoning,
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
        value => Err(unsupported(format!("responses reasoning effort {value}"))),
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

fn opaque_item(raw: &serde_json::Value) -> Part {
    Part::Opaque(Opaque {
        kind: OpaqueKind::ProviderSpecific(RESPONSES_ITEM),
        bytes: value_bytes(raw),
    })
}

fn value_bytes(value: &serde_json::Value) -> Box<[u8]> {
    value.to_string().into_bytes().into_boxed_slice()
}

fn parse_value<T: DeserializeOwned>(value: &serde_json::Value) -> Result<T, Error> {
    serde_json::from_value(value.clone()).map_err(|error| Error::InvalidInput(error.to_string()))
}

fn raw_value(raw: &RawJson) -> Result<Box<RawValue>, Error> {
    RawValue::from_string(raw.raw().to_owned())
        .map_err(|error| Error::InvalidInput(error.to_string()))
}

fn raw_from_bytes(bytes: &[u8]) -> Result<Box<RawValue>, Error> {
    RawValue::from_string(bytes_to_string(bytes)?)
        .map_err(|error| Error::InvalidInput(error.to_string()))
}

fn raw_from_serializable<T: Serialize>(value: &T) -> Result<Box<RawValue>, Error> {
    let raw = serde_json::to_string(value).map_err(|error| Error::Protocol(error.to_string()))?;
    RawValue::from_string(raw).map_err(|error| Error::Protocol(error.to_string()))
}

fn bytes_to_string(bytes: &[u8]) -> Result<String, Error> {
    str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|error| Error::InvalidInput(error.to_string()))
}

fn parse_json<T: DeserializeOwned>(body: &[u8]) -> Result<T, Error> {
    serde_json::from_slice(body).map_err(|error| Error::InvalidInput(error.to_string()))
}

fn parse_str<T: DeserializeOwned>(body: &str) -> Result<T, Error> {
    serde_json::from_str(body).map_err(|error| Error::InvalidInput(error.to_string()))
}

fn serialize_json<T: Serialize>(value: &T) -> Result<Vec<u8>, Error> {
    serde_json::to_vec(value).map_err(|error| Error::Protocol(error.to_string()))
}

fn unsupported(value: impl ToString) -> Error {
    Error::Unsupported(value.to_string())
}
