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
    ToolResultContent, ToolUse, Turn, Usage,
};
use crate::Error;
use wire::*;

const DEFAULT_MAX_TOKENS: u32 = 4096;
const DEFAULT_THINKING_BUDGET: u64 = 1024;
const ANTHROPIC_CONTENT_BLOCK: &str = "anthropic_content_block";
const ANTHROPIC_SYSTEM_BLOCK: &str = "anthropic_system_block";

#[derive(Debug, Default, Clone, Copy)]
pub struct Messages;

impl ProtocolCodec for Messages {
    fn decode_request(&self, body: &[u8]) -> Result<Conversation, Error> {
        let request: MessagesRequestIn = parse_json(body)?;
        let max_tokens = request
            .max_tokens
            .ok_or_else(|| Error::InvalidInput("messages max_tokens is required".to_owned()))?;
        let system = decode_system(request.system)?;
        let turns = request
            .messages
            .into_iter()
            .map(decode_message)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Conversation {
            system,
            turns,
            tools: decode_tools(request.tools)?,
            tool_choice: decode_tool_choice(request.tool_choice)?,
            sampling: Sampling {
                temperature: request.temperature,
                top_p: request.top_p,
                top_k: request.top_k,
                max_output_tokens: Some(max_tokens),
                stop: request
                    .stop_sequences
                    .unwrap_or_default()
                    .into_iter()
                    .map(Into::into)
                    .collect(),
                ..Sampling::default()
            },
            reasoning: decode_reasoning(request.thinking)?,
        })
    }

    fn encode_request(&self, conversation: &Conversation) -> Result<Vec<u8>, Error> {
        let messages = conversation
            .turns
            .iter()
            .map(encode_message)
            .collect::<Result<Vec<_>, _>>()?;
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

        let output = MessagesRequestOut {
            model: "llmwire",
            max_tokens: conversation
                .sampling
                .max_output_tokens
                .unwrap_or(DEFAULT_MAX_TOKENS),
            messages,
            system: encode_system(&conversation.system)?,
            tools,
            tool_choice: Some(encode_tool_choice(&conversation.tool_choice)?),
            temperature: conversation.sampling.temperature,
            top_p: conversation.sampling.top_p,
            top_k: conversation.sampling.top_k,
            stop_sequences: encode_stop(&conversation.sampling.stop),
            thinking: encode_reasoning(&conversation.reasoning),
        };

        serialize_json(&output)
    }

    fn decode_response(&self, body: &[u8]) -> Result<AssistantOutput, Error> {
        let response: MessagesResponseIn = parse_json(body)?;
        let parts = decode_blocks(response.content)?;
        let choice = Choice {
            index: 0,
            parts,
            finish: decode_finish(response.stop_reason, response.stop_sequence),
        };

        Ok(AssistantOutput {
            choices: vec![choice],
            usage: response.usage.map(decode_usage).unwrap_or_default(),
        })
    }

    fn encode_response(&self, output: &AssistantOutput) -> Result<Vec<u8>, Error> {
        if output.choices.len() > 1 {
            return Err(unsupported("messages multiple choices"));
        }

        let choice = output.choices.first();
        let content = choice
            .map(|choice| encode_blocks(&choice.parts))
            .transpose()?
            .unwrap_or_default();
        let stop_reason = choice
            .map(|choice| stop_reason_name(&choice.finish))
            .unwrap_or_else(|| "end_turn".to_owned());
        let stop_sequence = choice.and_then(|choice| {
            matches!(choice.finish.canonical, StopReason::StopSequence)
                .then(|| choice.finish.provider_raw.to_string())
        });

        let response = MessagesResponseOut {
            id: "msg_llmwire",
            kind: "message",
            role: "assistant",
            content,
            model: "llmwire",
            stop_reason,
            stop_sequence,
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
fn decode_message(message: MessageIn) -> Result<Turn, Error> {
    let role = match message.role.as_str() {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        role => return Err(unsupported(format!("messages role {role}"))),
    };
    Ok(Turn {
        role,
        parts: decode_content(message.content)?,
    })
}

fn decode_content(content: Option<TextOrBlocks>) -> Result<Vec<Part>, Error> {
    match content {
        None => Ok(Vec::new()),
        Some(TextOrBlocks::Text(text)) => Ok(vec![Part::Text(text)]),
        Some(TextOrBlocks::Blocks(blocks)) => decode_blocks(blocks),
    }
}

fn decode_blocks(blocks: Vec<Box<RawValue>>) -> Result<Vec<Part>, Error> {
    blocks.into_iter().map(decode_block).collect()
}

fn decode_block(raw: Box<RawValue>) -> Result<Part, Error> {
    let meta: BlockMetaIn = parse_str(raw.get())?;
    if meta.cache_control.is_some() {
        return Ok(opaque_block(ANTHROPIC_CONTENT_BLOCK, &raw));
    }

    match meta.kind.as_str() {
        "text" => {
            let block: TextBlockIn = parse_str(raw.get())?;
            Ok(Part::Text(block.text))
        }
        "image" => {
            let block: ImageBlockIn = parse_str(raw.get())?;
            match block.source.kind.as_str() {
                "base64" => match (block.source.media_type, block.source.data) {
                    (Some(media_type), Some(data)) => Ok(Part::Image(ImageRef::Base64 {
                        media_type: media_type.into(),
                        data: data.into(),
                    })),
                    _ => Ok(opaque_block(ANTHROPIC_CONTENT_BLOCK, &raw)),
                },
                "url" => match block.source.url {
                    Some(url) => Ok(Part::Image(ImageRef::Url(url.into()))),
                    None => Ok(opaque_block(ANTHROPIC_CONTENT_BLOCK, &raw)),
                },
                _ => Ok(opaque_block(ANTHROPIC_CONTENT_BLOCK, &raw)),
            }
        }
        "tool_use" => {
            let block: ToolUseBlockIn = parse_str(raw.get())?;
            let id: Box<str> = block.id.into();
            let kind = if id.starts_with("srvtoolu_") {
                crate::ir::ToolUseKind::Server
            } else {
                crate::ir::ToolUseKind::Client
            };
            Ok(Part::ToolUse(ToolUse {
                id: ToolId(id),
                name: block.name.into(),
                arguments: RawJson::from_raw(block.input.get().to_owned()),
                kind,
            }))
        }
        "tool_result" => {
            let block: ToolResultBlockIn = parse_str(raw.get())?;
            Ok(Part::ToolResult(ToolResult {
                tool_use_id: ToolId(block.tool_use_id.into()),
                content: decode_tool_result_content(block.content)?,
            }))
        }
        "thinking" => {
            let block: ThinkingBlockIn = parse_str(raw.get())?;
            Ok(Part::Thinking(Thinking {
                text: block.thinking,
                signature: block.signature.map(|signature| Opaque {
                    kind: OpaqueKind::AnthropicThinkingSignature,
                    bytes: signature.into_bytes().into_boxed_slice(),
                }),
            }))
        }
        "redacted_thinking" => {
            let block: RedactedThinkingBlockIn = parse_str(raw.get())?;
            Ok(Part::Opaque(Opaque {
                kind: OpaqueKind::AnthropicRedactedThinking,
                bytes: block.data.into_bytes().into_boxed_slice(),
            }))
        }
        _ => Ok(opaque_block(ANTHROPIC_CONTENT_BLOCK, &raw)),
    }
}

fn decode_tool_result_content(
    content: Option<ToolResultContentIn>,
) -> Result<ToolResultContent, Error> {
    match content {
        None => Ok(ToolResultContent::Text("".into())),
        Some(ToolResultContentIn::Text(text)) => Ok(ToolResultContent::Text(text.into())),
        Some(ToolResultContentIn::Blocks(blocks)) => {
            Ok(ToolResultContent::Parts(decode_blocks(blocks)?))
        }
        Some(ToolResultContentIn::Other(raw)) => {
            let value: serde_json::Value = parse_str(raw.get())?;
            if value.is_object() {
                Ok(ToolResultContent::Object(RawJson::from_raw(
                    raw.get().to_owned(),
                )))
            } else {
                Err(unsupported("messages tool result content"))
            }
        }
    }
}

fn decode_system(system: Option<TextOrBlocks>) -> Result<Vec<Part>, Error> {
    match system {
        None => Ok(Vec::new()),
        Some(TextOrBlocks::Text(text)) => Ok(vec![Part::Text(text)]),
        Some(TextOrBlocks::Blocks(blocks)) => blocks
            .into_iter()
            .map(|raw| {
                let meta: BlockMetaIn = parse_str(raw.get())?;
                if meta.cache_control.is_some() || meta.kind != "text" {
                    return Ok(opaque_block(ANTHROPIC_SYSTEM_BLOCK, &raw));
                }
                let block: TextBlockIn = parse_str(raw.get())?;
                Ok(Part::Text(block.text))
            })
            .collect(),
    }
}

fn decode_tools(tools: Option<Vec<ToolIn>>) -> Result<Vec<ToolDef>, Error> {
    tools
        .unwrap_or_default()
        .into_iter()
        .map(|tool| {
            Ok(ToolDef {
                name: tool.name.into(),
                description: tool.description.map(Into::into),
                parameters: tool
                    .input_schema
                    .map(|raw| RawJson::from_raw(raw.get().to_owned()))
                    .unwrap_or_else(|| RawJson::from_raw("{}")),
                strict: None,
            })
        })
        .collect()
}

fn decode_tool_choice(raw: Option<Box<RawValue>>) -> Result<ToolChoice, Error> {
    let Some(raw) = raw else {
        return Ok(ToolChoice::Auto);
    };
    let value: serde_json::Value = parse_str(raw.get())?;
    match value.get("type").and_then(serde_json::Value::as_str) {
        Some("auto") => Ok(ToolChoice::Auto),
        Some("any") => Ok(ToolChoice::Required),
        Some("none") => Ok(ToolChoice::None),
        Some("tool") => value
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(|name| ToolChoice::Named(name.into()))
            .ok_or_else(|| Error::InvalidInput("messages tool_choice missing name".to_owned())),
        _ => Ok(ToolChoice::Other(RawJson::from_raw(raw.get().to_owned()))),
    }
}

fn decode_reasoning(config: Option<ThinkingConfigIn>) -> Result<Reasoning, Error> {
    let Some(config) = config else {
        return Ok(Reasoning::default());
    };
    match config.kind.as_deref() {
        Some("enabled") => Ok(Reasoning {
            enabled: true,
            effort: None,
            budget_tokens: config.budget_tokens,
        }),
        Some("disabled") | None => Ok(Reasoning::default()),
        Some(kind) => Err(unsupported(format!("messages thinking type {kind}"))),
    }
}

fn decode_finish(stop_reason: Option<String>, stop_sequence: Option<String>) -> Finish {
    let provider_raw = stop_sequence
        .or_else(|| stop_reason.clone())
        .unwrap_or_default();
    let canonical = match stop_reason.as_deref() {
        Some("end_turn") => StopReason::EndTurn,
        Some("max_tokens") => StopReason::MaxTokens,
        Some("stop_sequence") => StopReason::StopSequence,
        Some("tool_use") => StopReason::ToolUse,
        Some("pause_turn") => StopReason::Pause,
        Some("refusal") => StopReason::ContentFilter,
        Some(value) => StopReason::Other(value.into()),
        None => StopReason::Other(Box::from("")),
    };
    Finish {
        canonical,
        provider_raw: provider_raw.into(),
    }
}

fn decode_usage(usage: UsageIn) -> Usage {
    let cached = usage.cache_read_input_tokens.unwrap_or(0);
    let cache_creation = usage.cache_creation_input_tokens.unwrap_or(0);
    Usage {
        input: usage
            .input_tokens
            .map(|input| input + cached + cache_creation),
        output: usage.output_tokens,
        cached,
        cache_creation,
        reasoning: 0,
    }
}

fn encode_system(parts: &[Part]) -> Result<Option<TextOrBlocksOut>, Error> {
    match parts {
        [] => Ok(None),
        [Part::Text(text)] => Ok(Some(TextOrBlocksOut::Text(text.clone()))),
        _ => Ok(Some(TextOrBlocksOut::Blocks(
            parts
                .iter()
                .map(encode_system_part)
                .collect::<Result<Vec<_>, _>>()?,
        ))),
    }
}

fn encode_system_part(part: &Part) -> Result<Box<RawValue>, Error> {
    match part {
        Part::Text(text) => raw_from_serializable(&TextBlockOut {
            kind: "text",
            text: text.clone(),
        }),
        Part::Opaque(opaque) => match &opaque.kind {
            OpaqueKind::ProviderSpecific(kind) if *kind == ANTHROPIC_SYSTEM_BLOCK => {
                raw_from_bytes(&opaque.bytes)
            }
            _ => Err(unsupported("messages system opaque")),
        },
        _ => Err(unsupported("messages system part")),
    }
}

fn encode_message(turn: &Turn) -> Result<MessageOut, Error> {
    let role = match turn.role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };
    Ok(MessageOut {
        role,
        content: TextOrBlocksOut::Blocks(encode_blocks(&turn.parts)?),
    })
}

fn encode_blocks(parts: &[Part]) -> Result<Vec<Box<RawValue>>, Error> {
    parts.iter().map(encode_block).collect()
}

fn encode_block(part: &Part) -> Result<Box<RawValue>, Error> {
    match part {
        Part::Text(text) => raw_from_serializable(&TextBlockOut {
            kind: "text",
            text: text.clone(),
        }),
        Part::Image(ImageRef::Base64 { media_type, data }) => {
            raw_from_serializable(&ImageBlockOut {
                kind: "image",
                source: ImageSourceOut {
                    kind: "base64",
                    media_type: Some(media_type.to_string()),
                    data: Some(data.to_string()),
                    url: None,
                },
            })
        }
        Part::Image(ImageRef::Url(url)) => raw_from_serializable(&ImageBlockOut {
            kind: "image",
            source: ImageSourceOut {
                kind: "url",
                media_type: None,
                data: None,
                url: Some(url.to_string()),
            },
        }),
        Part::ToolUse(tool_use) => raw_from_serializable(&ToolUseBlockOut {
            kind: "tool_use",
            id: tool_use.id.0.to_string(),
            name: tool_use.name.to_string(),
            input: raw_value(&tool_use.arguments)?,
        }),
        Part::ToolResult(result) => encode_tool_result(result),
        Part::Thinking(thinking) => raw_from_serializable(&ThinkingBlockOut {
            kind: "thinking",
            thinking: thinking.text.clone(),
            signature: thinking
                .signature
                .as_ref()
                .map(|signature| bytes_to_string(&signature.bytes))
                .transpose()?,
        }),
        Part::Opaque(opaque) => match &opaque.kind {
            OpaqueKind::ProviderSpecific(kind) if *kind == ANTHROPIC_CONTENT_BLOCK => {
                raw_from_bytes(&opaque.bytes)
            }
            OpaqueKind::AnthropicRedactedThinking => {
                raw_from_serializable(&RedactedThinkingBlockOut {
                    kind: "redacted_thinking",
                    data: bytes_to_string(&opaque.bytes)?,
                })
            }
            _ => Err(unsupported("messages opaque")),
        },
    }
}

fn encode_tool_result(result: &ToolResult) -> Result<Box<RawValue>, Error> {
    let content = match &result.content {
        ToolResultContent::Text(text) => TextOrBlocksOut::Text(text.to_string()),
        ToolResultContent::Parts(parts) => TextOrBlocksOut::Blocks(encode_blocks(parts)?),
        ToolResultContent::Object(_) => return Err(unsupported("messages tool result object")),
    };
    raw_from_serializable(&ToolResultBlockOut {
        kind: "tool_result",
        tool_use_id: result.tool_use_id.0.to_string(),
        content,
    })
}

fn encode_tool(tool: &ToolDef) -> Result<ToolOut, Error> {
    Ok(ToolOut {
        name: tool.name.to_string(),
        description: tool.description.as_ref().map(ToString::to_string),
        input_schema: raw_value(&tool.parameters)?,
    })
}

fn encode_tool_choice(tool_choice: &ToolChoice) -> Result<Box<RawValue>, Error> {
    match tool_choice {
        ToolChoice::Auto => raw_from_serializable(&serde_json::json!({ "type": "auto" })),
        ToolChoice::Required => raw_from_serializable(&serde_json::json!({ "type": "any" })),
        ToolChoice::None => raw_from_serializable(&serde_json::json!({ "type": "none" })),
        ToolChoice::Named(name) => {
            raw_from_serializable(&serde_json::json!({ "type": "tool", "name": name }))
        }
        ToolChoice::Other(raw) => raw_value(raw),
    }
}

fn encode_reasoning(reasoning: &Reasoning) -> Option<ThinkingConfigOut> {
    if !reasoning.enabled && reasoning.budget_tokens.is_none() && reasoning.effort.is_none() {
        return None;
    }
    Some(ThinkingConfigOut {
        kind: "enabled",
        budget_tokens: reasoning
            .budget_tokens
            .or_else(|| reasoning.effort.map(reasoning_budget))
            .unwrap_or(DEFAULT_THINKING_BUDGET),
    })
}

fn reasoning_budget(effort: ReasoningEffort) -> u64 {
    match effort {
        ReasoningEffort::Minimal => 1024,
        ReasoningEffort::Low => 2048,
        ReasoningEffort::Medium => 4096,
        ReasoningEffort::High => 8192,
        ReasoningEffort::None => 1024,
    }
}

fn encode_stop(stop: &[Box<str>]) -> Option<Vec<String>> {
    (!stop.is_empty()).then(|| stop.iter().map(ToString::to_string).collect())
}

fn stop_reason_name(finish: &Finish) -> String {
    match &finish.canonical {
        StopReason::EndTurn => "end_turn".to_owned(),
        StopReason::MaxTokens => "max_tokens".to_owned(),
        StopReason::StopSequence => "stop_sequence".to_owned(),
        StopReason::ToolUse => "tool_use".to_owned(),
        StopReason::ContentFilter => "refusal".to_owned(),
        StopReason::Pause => "pause_turn".to_owned(),
        StopReason::Cancelled => "end_turn".to_owned(),
        StopReason::Other(value) if value.is_empty() => "end_turn".to_owned(),
        StopReason::Other(value) => value.to_string(),
    }
}

fn encode_usage(usage: Usage) -> UsageOut {
    UsageOut {
        input_tokens: usage.input.map(|input| {
            input
                .saturating_sub(usage.cached)
                .saturating_sub(usage.cache_creation)
        }),
        output_tokens: usage.output,
        cache_read_input_tokens: (usage.cached > 0).then_some(usage.cached),
        cache_creation_input_tokens: (usage.cache_creation > 0).then_some(usage.cache_creation),
    }
}

fn opaque_block(kind: &'static str, raw: &RawValue) -> Part {
    Part::Opaque(Opaque {
        kind: OpaqueKind::ProviderSpecific(kind),
        bytes: raw.get().as_bytes().to_vec().into_boxed_slice(),
    })
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
