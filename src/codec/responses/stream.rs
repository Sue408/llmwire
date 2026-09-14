use serde_json::{json, Value};

use crate::codec::{ProtocolCodec, StreamDecode};
use crate::framing::SseFrame;
use crate::ids::OpaqueKind;
use crate::ir::{
    Delta, Event, Opaque, Part, PartKind, StreamState, Termination, ToolId, ToolStart, ToolUseKind,
    Usage, UsagePatch,
};
use crate::Error;

use super::Responses;

const RESPONSES_ITEM: &str = "responses_item";
const INDEX_STRIDE: usize = 4096;

pub(super) fn decode_stream_frame(
    frame: &SseFrame,
    state: &StreamState,
) -> Result<StreamDecode, Error> {
    let Some(data) = frame.data.as_deref() else {
        return Ok(StreamDecode::default());
    };
    if data.trim() == "[DONE]" {
        return Err(Error::Protocol(
            "responses streams do not use data: [DONE]".to_owned(),
        ));
    }

    let value: Value = serde_json::from_str(data)
        .map_err(|error| Error::InvalidInput(format!("invalid responses event: {error}")))?;
    let kind = frame
        .event
        .as_deref()
        .or_else(|| value.get("type").and_then(Value::as_str))
        .ok_or_else(|| Error::InvalidInput("responses event missing type".to_owned()))?;

    let events = match kind {
        "response.created" => {
            let response = value.get("response").ok_or_else(|| {
                Error::InvalidInput("response.created missing response".to_owned())
            })?;
            vec![Event::MessageStart {
                id: string_field(response, "id")?.into(),
                model: string_field(response, "model").unwrap_or("llmwire").into(),
            }]
        }
        "response.in_progress" => {
            if state.has_message_start() {
                Vec::new()
            } else if let Some(response) = value.get("response") {
                vec![Event::MessageStart {
                    id: string_field(response, "id")?.into(),
                    model: string_field(response, "model").unwrap_or("llmwire").into(),
                }]
            } else {
                Vec::new()
            }
        }
        "response.output_item.added" => decode_output_item_added(&value, state)?,
        "response.output_item.done" => decode_output_item_done(&value, state)?,
        "response.content_part.added" => decode_content_part_added(&value, state)?,
        "response.content_part.done" => {
            let index = part_index(&value, "content_index")?;
            if state.is_open(index) {
                vec![Event::PartStop { index }]
            } else {
                Vec::new()
            }
        }
        "response.output_text.delta" | "response.refusal.delta" => {
            let index = part_index(&value, "content_index")?;
            let text = string_field(&value, "delta")?;
            vec![Event::PartDelta {
                index,
                delta: Delta::Text(text.into()),
            }]
        }
        "response.output_text.done" | "response.refusal.done" => {
            let index = part_index(&value, "content_index")?;
            if state.is_open(index) {
                vec![Event::PartStop { index }]
            } else {
                Vec::new()
            }
        }
        "response.reasoning_summary_part.added" => {
            let index = summary_index(&value)?;
            if state.part_index_used(index) {
                Vec::new()
            } else {
                vec![Event::PartStart {
                    index,
                    kind: PartKind::Thinking,
                    tool: None,
                }]
            }
        }
        "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
            let index = summary_index(&value)?;
            let text = string_field(&value, "delta")?;
            vec![Event::PartDelta {
                index,
                delta: Delta::Thinking(text.into()),
            }]
        }
        "response.reasoning_summary_text.done" | "response.reasoning_text.done" => {
            let index = summary_index(&value)?;
            if state.is_open(index) {
                vec![Event::PartStop { index }]
            } else {
                Vec::new()
            }
        }
        "response.reasoning_summary_part.done" => {
            let index = summary_index(&value)?;
            if state.is_open(index) {
                vec![Event::PartStop { index }]
            } else {
                Vec::new()
            }
        }
        "response.function_call_arguments.delta" => {
            let output_index = u32_field(&value, "output_index")?;
            let index = state.tool_part_index(output_index).ok_or_else(|| {
                Error::Protocol("function_call delta before item start".to_owned())
            })?;
            let arguments = string_field(&value, "delta")?;
            vec![Event::PartDelta {
                index,
                delta: Delta::ToolArguments(arguments.into()),
            }]
        }
        "response.function_call_arguments.done" => {
            let output_index = u32_field(&value, "output_index")?;
            let index = state.tool_part_index(output_index).ok_or_else(|| {
                Error::Protocol("function_call done before item start".to_owned())
            })?;
            let arguments = string_field(&value, "arguments")?;
            let mut events = Vec::new();
            if state.tool_arguments(index).unwrap_or("").is_empty() && !arguments.is_empty() {
                events.push(Event::PartDelta {
                    index,
                    delta: Delta::ToolArguments(arguments.into()),
                });
            }
            if state.is_open(index) {
                events.push(Event::PartStop { index });
            }
            events
        }
        "response.completed" | "response.incomplete" => return decode_terminal(&value),
        "response.failed" => {
            let error = value
                .get("response")
                .and_then(|response| response.get("error"))
                .or_else(|| value.get("error"));
            let message = error
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("responses stream failed");
            return Ok(StreamDecode {
                events: vec![Event::Error(Box::new(Error::Protocol(message.to_owned())))],
                termination: Some(Termination::Explicit),
            });
        }
        "error" => {
            let message = value
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .or_else(|| value.get("message").and_then(Value::as_str))
                .unwrap_or("responses stream error");
            return Ok(StreamDecode {
                events: vec![Event::Error(Box::new(Error::Protocol(message.to_owned())))],
                termination: None,
            });
        }
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
        Event::MessageStart { id, model } => Ok(vec![json_frame(
            "response.created",
            json!({
                "type": "response.created",
                "response": {
                    "id": id,
                    "object": "response",
                    "status": "in_progress",
                    "model": model,
                    "output": [],
                }
            }),
        )?]),
        Event::PartStart { index, kind, tool } => {
            encode_part_start(*index, *kind, tool.as_ref(), state)
        }
        Event::PartDelta { index, delta } => encode_part_delta(*index, delta, state),
        Event::PartStop { index } => encode_part_stop(*index, state),
        Event::UsagePatch(_) => Ok(Vec::new()),
        Event::Finish(_) => encode_terminal(state),
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

fn decode_output_item_added(value: &Value, state: &StreamState) -> Result<Vec<Event>, Error> {
    let output_index = u32_field(value, "output_index")?;
    let item = value
        .get("item")
        .ok_or_else(|| Error::InvalidInput("output_item.added missing item".to_owned()))?;
    let item_type = item
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::InvalidInput("output item missing type".to_owned()))?;

    match item_type {
        "message" => {
            let index = response_index(output_index, 0);
            if state.part_index_used(index) {
                Ok(Vec::new())
            } else {
                Ok(vec![Event::PartStart {
                    index,
                    kind: PartKind::Text,
                    tool: None,
                }])
            }
        }
        "function_call" => {
            let index = response_index(output_index, 0);
            if state.part_index_used(index) {
                return Ok(Vec::new());
            }
            let call_id = string_field(item, "call_id")?;
            let name = string_field(item, "name")?;
            Ok(vec![Event::PartStart {
                index,
                kind: PartKind::ToolUse,
                tool: Some(ToolStart {
                    source_index: Some(output_index),
                    id: ToolId(call_id.into()),
                    name: name.into(),
                    kind: ToolUseKind::Client,
                }),
            }])
        }
        "reasoning" => {
            let index = response_index(output_index, 0);
            if state.part_index_used(index) {
                return Ok(Vec::new());
            }
            let encrypted = item.get("encrypted_content").and_then(Value::as_str);
            let summary_empty = item
                .get("summary")
                .and_then(Value::as_array)
                .map(Vec::is_empty)
                .unwrap_or(true);
            if let Some(encrypted) = encrypted.filter(|_| summary_empty) {
                Ok(vec![
                    Event::PartStart {
                        index,
                        kind: PartKind::Opaque(OpaqueKind::ResponsesEncryptedReasoning),
                        tool: None,
                    },
                    Event::PartDelta {
                        index,
                        delta: Delta::Opaque(Opaque {
                            kind: OpaqueKind::ResponsesEncryptedReasoning,
                            bytes: encrypted.as_bytes().to_vec().into_boxed_slice(),
                        }),
                    },
                ])
            } else {
                Ok(vec![Event::PartStart {
                    index,
                    kind: PartKind::Thinking,
                    tool: None,
                }])
            }
        }
        _ => {
            let index = response_index(output_index, 0);
            if state.part_index_used(index) {
                return Ok(Vec::new());
            }
            let bytes = item.to_string().into_bytes().into_boxed_slice();
            Ok(vec![
                Event::PartStart {
                    index,
                    kind: PartKind::Opaque(OpaqueKind::ProviderSpecific(RESPONSES_ITEM)),
                    tool: None,
                },
                Event::PartDelta {
                    index,
                    delta: Delta::Opaque(Opaque {
                        kind: OpaqueKind::ProviderSpecific(RESPONSES_ITEM),
                        bytes,
                    }),
                },
            ])
        }
    }
}

fn decode_output_item_done(value: &Value, state: &StreamState) -> Result<Vec<Event>, Error> {
    let output_index = u32_field(value, "output_index")?;
    let index = response_index(output_index, 0);
    let mut events = Vec::new();
    if state.is_open(index) {
        events.push(Event::PartStop { index });
    }

    let item = value
        .get("item")
        .ok_or_else(|| Error::InvalidInput("output_item.done missing item".to_owned()))?;
    let item_type = item
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");

    match item_type {
        "reasoning" => {
            if let Some(encrypted) = item.get("encrypted_content").and_then(Value::as_str) {
                let opaque_index = state.next_part_index().max(index + 1);
                events.extend([
                    Event::PartStart {
                        index: opaque_index,
                        kind: PartKind::Opaque(OpaqueKind::ResponsesEncryptedReasoning),
                        tool: None,
                    },
                    Event::PartDelta {
                        index: opaque_index,
                        delta: Delta::Opaque(Opaque {
                            kind: OpaqueKind::ResponsesEncryptedReasoning,
                            bytes: encrypted.as_bytes().to_vec().into_boxed_slice(),
                        }),
                    },
                    Event::PartStop {
                        index: opaque_index,
                    },
                ]);
            } else if !state.part_index_used(index) {
                let text = reasoning_text(item);
                if !text.is_empty() {
                    events.extend([
                        Event::PartStart {
                            index,
                            kind: PartKind::Thinking,
                            tool: None,
                        },
                        Event::PartDelta {
                            index,
                            delta: Delta::Thinking(text.into()),
                        },
                        Event::PartStop { index },
                    ]);
                }
            }
        }
        "function_call" if !state.part_index_used(index) => {
            let call_id = string_field(item, "call_id")?;
            let name = string_field(item, "name")?;
            let arguments = item
                .get("arguments")
                .and_then(Value::as_str)
                .unwrap_or_default();
            events.extend([
                Event::PartStart {
                    index,
                    kind: PartKind::ToolUse,
                    tool: Some(ToolStart {
                        source_index: Some(output_index),
                        id: ToolId(call_id.into()),
                        name: name.into(),
                        kind: ToolUseKind::Client,
                    }),
                },
                Event::PartDelta {
                    index,
                    delta: Delta::ToolArguments(arguments.into()),
                },
                Event::PartStop { index },
            ]);
        }
        "message" if !state.part_index_used(index) => {
            let text = message_text(item);
            if !text.is_empty() {
                events.extend([
                    Event::PartStart {
                        index,
                        kind: PartKind::Text,
                        tool: None,
                    },
                    Event::PartDelta {
                        index,
                        delta: Delta::Text(text.into()),
                    },
                    Event::PartStop { index },
                ]);
            }
        }
        _ if !state.part_index_used(index) => {
            let bytes = item.to_string().into_bytes().into_boxed_slice();
            events.extend([
                Event::PartStart {
                    index,
                    kind: PartKind::Opaque(OpaqueKind::ProviderSpecific(RESPONSES_ITEM)),
                    tool: None,
                },
                Event::PartDelta {
                    index,
                    delta: Delta::Opaque(Opaque {
                        kind: OpaqueKind::ProviderSpecific(RESPONSES_ITEM),
                        bytes,
                    }),
                },
                Event::PartStop { index },
            ]);
        }
        _ => {}
    }

    Ok(events)
}

fn message_text(item: &Value) -> String {
    item.get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("")
}

fn reasoning_text(item: &Value) -> String {
    item.get("summary")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("")
}

fn decode_content_part_added(value: &Value, state: &StreamState) -> Result<Vec<Event>, Error> {
    let index = part_index(value, "content_index")?;
    if state.part_index_used(index) {
        return Ok(Vec::new());
    }
    let part = value
        .get("part")
        .ok_or_else(|| Error::InvalidInput("content_part.added missing part".to_owned()))?;
    let kind = match part.get("type").and_then(Value::as_str) {
        Some("refusal") => PartKind::Text,
        _ => PartKind::Text,
    };
    Ok(vec![Event::PartStart {
        index,
        kind,
        tool: None,
    }])
}

fn decode_terminal(value: &Value) -> Result<StreamDecode, Error> {
    let response = value.get("response").ok_or_else(|| {
        Error::InvalidInput("terminal response event missing response".to_owned())
    })?;
    let encoded = serde_json::to_vec(response)
        .map_err(|error| Error::Protocol(format!("failed to read responses snapshot: {error}")))?;
    let output = Responses.decode_response(&encoded)?;
    let usage = output.usage;
    let finish = output
        .choices
        .first()
        .map(|choice| choice.finish.clone())
        .unwrap_or_default();
    Ok(StreamDecode {
        events: vec![Event::UsagePatch(usage_patch(usage)), Event::Finish(finish)],
        termination: Some(Termination::Explicit),
    })
}

fn encode_part_start(
    index: usize,
    kind: PartKind,
    tool: Option<&ToolStart>,
    state: &StreamState,
) -> Result<Vec<SseFrame>, Error> {
    let output_index = output_index_for(state, index);
    match kind {
        PartKind::Text => Ok(vec![
            json_frame(
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": {
                        "type": "message",
                        "id": format!("msg_{output_index}"),
                        "role": "assistant",
                        "status": "in_progress",
                        "content": [],
                    }
                }),
            )?,
            json_frame(
                "response.content_part.added",
                json!({
                    "type": "response.content_part.added",
                    "output_index": output_index,
                    "content_index": 0,
                    "part": {"type": "output_text", "text": "", "annotations": []},
                }),
            )?,
        ]),
        PartKind::Thinking => Ok(vec![
            json_frame(
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": {
                        "type": "reasoning",
                        "id": format!("rs_{output_index}"),
                        "summary": [],
                        "status": "in_progress",
                    }
                }),
            )?,
            json_frame(
                "response.reasoning_summary_part.added",
                json!({
                    "type": "response.reasoning_summary_part.added",
                    "output_index": output_index,
                    "summary_index": 0,
                }),
            )?,
        ]),
        PartKind::ToolUse => {
            let tool = tool.ok_or_else(|| {
                Error::Protocol("responses tool start is missing metadata".to_owned())
            })?;
            Ok(vec![json_frame(
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": {
                        "type": "function_call",
                        "id": format!("fc_{output_index}"),
                        "call_id": tool.id.0,
                        "name": tool.name,
                        "arguments": "",
                        "status": "in_progress",
                    }
                }),
            )?])
        }
        PartKind::Opaque(OpaqueKind::ResponsesEncryptedReasoning) => {
            let encrypted = state
                .part(index)
                .and_then(|part| match part {
                    Part::Opaque(opaque) => Some(bytes_to_string(&opaque.bytes)),
                    _ => None,
                })
                .transpose()?
                .unwrap_or_default();
            Ok(vec![json_frame(
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": {
                        "type": "reasoning",
                        "id": format!("rs_{output_index}"),
                        "summary": [],
                        "encrypted_content": encrypted,
                        "status": "in_progress",
                    }
                }),
            )?])
        }
        PartKind::Opaque(_) => Ok(vec![json_frame(
            "response.output_item.added",
            json!({
                "type": "response.output_item.added",
                "output_index": output_index,
                "item": {"type": "unknown", "id": format!("opaque_{output_index}")},
            }),
        )?]),
        PartKind::ToolResult => Err(Error::Unsupported(
            "responses stream cannot encode tool_result output".to_owned(),
        )),
    }
}

fn encode_part_delta(
    index: usize,
    delta: &Delta,
    state: &StreamState,
) -> Result<Vec<SseFrame>, Error> {
    let output_index = output_index_for(state, index);
    match delta {
        Delta::Text(text) => Ok(vec![json_frame(
            "response.output_text.delta",
            json!({
                "type": "response.output_text.delta",
                "output_index": output_index,
                "content_index": 0,
                "delta": text,
            }),
        )?]),
        Delta::Thinking(text) => Ok(vec![json_frame(
            "response.reasoning_summary_text.delta",
            json!({
                "type": "response.reasoning_summary_text.delta",
                "output_index": output_index,
                "summary_index": 0,
                "delta": text,
            }),
        )?]),
        Delta::ToolArguments(arguments) => {
            let call_id = state
                .tool_start(index)
                .map(|tool| tool.id.0.to_string())
                .unwrap_or_else(|| format!("call_{output_index}"));
            Ok(vec![json_frame(
                "response.function_call_arguments.delta",
                json!({
                    "type": "response.function_call_arguments.delta",
                    "output_index": output_index,
                    "item_id": format!("fc_{output_index}"),
                    "call_id": call_id,
                    "delta": arguments,
                }),
            )?])
        }
        Delta::Opaque(_) => Ok(Vec::new()),
    }
}

fn encode_part_stop(index: usize, state: &StreamState) -> Result<Vec<SseFrame>, Error> {
    let output_index = output_index_for(state, index);
    match state.part(index) {
        Some(Part::Text(text)) => Ok(vec![
            json_frame(
                "response.output_text.done",
                json!({
                    "type": "response.output_text.done",
                    "output_index": output_index,
                    "content_index": 0,
                    "text": text,
                }),
            )?,
            json_frame(
                "response.content_part.done",
                json!({
                    "type": "response.content_part.done",
                    "output_index": output_index,
                    "content_index": 0,
                    "part": {"type": "output_text", "text": text, "annotations": []},
                }),
            )?,
            json_frame(
                "response.output_item.done",
                json!({
                    "type": "response.output_item.done",
                    "output_index": output_index,
                    "item": {
                        "type": "message",
                        "id": format!("msg_{output_index}"),
                        "role": "assistant",
                        "status": "completed",
                        "content": [{"type": "output_text", "text": text, "annotations": []}],
                    }
                }),
            )?,
        ]),
        Some(Part::ToolUse(tool)) => Ok(vec![
            json_frame(
                "response.function_call_arguments.done",
                json!({
                    "type": "response.function_call_arguments.done",
                    "output_index": output_index,
                    "item_id": format!("fc_{output_index}"),
                    "call_id": tool.id.0,
                    "arguments": tool.arguments.raw(),
                }),
            )?,
            json_frame(
                "response.output_item.done",
                json!({
                    "type": "response.output_item.done",
                    "output_index": output_index,
                    "item": {
                        "type": "function_call",
                        "id": format!("fc_{output_index}"),
                        "call_id": tool.id.0,
                        "name": tool.name,
                        "arguments": tool.arguments.raw(),
                        "status": "completed",
                    }
                }),
            )?,
        ]),
        Some(Part::Thinking(thinking)) => Ok(vec![
            json_frame(
                "response.reasoning_summary_text.done",
                json!({
                    "type": "response.reasoning_summary_text.done",
                    "output_index": output_index,
                    "summary_index": 0,
                    "text": thinking.text,
                }),
            )?,
            json_frame(
                "response.reasoning_summary_part.done",
                json!({
                    "type": "response.reasoning_summary_part.done",
                    "output_index": output_index,
                    "summary_index": 0,
                }),
            )?,
            json_frame(
                "response.output_item.done",
                json!({
                    "type": "response.output_item.done",
                    "output_index": output_index,
                    "item": {
                        "type": "reasoning",
                        "id": format!("rs_{output_index}"),
                        "summary": [{"type": "summary_text", "text": thinking.text}],
                        "status": "completed",
                    }
                }),
            )?,
        ]),
        Some(Part::Opaque(opaque)) => {
            let item = opaque_item(opaque, output_index)?;
            Ok(vec![json_frame(
                "response.output_item.done",
                json!({
                    "type": "response.output_item.done",
                    "output_index": output_index,
                    "item": item,
                }),
            )?])
        }
        Some(Part::ToolResult(_)) | Some(Part::Image(_)) => Err(Error::Unsupported(
            "responses stream cannot encode response part".to_owned(),
        )),
        None => Ok(vec![json_frame(
            "response.output_item.done",
            json!({
                "type": "response.output_item.done",
                "output_index": output_index,
                "item": {"type": "unknown", "id": format!("opaque_{output_index}")},
            }),
        )?]),
    }
}

fn encode_terminal(state: &StreamState) -> Result<Vec<SseFrame>, Error> {
    let output = state.assistant_output()?;
    let encoded = Responses.encode_response(&output)?;
    let mut response: Value = serde_json::from_slice(&encoded).map_err(|error| {
        Error::Protocol(format!("failed to encode responses snapshot: {error}"))
    })?;
    if let Some((id, model)) = state.message() {
        response["id"] = Value::String(id.to_owned());
        response["model"] = Value::String(model.to_owned());
    }
    let status = response
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("completed");
    let event = if status == "completed" {
        "response.completed"
    } else {
        "response.incomplete"
    };
    Ok(vec![json_frame(
        event,
        json!({
            "type": event,
            "response": response,
        }),
    )?])
}

fn opaque_item(opaque: &Opaque, output_index: u32) -> Result<Value, Error> {
    match opaque.kind {
        OpaqueKind::ResponsesEncryptedReasoning => Ok(json!({
            "type": "reasoning",
            "id": format!("rs_{output_index}"),
            "summary": [],
            "encrypted_content": bytes_to_string(&opaque.bytes)?,
            "status": "completed",
        })),
        OpaqueKind::ProviderSpecific(kind) if kind == RESPONSES_ITEM => {
            serde_json::from_slice(&opaque.bytes).map_err(|error| {
                Error::InvalidInput(format!("responses opaque item is not valid JSON: {error}"))
            })
        }
        _ => Err(Error::Unsupported(
            "responses stream cannot encode opaque kind".to_owned(),
        )),
    }
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

fn part_index(value: &Value, field: &str) -> Result<usize, Error> {
    Ok(response_index(
        u32_field(value, "output_index")?,
        u32_field(value, field)?,
    ))
}

fn summary_index(value: &Value) -> Result<usize, Error> {
    let sub_index = value
        .get("summary_index")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    Ok(response_index(u32_field(value, "output_index")?, sub_index))
}

fn response_index(output_index: u32, sub_index: u32) -> usize {
    (output_index as usize + 1) * INDEX_STRIDE + sub_index as usize
}

fn output_index_for(state: &StreamState, index: usize) -> u32 {
    let mut indices = state.part_indices();
    indices.sort_unstable();
    indices
        .iter()
        .position(|candidate| *candidate == index)
        .unwrap_or(index) as u32
}

fn u32_field(value: &Value, field: &str) -> Result<u32, Error> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|number| u32::try_from(number).ok())
        .ok_or_else(|| Error::InvalidInput(format!("responses event missing {field}")))
}

fn string_field<'a>(value: &'a Value, field: &str) -> Result<&'a str, Error> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::InvalidInput(format!("responses event missing {field}")))
}

fn bytes_to_string(bytes: &[u8]) -> Result<String, Error> {
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|error| Error::InvalidInput(error.to_string()))
}

fn json_frame(event: &str, mut value: Value) -> Result<SseFrame, Error> {
    if value.get("type").is_none() {
        value["type"] = Value::String(event.to_owned());
    }
    Ok(SseFrame {
        event: Some(event.to_owned()),
        data: Some(serde_json::to_string(&value).map_err(|error| {
            Error::Protocol(format!(
                "failed to serialize responses stream frame: {error}"
            ))
        })?),
    })
}
