//! 请求级转换门面。
use serde_json::Value;
use std::collections::BTreeSet;

use crate::caps::{Capabilities, Mode, ParamSet, ThinkingPolicy};
use crate::codec::ProtocolCodec;
use crate::framing::{encode_frame, SseFrame, SseFramer};
use crate::ids::OpaqueKind;
use crate::ir::{
    AssistantOutput, Conversation, Delta, Event, Finish, Opaque, Part, PartKind, StopReason,
    StreamState, Termination, ToolChoice,
};
use crate::report::{Report, Severity, UnmappedReason};
use crate::{Error, ProtocolId};

/// 请求级、字节导向的双向协议转换器。
///
/// 一个实例只服务一次请求。非流式顺序是 `request`、`response`、`take_report`；
/// 流式顺序是 `request`、重复 `feed`、`finish`、`take_report`。
pub trait Converter: Send {
    /// 将客户端请求转换为后端请求，并记录本次请求的流式模式与上下文。
    ///
    /// 输出采用追加语义：SDK 不清空 `out`，调用方复用缓冲区前应自行 `clear()`。
    fn request(&mut self, body: &[u8], out: &mut Vec<u8>) -> Result<(), Error>;

    /// 将完整的后端非流式响应转换为客户端响应。
    ///
    /// 对流式请求调用本方法会返回 [`Error::Protocol`]。
    fn response(&mut self, body: &[u8], out: &mut Vec<u8>) -> Result<(), Error>;

    /// 消费一个上游字节片段，并把可输出的流式事件追加到 `out`。
    ///
    /// 调用前必须先以 `stream=true` 的请求调用 [`Converter::request`]。
    fn feed(&mut self, chunk: &[u8], out: &mut Vec<u8>) -> Result<(), Error>;

    /// 冲刷分帧尾部和流式状态，并返回终止原因。
    ///
    /// 调用前必须已进入流式模式。只有明确终止、完整结束或调用方断连等状态才会返回，
    /// 无终止符的普通关闭不会被视为 `Explicit`。
    fn finish(&mut self, out: &mut Vec<u8>) -> Result<Termination, Error>;

    /// 取出并清空当前实例累积的质量报告。
    fn take_report(&mut self) -> Report;
}

/// 构造一个请求级转换器。
///
/// `src` 是客户端协议，`dst` 是后端协议。`caps` 决定思考内容、缓存控制与参数支持等策略；
/// 简单场景可用 [`crate::resolve`] 生成。
///
/// # Errors
///
/// 当前协议组合无法构造转换器时返回 [`Error::Unsupported`]。
pub fn converter(
    src: ProtocolId,
    dst: ProtocolId,
    caps: Capabilities,
) -> Result<Box<dyn Converter>, Error> {
    Ok(Box::new(ConverterImpl {
        src,
        dst,
        source: codec_for(src),
        target: codec_for(dst),
        caps,
        conversation: None,
        state: StreamState::new(),
        source_state: StreamState::new(),
        framer: SseFramer::new(),
        report: Report::new(),
        streaming: None,
        skipped_parts: BTreeSet::new(),
        explicit_termination: false,
    }))
}

struct ConverterImpl {
    src: ProtocolId,
    dst: ProtocolId,
    source: Box<dyn ProtocolCodec>,
    target: Box<dyn ProtocolCodec>,
    caps: Capabilities,
    conversation: Option<Conversation>,
    state: StreamState,
    source_state: StreamState,
    framer: SseFramer,
    report: Report,
    streaming: Option<bool>,
    explicit_termination: bool,
    skipped_parts: BTreeSet<usize>,
}

impl Converter for ConverterImpl {
    fn request(&mut self, body: &[u8], out: &mut Vec<u8>) -> Result<(), Error> {
        let stream = request_stream_mode(body)?;
        let model = request_model(body)?;
        let conversation = self
            .source
            .decode_request_with_report(body, &mut self.report)?;
        self.record_request_drops(&conversation)?;
        let encoded = self
            .target
            .encode_request_with_report(&conversation, &mut self.report)?;
        let encoded = set_request_metadata(encoded, stream, model.as_deref())?;
        self.conversation = Some(conversation);
        self.streaming = Some(stream);
        out.extend_from_slice(&encoded);
        Ok(())
    }

    fn response(&mut self, body: &[u8], out: &mut Vec<u8>) -> Result<(), Error> {
        if self.streaming == Some(true) {
            return Err(Error::Protocol(
                "response body is not valid for a streaming request".to_owned(),
            ));
        }

        let output = self
            .target
            .decode_response_with_report(body, &mut self.report)?;
        let output = self.filter_response_for_source(output);
        let encoded = self
            .source
            .encode_response_with_report(&output, &mut self.report)?;
        out.extend_from_slice(&encoded);
        Ok(())
    }

    fn feed(&mut self, chunk: &[u8], out: &mut Vec<u8>) -> Result<(), Error> {
        if self.streaming != Some(true) {
            return Err(Error::Protocol(
                "feed called before streaming request".to_owned(),
            ));
        }

        if let Err(error) = self.feed_inner(chunk, out) {
            self.emit_stream_error(out, error);
        }
        Ok(())
    }

    fn finish(&mut self, out: &mut Vec<u8>) -> Result<Termination, Error> {
        if self.streaming != Some(true) {
            return Err(Error::Protocol(
                "finish called before streaming request".to_owned(),
            ));
        }

        let mut frames = Vec::new();
        if let Err(error) = self.framer.finish(&mut frames) {
            self.emit_stream_error(out, error);
            return Ok(Termination::NetworkError);
        }
        if let Err(error) = self.process_frames(frames, out) {
            self.emit_stream_error(out, error);
            return Ok(Termination::NetworkError);
        }

        if self.explicit_termination {
            Ok(Termination::Explicit)
        } else if self.state.is_finished() {
            Ok(Termination::CleanClose)
        } else {
            Ok(Termination::ClientAbort)
        }
    }

    fn take_report(&mut self) -> Report {
        std::mem::take(&mut self.report)
    }
}

impl ConverterImpl {
    fn feed_inner(&mut self, chunk: &[u8], out: &mut Vec<u8>) -> Result<(), Error> {
        let mut frames = Vec::new();
        self.framer.feed(chunk, &mut frames)?;
        self.process_frames(frames, out)
    }

    fn process_frames(&mut self, frames: Vec<SseFrame>, out: &mut Vec<u8>) -> Result<(), Error> {
        for frame in frames {
            let decoded = self.target.decode_stream_frame_with_report(
                &frame,
                &self.state,
                &mut self.report,
            )?;
            self.state.apply_all(decoded.events.iter().cloned())?;
            self.record_degradations(&decoded.events);

            for event in &decoded.events {
                if event_is_skipped(event, &self.skipped_parts) {
                    continue;
                }
                if !event_supported_by_source(self.src, self.caps.thinking, event) {
                    if let Event::PartStart { index, .. } = event {
                        self.skipped_parts.insert(*index);
                    }
                    self.report_removed_stream_event(event);
                    continue;
                }

                self.source_state.apply(event.clone())?;
                match self.source.encode_stream_event_with_report(
                    event,
                    &self.source_state,
                    &mut self.report,
                ) {
                    Ok(frames) => append_frames(out, frames),
                    Err(Error::Unsupported(_message)) => {
                        if let Event::PartStart { index, .. } = event {
                            self.skipped_parts.insert(*index);
                        }
                        self.report_removed_stream_event(event);
                    }
                    Err(error) => return Err(error),
                }
            }

            if decoded.termination.is_some() {
                self.explicit_termination = true;
                let has_error = decoded
                    .events
                    .iter()
                    .any(|event| matches!(event, Event::Error(_)));
                if decoded.termination == Some(Termination::Explicit)
                    && !self.state.is_finished()
                    && !has_error
                {
                    self.report.warn(
                        "stream.finish",
                        "upstream terminal did not include finish metadata; synthesized EndTurn",
                        Severity::Degraded,
                    );
                    let finish = Event::Finish(Finish {
                        canonical: StopReason::EndTurn,
                        provider_raw: Box::from(""),
                    });
                    self.state.apply(finish.clone())?;
                    self.source_state.apply(finish.clone())?;
                    match self.source.encode_stream_event_with_report(
                        &finish,
                        &self.source_state,
                        &mut self.report,
                    ) {
                        Ok(frames) => append_frames(out, frames),
                        Err(Error::Unsupported(_message)) => {
                            self.report.unmapped(
                                "stream.finish",
                                UnmappedReason::UnsupportedByTarget,
                                Severity::Degraded,
                            );
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
        }

        Ok(())
    }

    fn record_request_drops(&mut self, conversation: &Conversation) -> Result<(), Error> {
        let mut fields: Vec<String> = Vec::new();
        let mut image_detail_fields: Vec<String> = Vec::new();
        let sampling = &conversation.sampling;

        if sampling.temperature.is_some() && !self.caps.supports(ParamSet::TEMPERATURE) {
            fields.push("sampling.temperature".to_owned());
        }
        if sampling.top_p.is_some() && !self.caps.supports(ParamSet::TOP_P) {
            fields.push("sampling.top_p".to_owned());
        }
        if sampling.top_k.is_some() && !self.caps.supports(ParamSet::TOP_K) {
            fields.push("sampling.top_k".to_owned());
        }
        if sampling.max_output_tokens.is_some() && !self.caps.supports(ParamSet::MAX_OUTPUT_TOKENS)
        {
            fields.push("sampling.max_output_tokens".to_owned());
        }
        if !sampling.stop.is_empty() && !self.caps.supports(ParamSet::STOP) {
            fields.push("sampling.stop".to_owned());
        }
        if sampling.seed.is_some() && !self.caps.supports(ParamSet::SEED) {
            fields.push("sampling.seed".to_owned());
        }
        if sampling.n.is_some_and(|n| n > 1) && !self.caps.supports(ParamSet::N) {
            fields.push("sampling.n".to_owned());
        }
        if sampling.presence_penalty.is_some() && !self.caps.supports(ParamSet::PRESENCE_PENALTY) {
            fields.push("sampling.presence_penalty".to_owned());
        }
        if sampling.frequency_penalty.is_some() && !self.caps.supports(ParamSet::FREQUENCY_PENALTY)
        {
            fields.push("sampling.frequency_penalty".to_owned());
        }
        if (conversation.reasoning.enabled
            || conversation.reasoning.effort.is_some()
            || conversation.reasoning.budget_tokens.is_some())
            && !self.caps.supports(ParamSet::REASONING)
        {
            fields.push("reasoning".to_owned());
        }
        if !conversation.tools.is_empty() && !self.caps.supports(ParamSet::TOOLS) {
            fields.push("tools".to_owned());
        }
        if !matches!(conversation.tool_choice, ToolChoice::Auto)
            && !self.caps.supports(ParamSet::TOOL_CHOICE)
        {
            fields.push("tool_choice".to_owned());
        }
        if self.dst == ProtocolId::Messages {
            for (turn_index, turn) in conversation.turns.iter().enumerate() {
                for (part_index, part) in turn.parts.iter().enumerate() {
                    if let Part::Image(image) = part {
                        if image.detail.is_some() {
                            image_detail_fields.push(format!(
                                "request.messages[{turn_index}].content[{part_index}].detail"
                            ));
                        }
                    }
                }
            }
        }

        let severity = if self.caps.mode == Mode::Strict {
            Severity::Fatal
        } else {
            Severity::Degraded
        };

        for field in &fields {
            self.report.unmapped(
                field.as_str(),
                UnmappedReason::UnsupportedByTarget,
                severity,
            );
        }
        for field in &image_detail_fields {
            self.report
                .unmapped(field.as_str(), UnmappedReason::NotRepresentable, severity);
        }

        if severity == Severity::Fatal && (!fields.is_empty() || !image_detail_fields.is_empty()) {
            let mut rejected = fields;
            rejected.extend(image_detail_fields);
            return Err(Error::Unsupported(format!(
                "strict mode rejected unsupported fields: {}",
                rejected.join(", ")
            )));
        }

        Ok(())
    }
    fn filter_response_for_source(&mut self, mut output: AssistantOutput) -> AssistantOutput {
        let severity = if self.caps.mode == Mode::Strict {
            Severity::Fatal
        } else {
            Severity::Degraded
        };

        for (choice_index, choice) in output.choices.iter_mut().enumerate() {
            let mut parts = Vec::with_capacity(choice.parts.len());
            for (part_index, part) in choice.parts.drain(..).enumerate() {
                let stripped_by_policy = (self.src == ProtocolId::Chat
                    || self.caps.thinking == ThinkingPolicy::Strip)
                    && matches!(part, Part::Thinking(_) | Part::Opaque(_));
                if stripped_by_policy || !part_supported_by_source(self.src, &part) {
                    self.report_removed_response_part(choice_index, part_index, part, severity);
                } else {
                    parts.push(part);
                }
            }
            choice.parts = parts;
        }

        output
    }

    fn report_removed_response_part(
        &mut self,
        choice_index: usize,
        part_index: usize,
        part: Part,
        severity: Severity,
    ) {
        let field = format!("response.choices[{choice_index}].parts[{part_index}]");
        match part {
            Part::Thinking(thinking) => {
                self.report.unmapped(
                    format!("{field}.thinking"),
                    UnmappedReason::NotRepresentable,
                    severity,
                );
                if let Some(signature) = thinking.signature {
                    self.report.opaque(
                        format!("{field}.signature"),
                        signature.kind,
                        signature.bytes.len(),
                        severity,
                    );
                }
            }
            Part::Opaque(opaque) => {
                self.report.opaque(
                    format!("{field}.opaque"),
                    opaque.kind,
                    opaque.bytes.len(),
                    severity,
                );
            }
            _ => self
                .report
                .unmapped(field, UnmappedReason::NotRepresentable, severity),
        }
    }

    fn report_removed_stream_event(&mut self, event: &Event) {
        let severity = if self.caps.mode == Mode::Strict {
            Severity::Fatal
        } else {
            Severity::Degraded
        };
        match event {
            Event::PartStart {
                kind: PartKind::Opaque(kind),
                index,
                ..
            } => self
                .report
                .opaque(format!("stream.source_event[{index}]"), *kind, 0, severity),
            Event::PartDelta {
                delta: Delta::Opaque(Opaque { kind, bytes }),
                index,
            } => self.report.opaque(
                format!("stream.source_event[{index}]"),
                *kind,
                bytes.len(),
                severity,
            ),
            Event::PartStart { kind, index, .. } => self.report.unmapped(
                format!("stream.source_event[{index}].{kind:?}"),
                UnmappedReason::UnsupportedByTarget,
                severity,
            ),
            _ => self.report.unmapped(
                "stream.source_event",
                UnmappedReason::UnsupportedByTarget,
                severity,
            ),
        }
    }

    fn record_degradations(&mut self, events: &[Event]) {
        let severity = if self.caps.mode == Mode::Strict {
            Severity::Fatal
        } else {
            Severity::Degraded
        };

        if self.src == ProtocolId::Chat || self.caps.thinking == ThinkingPolicy::Strip {
            for (index, event) in events.iter().enumerate() {
                match event {
                    Event::PartDelta {
                        delta: Delta::Thinking(_),
                        ..
                    }
                    | Event::PartStart {
                        kind: PartKind::Thinking,
                        ..
                    } => self.report.unmapped(
                        format!("stream.events[{index}]"),
                        UnmappedReason::NotRepresentable,
                        severity,
                    ),
                    Event::PartDelta {
                        delta: Delta::Opaque(opaque),
                        ..
                    } => self.report.opaque(
                        format!("stream.events[{index}]"),
                        opaque.kind,
                        opaque.bytes.len(),
                        severity,
                    ),
                    Event::PartStart {
                        kind: PartKind::Opaque(kind),
                        ..
                    } => self
                        .report
                        .opaque(format!("stream.events[{index}]"), *kind, 0, severity),
                    Event::Error(error) => self.report.warn(
                        format!("stream.events[{index}].error"),
                        error.to_string(),
                        severity,
                    ),
                    _ => {}
                }
            }
        }

        if self.src == ProtocolId::Messages {
            for (index, event) in events.iter().enumerate() {
                if matches!(event, Event::MessageStart { .. })
                    && self.source_state.usage().input.is_none()
                {
                    self.report.warn(
                        "stream.message_start.usage.input",
                        "messages message_start requires input_tokens; emitted 0 with estimated=true",
                        severity,
                    );
                }
                if let Event::Error(error) = event {
                    self.report.warn(
                        format!("stream.events[{index}].error"),
                        error.to_string(),
                        severity,
                    );
                }
            }
        }
    }
    fn emit_stream_error(&mut self, out: &mut Vec<u8>, error: Error) {
        self.report.warn(
            "stream.error",
            format!(
                "{} -> {}: {error}",
                protocol_name(self.src),
                protocol_name(self.dst)
            ),
            Severity::Degraded,
        );

        let event = Event::Error(Box::new(error));
        match self.source.encode_stream_event_with_report(
            &event,
            &self.source_state,
            &mut self.report,
        ) {
            Ok(frames) => append_frames(out, frames),
            Err(encode_error) => {
                let frame = SseFrame {
                    event: None,
                    data: Some(
                        serde_json::json!({
                            "error": {
                                "message": encode_error.to_string(),
                            }
                        })
                        .to_string(),
                    ),
                };
                out.extend_from_slice(&encode_frame(&frame));
            }
        }
    }
}

fn event_is_skipped(event: &Event, skipped_parts: &BTreeSet<usize>) -> bool {
    match event {
        Event::PartDelta { index, .. } | Event::PartStop { index } => skipped_parts.contains(index),
        _ => false,
    }
}

fn event_supported_by_source(
    protocol: ProtocolId,
    thinking_policy: ThinkingPolicy,
    event: &Event,
) -> bool {
    match event {
        Event::PartStart { kind, .. } => part_kind_supported_by_source(protocol, *kind),
        Event::PartDelta { delta, .. } => match delta {
            Delta::Text(_) | Delta::ToolArguments(_) => true,
            Delta::Thinking(_) => {
                protocol != ProtocolId::Chat && thinking_policy != ThinkingPolicy::Strip
            }
            Delta::Opaque(opaque) => opaque_supported_by_source(protocol, opaque.kind),
        },
        _ => true,
    }
}

fn part_kind_supported_by_source(protocol: ProtocolId, kind: PartKind) -> bool {
    match protocol {
        ProtocolId::Chat => matches!(kind, PartKind::Text | PartKind::ToolUse),
        ProtocolId::Messages => matches!(
            kind,
            PartKind::Text
                | PartKind::Thinking
                | PartKind::ToolUse
                | PartKind::Opaque(OpaqueKind::AnthropicRedactedThinking)
                | PartKind::Opaque(OpaqueKind::ProviderSpecific("anthropic_content_block"))
        ),
        ProtocolId::Responses => matches!(
            kind,
            PartKind::Text
                | PartKind::Thinking
                | PartKind::ToolUse
                | PartKind::Opaque(OpaqueKind::ResponsesEncryptedReasoning)
                | PartKind::Opaque(OpaqueKind::ProviderSpecific("responses_item"))
        ),
    }
}

fn opaque_supported_by_source(protocol: ProtocolId, kind: OpaqueKind) -> bool {
    match protocol {
        ProtocolId::Chat => false,
        ProtocolId::Messages => matches!(
            kind,
            OpaqueKind::AnthropicThinkingSignature
                | OpaqueKind::AnthropicRedactedThinking
                | OpaqueKind::ProviderSpecific("anthropic_content_block")
        ),
        ProtocolId::Responses => matches!(
            kind,
            OpaqueKind::ResponsesEncryptedReasoning
                | OpaqueKind::ProviderSpecific("responses_item")
        ),
    }
}

fn part_supported_by_source(protocol: ProtocolId, part: &Part) -> bool {
    match protocol {
        ProtocolId::Chat => matches!(part, Part::Text(_) | Part::ToolUse(_)),
        ProtocolId::Messages => matches!(
            part,
            Part::Text(_)
                | Part::Image(_)
                | Part::ToolUse(_)
                | Part::ToolResult(_)
                | Part::Thinking(_)
                | Part::Opaque(Opaque {
                    kind: OpaqueKind::AnthropicRedactedThinking,
                    ..
                })
                | Part::Opaque(Opaque {
                    kind: OpaqueKind::ProviderSpecific("anthropic_content_block"),
                    ..
                })
        ),
        ProtocolId::Responses => matches!(
            part,
            Part::Text(_)
                | Part::ToolUse(_)
                | Part::Thinking(crate::ir::Thinking {
                    signature: None,
                    ..
                })
                | Part::Opaque(Opaque {
                    kind: OpaqueKind::ResponsesEncryptedReasoning,
                    ..
                })
                | Part::Opaque(Opaque {
                    kind: OpaqueKind::ProviderSpecific("responses_item"),
                    ..
                })
        ),
    }
}

fn codec_for(protocol: ProtocolId) -> Box<dyn ProtocolCodec> {
    match protocol {
        ProtocolId::Chat => Box::new(crate::codec::Chat),
        ProtocolId::Messages => Box::new(crate::codec::Messages),
        ProtocolId::Responses => Box::new(crate::codec::Responses),
    }
}

fn request_stream_mode(body: &[u8]) -> Result<bool, Error> {
    let value: Value =
        serde_json::from_slice(body).map_err(|error| Error::InvalidInput(error.to_string()))?;
    Ok(value
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

fn request_model(body: &[u8]) -> Result<Option<String>, Error> {
    let value: Value =
        serde_json::from_slice(body).map_err(|error| Error::InvalidInput(error.to_string()))?;
    Ok(value
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_owned))
}
fn set_request_metadata(
    body: Vec<u8>,
    stream: bool,
    model: Option<&str>,
) -> Result<Vec<u8>, Error> {
    let mut value: Value =
        serde_json::from_slice(&body).map_err(|error| Error::InvalidInput(error.to_string()))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| Error::Protocol("request body must be a JSON object".to_owned()))?;

    if let Some(model) = model {
        object.insert("model".to_owned(), Value::String(model.to_owned()));
    }

    if stream {
        object.insert("stream".to_owned(), Value::Bool(true));
    } else {
        object.remove("stream");
    }

    serde_json::to_vec(&value).map_err(|error| Error::Protocol(error.to_string()))
}
fn append_frames(out: &mut Vec<u8>, frames: Vec<SseFrame>) {
    for frame in frames {
        out.extend_from_slice(&encode_frame(&frame));
    }
}

fn protocol_name(protocol: ProtocolId) -> &'static str {
    match protocol {
        ProtocolId::Chat => "chat",
        ProtocolId::Messages => "messages",
        ProtocolId::Responses => "responses",
    }
}
