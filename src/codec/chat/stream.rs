use std::collections::BTreeMap;

use crate::codec::StreamDecode;
use crate::framing::SseFrame;
use crate::ir::{Delta, Event, PartKind, StreamState, ToolId, ToolStart, ToolUseKind, UsagePatch};
use crate::Error;

use super::wire::*;
use super::{decode_finish, decode_usage, parse_str, stop_reason_name};

pub(super) fn decode_stream_frame(
    frame: &SseFrame,
    state: &StreamState,
) -> Result<StreamDecode, Error> {
    let Some(data) = frame.data.as_deref() else {
        return Ok(StreamDecode::default());
    };

    if data.trim() == "[DONE]" {
        return Ok(StreamDecode {
            events: Vec::new(),
            termination: Some(crate::ir::Termination::Explicit),
        });
    }

    let chunk: ChatChunkIn = parse_str(data)?;
    let mut events = Vec::new();
    let mut next_index = state.next_part_index();
    let mut open_indices = state.open_indices();
    let mut used_indices = state.part_indices();
    let mut tool_indices = BTreeMap::new();

    if !state.has_message_start() && !chunk.choices.is_empty() {
        events.push(Event::MessageStart {
            id: chunk.id.unwrap_or_default().into_boxed_str(),
            model: chunk.model.unwrap_or_default().into_boxed_str(),
        });
    }

    for choice in chunk.choices {
        if choice.index != 0 {
            return Err(Error::Unsupported(
                "chat streaming multiple choices is not supported yet".to_owned(),
            ));
        }

        if let Some(delta) = choice.delta {
            if let Some(content) = delta.content {
                let index = match state.open_index(PartKind::Text) {
                    Some(index) => index,
                    None => {
                        let index = next_index;
                        next_index += 1;
                        events.push(Event::PartStart {
                            index,
                            kind: PartKind::Text,
                            tool: None,
                        });
                        open_indices.push(index);
                        index
                    }
                };
                events.push(Event::PartDelta {
                    index,
                    delta: Delta::Text(content.into_boxed_str()),
                });
            }

            if let Some(tool_calls) = delta.tool_calls {
                for tool_call in tool_calls {
                    if let Some(kind) = tool_call.kind.as_deref() {
                        if kind != "function" {
                            return Err(Error::Unsupported(format!(
                                "chat tool call type {kind} is not supported"
                            )));
                        }
                    }

                    let index = if let Some(index) = state
                        .tool_part_index(tool_call.index)
                        .or_else(|| tool_indices.get(&tool_call.index).copied())
                    {
                        index
                    } else {
                        let id = tool_call.id.ok_or_else(|| {
                            Error::Protocol(format!(
                                "chat tool call {} start missing id",
                                tool_call.index
                            ))
                        })?;
                        let name = tool_call
                            .function
                            .as_ref()
                            .and_then(|function| function.name.clone())
                            .ok_or_else(|| {
                                Error::Protocol(format!(
                                    "chat tool call {} start missing function name",
                                    tool_call.index
                                ))
                            })?;
                        let source_index = tool_call.index as usize;
                        let index = if !used_indices.contains(&source_index) {
                            source_index
                        } else {
                            next_index
                        };
                        next_index = next_index.max(index + 1);
                        used_indices.push(index);
                        tool_indices.insert(tool_call.index, index);
                        events.push(Event::PartStart {
                            index,
                            kind: PartKind::ToolUse,
                            tool: Some(ToolStart {
                                source_index: Some(tool_call.index),
                                id: ToolId(id.into_boxed_str()),
                                name: name.into_boxed_str(),
                                kind: ToolUseKind::Client,
                            }),
                        });
                        open_indices.push(index);
                        index
                    };

                    if let Some(arguments) =
                        tool_call.function.and_then(|function| function.arguments)
                    {
                        events.push(Event::PartDelta {
                            index,
                            delta: Delta::ToolArguments(arguments.into_boxed_str()),
                        });
                    }
                }
            }
        }

        if let Some(finish_reason) = choice.finish_reason {
            open_indices.sort_unstable();
            for index in open_indices.drain(..) {
                events.push(Event::PartStop { index });
            }
            events.push(Event::Finish(decode_finish(Some(finish_reason), None)));
        }
    }

    if let Some(usage) = chunk.usage {
        let usage = decode_usage(usage);
        events.push(Event::UsagePatch(UsagePatch {
            input: usage.input,
            output: usage.output,
            cached: Some(usage.cached),
            cache_creation: Some(usage.cache_creation),
            reasoning: Some(usage.reasoning),
        }));
    }

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
        Event::MessageStart { id, model } => Ok(vec![json_frame(ChatChunkOut {
            id: id.to_string(),
            object: "chat.completion.chunk",
            created: 0,
            model: model.to_string(),
            choices: vec![ChatChunkChoiceOut {
                index: 0,
                delta: ChatDeltaOut {
                    role: Some("assistant"),
                    ..ChatDeltaOut::default()
                },
                finish_reason: None,
            }],
            usage: None,
        })?]),
        Event::PartStart {
            index,
            kind: PartKind::ToolUse,
            tool: Some(tool),
        } => {
            let (id, model) = response_metadata(state);
            Ok(vec![json_frame(ChatChunkOut {
                id,
                object: "chat.completion.chunk",
                created: 0,
                model,
                choices: vec![ChatChunkChoiceOut {
                    index: 0,
                    delta: ChatDeltaOut {
                        tool_calls: Some(vec![ChatToolCallDeltaOut {
                            index: *index as u32,
                            id: Some(tool.id.0.to_string()),
                            kind: Some("function"),
                            function: ChatFunctionCallDeltaOut {
                                name: Some(tool.name.to_string()),
                                arguments: None,
                            },
                        }]),
                        ..ChatDeltaOut::default()
                    },
                    finish_reason: None,
                }],
                usage: None,
            })?])
        }
        Event::PartStart { .. } | Event::PartStop { .. } => Ok(Vec::new()),
        Event::PartDelta { index, delta } => match delta {
            Delta::Text(text) => {
                let (id, model) = response_metadata(state);
                Ok(vec![json_frame(ChatChunkOut {
                    id,
                    object: "chat.completion.chunk",
                    created: 0,
                    model,
                    choices: vec![ChatChunkChoiceOut {
                        index: 0,
                        delta: ChatDeltaOut {
                            content: Some(text.to_string()),
                            ..ChatDeltaOut::default()
                        },
                        finish_reason: None,
                    }],
                    usage: None,
                })?])
            }
            Delta::ToolArguments(arguments) => {
                let (id, model) = response_metadata(state);
                Ok(vec![json_frame(ChatChunkOut {
                    id,
                    object: "chat.completion.chunk",
                    created: 0,
                    model,
                    choices: vec![ChatChunkChoiceOut {
                        index: 0,
                        delta: ChatDeltaOut {
                            tool_calls: Some(vec![ChatToolCallDeltaOut {
                                index: *index as u32,
                                id: None,
                                kind: None,
                                function: ChatFunctionCallDeltaOut {
                                    name: None,
                                    arguments: Some(arguments.to_string()),
                                },
                            }]),
                            ..ChatDeltaOut::default()
                        },
                        finish_reason: None,
                    }],
                    usage: None,
                })?])
            }
            Delta::Thinking(_) | Delta::Opaque(_) => Ok(Vec::new()),
        },
        Event::UsagePatch(patch) => {
            let (id, model) = response_metadata(state);
            Ok(vec![json_frame(ChatChunkOut {
                id,
                object: "chat.completion.chunk",
                created: 0,
                model,
                choices: Vec::new(),
                usage: Some(UsageOut {
                    prompt_tokens: patch.input,
                    completion_tokens: patch.output,
                    total_tokens: match (patch.input, patch.output) {
                        (Some(input), Some(output)) => Some(input + output),
                        _ => None,
                    },
                    prompt_tokens_details: patch
                        .cached
                        .map(|cached_tokens| PromptTokensDetailsOut { cached_tokens }),
                    completion_tokens_details: patch
                        .reasoning
                        .map(|reasoning_tokens| CompletionTokensDetailsOut { reasoning_tokens }),
                }),
            })?])
        }
        Event::Finish(finish) => {
            let (id, model) = response_metadata(state);
            let finish_frame = json_frame(ChatChunkOut {
                id,
                object: "chat.completion.chunk",
                created: 0,
                model,
                choices: vec![ChatChunkChoiceOut {
                    index: 0,
                    delta: ChatDeltaOut::default(),
                    finish_reason: Some(stop_reason_name(finish)),
                }],
                usage: None,
            })?;
            Ok(vec![finish_frame, done_frame()])
        }
        Event::Error(error) => Ok(vec![json_value_frame(serde_json::json!({
            "error": {
                "message": error.to_string(),
            }
        }))?]),
    }
}

fn response_metadata(state: &StreamState) -> (String, String) {
    state
        .message()
        .map(|(id, model)| (id.to_owned(), model.to_owned()))
        .unwrap_or_default()
}

fn json_frame(value: ChatChunkOut) -> Result<SseFrame, Error> {
    json_value_frame(serde_json::to_value(value).map_err(|error| {
        Error::Protocol(format!("failed to serialize chat stream frame: {error}"))
    })?)
}

fn json_value_frame(value: serde_json::Value) -> Result<SseFrame, Error> {
    Ok(SseFrame {
        event: None,
        data: Some(serde_json::to_string(&value).map_err(|error| {
            Error::Protocol(format!("failed to serialize chat stream frame: {error}"))
        })?),
    })
}

fn done_frame() -> SseFrame {
    SseFrame {
        event: None,
        data: Some("[DONE]".to_owned()),
    }
}
