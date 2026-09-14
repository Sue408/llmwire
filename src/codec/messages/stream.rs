use serde_json::json;

use crate::codec::StreamDecode;
use crate::framing::SseFrame;
use crate::ids::OpaqueKind;
use crate::ir::{
    Delta, Event, Opaque, PartKind, StopReason, StreamState, ToolId, ToolStart, ToolUseKind, Usage,
    UsagePatch,
};
use crate::Error;

use super::wire::*;
use super::{decode_finish, decode_usage, encode_usage, parse_str, stop_reason_name};

pub(super) fn decode_stream_frame(
    frame: &SseFrame,
    _state: &StreamState,
) -> Result<StreamDecode, Error> {
    let Some(kind) = frame.event.as_deref() else {
        return Ok(StreamDecode::default());
    };
    let data = frame.data.as_deref().unwrap_or("{}");

    let events = match kind {
        "message_start" => {
            let input: MessageStartIn = parse_str(data)?;
            let mut events = vec![Event::MessageStart {
                id: input.message.id.into_boxed_str(),
                model: input.message.model.unwrap_or_default().into_boxed_str(),
            }];
            if let Some(usage) = input.message.usage {
                events.push(Event::UsagePatch(usage_patch(decode_usage(usage))));
            }
            events
        }
        "content_block_start" => {
            let input: ContentBlockStartIn = parse_str(data)?;
            let index = input.index;
            let mut events = Vec::new();

            match input.content_block {
                ContentBlockStartBodyIn::Text { text } => {
                    events.push(Event::PartStart {
                        index,
                        kind: PartKind::Text,
                        tool: None,
                    });
                    if !text.is_empty() {
                        events.push(Event::PartDelta {
                            index,
                            delta: Delta::Text(text.into_boxed_str()),
                        });
                    }
                }
                ContentBlockStartBodyIn::ToolUse { id, name, input } => {
                    let kind = if id.starts_with("srvtoolu_") {
                        ToolUseKind::Server
                    } else {
                        ToolUseKind::Client
                    };
                    events.push(Event::PartStart {
                        index,
                        kind: PartKind::ToolUse,
                        tool: Some(ToolStart {
                            source_index: Some(index as u32),
                            id: ToolId(id.into_boxed_str()),
                            name: name.into_boxed_str(),
                            kind,
                        }),
                    });
                    if let Some(input) = input {
                        let raw = input.to_string();
                        if raw != "{}" {
                            events.push(Event::PartDelta {
                                index,
                                delta: Delta::ToolArguments(raw.into_boxed_str()),
                            });
                        }
                    }
                }
                ContentBlockStartBodyIn::Thinking {
                    thinking,
                    signature,
                } => {
                    events.push(Event::PartStart {
                        index,
                        kind: PartKind::Thinking,
                        tool: None,
                    });
                    if !thinking.is_empty() {
                        events.push(Event::PartDelta {
                            index,
                            delta: Delta::Thinking(thinking.into_boxed_str()),
                        });
                    }
                    if let Some(signature) = signature {
                        events.push(Event::PartDelta {
                            index,
                            delta: Delta::Opaque(Opaque {
                                kind: OpaqueKind::AnthropicThinkingSignature,
                                bytes: signature.into_bytes().into_boxed_slice(),
                            }),
                        });
                    }
                }
                ContentBlockStartBodyIn::RedactedThinking { data } => {
                    events.push(Event::PartStart {
                        index,
                        kind: PartKind::Opaque(OpaqueKind::AnthropicRedactedThinking),
                        tool: None,
                    });
                    if !data.is_empty() {
                        events.push(Event::PartDelta {
                            index,
                            delta: Delta::Opaque(Opaque {
                                kind: OpaqueKind::AnthropicRedactedThinking,
                                bytes: data.into_bytes().into_boxed_slice(),
                            }),
                        });
                    }
                }
                ContentBlockStartBodyIn::Unknown => {}
            }

            events
        }
        "content_block_delta" => {
            let input: ContentBlockDeltaIn = parse_str(data)?;
            let index = input.index;
            match input.delta {
                ContentBlockDeltaBodyIn::TextDelta { text } => vec![Event::PartDelta {
                    index,
                    delta: Delta::Text(text.into_boxed_str()),
                }],
                ContentBlockDeltaBodyIn::ThinkingDelta { thinking } => vec![Event::PartDelta {
                    index,
                    delta: Delta::Thinking(thinking.into_boxed_str()),
                }],
                ContentBlockDeltaBodyIn::InputJsonDelta { partial_json } => {
                    vec![Event::PartDelta {
                        index,
                        delta: Delta::ToolArguments(partial_json.into_boxed_str()),
                    }]
                }
                ContentBlockDeltaBodyIn::SignatureDelta { signature } => vec![Event::PartDelta {
                    index,
                    delta: Delta::Opaque(Opaque {
                        kind: OpaqueKind::AnthropicThinkingSignature,
                        bytes: signature.into_bytes().into_boxed_slice(),
                    }),
                }],
                ContentBlockDeltaBodyIn::Unknown => Vec::new(),
            }
        }
        "content_block_stop" => {
            let input: ContentBlockStopIn = parse_str(data)?;
            vec![Event::PartStop { index: input.index }]
        }
        "message_delta" => {
            let input: MessageDeltaIn = parse_str(data)?;
            let mut events = Vec::new();
            if let Some(usage) = input.usage {
                events.push(Event::UsagePatch(usage_patch(decode_usage(usage))));
            }
            if input.delta.stop_reason.is_some() || input.delta.stop_sequence.is_some() {
                events.push(Event::Finish(decode_finish(
                    input.delta.stop_reason,
                    input.delta.stop_sequence,
                )));
            }
            events
        }
        "message_stop" => {
            return Ok(StreamDecode {
                events: Vec::new(),
                termination: Some(crate::ir::Termination::Explicit),
            });
        }
        "error" => {
            let message = serde_json::from_str::<serde_json::Value>(data)
                .ok()
                .and_then(|value| {
                    value
                        .get("error")
                        .and_then(|error| error.get("message"))
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| data.to_owned());
            vec![Event::Error(Box::new(Error::Protocol(message)))]
        }
        "ping" => Vec::new(),
        _ => Vec::new(),
    };

    Ok(StreamDecode {
        events,
        termination: None,
    })
}

pub(super) fn encode_stream_event(
    event: &Event,
    state: &StreamState,
) -> Result<Vec<SseFrame>, Error> {
    match event {
        Event::MessageStart { id, model } => {
            let input_unknown = state.usage().input.is_none();
            let mut usage = state.usage();
            if input_unknown {
                usage.input = Some(0);
            }
            let mut usage_value = json!(encode_usage(usage));
            if input_unknown {
                usage_value["estimated"] = json!(true);
            }
            Ok(vec![json_frame(
                "message_start",
                json!({
                    "type": "message_start",
                    "message": {
                        "id": id,
                        "type": "message",
                        "role": "assistant",
                        "model": model,
                        "content": [],
                        "stop_reason": null,
                        "stop_sequence": null,
                        "usage": usage_value,
                    }
                }),
            )?])
        }
        Event::PartStart {
            index,
            kind: PartKind::Text,
            ..
        } => Ok(vec![json_frame(
            "content_block_start",
            json!({
                "type": "content_block_start",
                "index": index,
                "content_block": {"type": "text", "text": ""},
            }),
        )?]),
        Event::PartStart {
            index,
            kind: PartKind::Thinking,
            ..
        } => Ok(vec![json_frame(
            "content_block_start",
            json!({
                "type": "content_block_start",
                "index": index,
                "content_block": {"type": "thinking", "thinking": ""},
            }),
        )?]),
        Event::PartStart {
            index,
            kind: PartKind::Opaque(OpaqueKind::AnthropicRedactedThinking),
            ..
        } => Ok(vec![json_frame(
            "content_block_start",
            json!({
                "type": "content_block_start",
                "index": index,
                "content_block": {"type": "redacted_thinking", "data": ""},
            }),
        )?]),
        Event::PartStart {
            index,
            kind: PartKind::ToolUse,
            tool: Some(tool),
        } => Ok(vec![json_frame(
            "content_block_start",
            json!({
                "type": "content_block_start",
                "index": index,
                "content_block": {
                    "type": "tool_use",
                    "id": tool.id.0,
                    "name": tool.name,
                    "input": {},
                },
            }),
        )?]),
        Event::PartStart {
            kind: PartKind::Opaque(kind),
            ..
        } => Err(Error::Unsupported(format!(
            "messages streaming cannot encode opaque kind {kind:?}"
        ))),
        Event::PartStart {
            kind: PartKind::ToolUse,
            ..
        } => Err(Error::Protocol(
            "tool use stream event is missing tool metadata".to_owned(),
        )),
        Event::PartStart {
            kind: PartKind::ToolResult,
            ..
        } => Err(Error::Unsupported(
            "messages streaming cannot encode tool_result output".to_owned(),
        )),
        Event::PartDelta { index, delta } => encode_delta(*index, delta),
        Event::PartStop { index } => Ok(vec![json_frame(
            "content_block_stop",
            json!({"type": "content_block_stop", "index": index}),
        )?]),
        Event::UsagePatch(patch) => Ok(vec![json_frame(
            "message_delta",
            json!({
                "type": "message_delta",
                "delta": {},
                "usage": usage_patch_out(*patch),
            }),
        )?]),
        Event::Finish(finish) => {
            let stop_reason = stop_reason_name(finish);
            let stop_sequence = matches!(finish.canonical, StopReason::StopSequence)
                .then(|| finish.provider_raw.to_string());
            Ok(vec![
                json_frame(
                    "message_delta",
                    json!({
                        "type": "message_delta",
                        "delta": {
                            "stop_reason": stop_reason,
                            "stop_sequence": stop_sequence,
                        },
                        "usage": encode_usage(state.usage()),
                    }),
                )?,
                json_frame("message_stop", json!({"type": "message_stop"}))?,
            ])
        }
        Event::Error(error) => Ok(vec![json_frame(
            "error",
            json!({
                "type": "error",
                "error": {
                    "type": "api_error",
                    "message": error.to_string(),
                }
            }),
        )?]),
    }
}

fn encode_delta(index: usize, delta: &Delta) -> Result<Vec<SseFrame>, Error> {
    let value = match delta {
        Delta::Text(text) => json!({
            "type": "content_block_delta",
            "index": index,
            "delta": {"type": "text_delta", "text": text},
        }),
        Delta::Thinking(thinking) => json!({
            "type": "content_block_delta",
            "index": index,
            "delta": {"type": "thinking_delta", "thinking": thinking},
        }),
        Delta::ToolArguments(partial_json) => json!({
            "type": "content_block_delta",
            "index": index,
            "delta": {"type": "input_json_delta", "partial_json": partial_json},
        }),
        Delta::Opaque(opaque) if opaque.kind == OpaqueKind::AnthropicThinkingSignature => {
            let signature = std::str::from_utf8(&opaque.bytes).map_err(|error| {
                Error::InvalidInput(format!("thinking signature is not utf-8: {error}"))
            })?;
            json!({
                "type": "content_block_delta",
                "index": index,
                "delta": {"type": "signature_delta", "signature": signature},
            })
        }
        Delta::Opaque(_) => return Ok(Vec::new()),
    };

    Ok(vec![json_frame("content_block_delta", value)?])
}

fn usage_patch(usage: Usage) -> UsagePatch {
    UsagePatch {
        input: usage.input,
        output: usage.output,
        cached: Some(usage.cached),
        cache_creation: Some(usage.cache_creation),
        reasoning: Some(usage.reasoning),
    }
}

fn usage_patch_out(patch: UsagePatch) -> serde_json::Value {
    json!({
        "input_tokens": patch.input,
        "output_tokens": patch.output,
        "cache_read_input_tokens": patch.cached,
        "cache_creation_input_tokens": patch.cache_creation,
    })
}

fn json_frame(event: &str, value: serde_json::Value) -> Result<SseFrame, Error> {
    Ok(SseFrame {
        event: Some(event.to_owned()),
        data: Some(serde_json::to_string(&value).map_err(|error| {
            Error::Protocol(format!(
                "failed to serialize messages stream frame: {error}"
            ))
        })?),
    })
}
