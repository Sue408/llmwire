use serde_json::Value;

use crate::caps::{Capabilities, Mode, ParamSet, ThinkingPolicy};
use crate::codec::ProtocolCodec;
use crate::framing::{encode_frame, SseFrame, SseFramer};
use crate::ir::{Conversation, Delta, Event, PartKind, StreamState, Termination, ToolChoice};
use crate::report::{Report, Severity, UnmappedReason};
use crate::{Error, ProtocolId};

pub trait Converter: Send {
    fn request(&mut self, body: &[u8], out: &mut Vec<u8>) -> Result<(), Error>;
    fn response(&mut self, body: &[u8], out: &mut Vec<u8>) -> Result<(), Error>;
    fn feed(&mut self, chunk: &[u8], out: &mut Vec<u8>) -> Result<(), Error>;
    fn finish(&mut self, out: &mut Vec<u8>) -> Result<Termination, Error>;
    fn take_report(&mut self) -> Report;
}

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
        framer: SseFramer::new(),
        report: Report::new(),
        streaming: None,
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
    framer: SseFramer,
    report: Report,
    streaming: Option<bool>,
    explicit_termination: bool,
}

impl Converter for ConverterImpl {
    fn request(&mut self, body: &[u8], out: &mut Vec<u8>) -> Result<(), Error> {
        let stream = request_stream_mode(body)?;
        let conversation = self
            .source
            .decode_request_with_report(body, &mut self.report)?;
        self.record_request_drops(&conversation)?;
        let encoded = self
            .target
            .encode_request_with_report(&conversation, &mut self.report)?;
        let encoded = set_stream_mode(encoded, stream)?;
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
            let decoded = self.source.decode_stream_frame_with_report(
                &frame,
                &self.state,
                &mut self.report,
            )?;
            self.state.apply_all(decoded.events.iter().cloned())?;
            self.record_degradations(&decoded.events);

            for event in &decoded.events {
                match self.target.encode_stream_event_with_report(
                    event,
                    &self.state,
                    &mut self.report,
                ) {
                    Ok(frames) => append_frames(out, frames),
                    Err(Error::Unsupported(_message)) => {
                        self.report.unmapped(
                            "stream.target_event",
                            UnmappedReason::UnsupportedByTarget,
                            Severity::Degraded,
                        );
                    }
                    Err(error) => return Err(error),
                }
            }

            if decoded.termination.is_some() {
                self.explicit_termination = true;
            }
        }

        Ok(())
    }

    fn record_request_drops(&mut self, conversation: &Conversation) -> Result<(), Error> {
        let mut fields = Vec::new();
        let sampling = &conversation.sampling;

        if sampling.temperature.is_some() && !self.caps.supports(ParamSet::TEMPERATURE) {
            fields.push("sampling.temperature");
        }
        if sampling.top_p.is_some() && !self.caps.supports(ParamSet::TOP_P) {
            fields.push("sampling.top_p");
        }
        if sampling.top_k.is_some() && !self.caps.supports(ParamSet::TOP_K) {
            fields.push("sampling.top_k");
        }
        if sampling.max_output_tokens.is_some() && !self.caps.supports(ParamSet::MAX_OUTPUT_TOKENS)
        {
            fields.push("sampling.max_output_tokens");
        }
        if !sampling.stop.is_empty() && !self.caps.supports(ParamSet::STOP) {
            fields.push("sampling.stop");
        }
        if sampling.seed.is_some() && !self.caps.supports(ParamSet::SEED) {
            fields.push("sampling.seed");
        }
        if sampling.n.is_some_and(|n| n > 1) && !self.caps.supports(ParamSet::N) {
            fields.push("sampling.n");
        }
        if sampling.presence_penalty.is_some() && !self.caps.supports(ParamSet::PRESENCE_PENALTY) {
            fields.push("sampling.presence_penalty");
        }
        if sampling.frequency_penalty.is_some() && !self.caps.supports(ParamSet::FREQUENCY_PENALTY)
        {
            fields.push("sampling.frequency_penalty");
        }
        if (conversation.reasoning.enabled
            || conversation.reasoning.effort.is_some()
            || conversation.reasoning.budget_tokens.is_some())
            && !self.caps.supports(ParamSet::REASONING)
        {
            fields.push("reasoning");
        }
        if !conversation.tools.is_empty() && !self.caps.supports(ParamSet::TOOLS) {
            fields.push("tools");
        }
        if !matches!(conversation.tool_choice, ToolChoice::Auto)
            && !self.caps.supports(ParamSet::TOOL_CHOICE)
        {
            fields.push("tool_choice");
        }

        let severity = if self.caps.mode == Mode::Strict {
            Severity::Fatal
        } else {
            Severity::Degraded
        };

        for field in &fields {
            self.report
                .unmapped(*field, UnmappedReason::UnsupportedByTarget, severity);
        }

        if severity == Severity::Fatal && !fields.is_empty() {
            return Err(Error::Unsupported(format!(
                "strict mode rejected unsupported fields: {}",
                fields.join(", ")
            )));
        }

        Ok(())
    }
    fn record_degradations(&mut self, events: &[Event]) {
        let severity = if self.caps.mode == Mode::Strict {
            Severity::Fatal
        } else {
            Severity::Degraded
        };

        if self.dst == ProtocolId::Chat || self.caps.thinking == ThinkingPolicy::Strip {
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

        if self.dst == ProtocolId::Messages {
            for (index, event) in events.iter().enumerate() {
                if matches!(event, Event::MessageStart { .. }) && self.state.usage().input.is_none()
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
        match self
            .target
            .encode_stream_event_with_report(&event, &self.state, &mut self.report)
        {
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

fn set_stream_mode(body: Vec<u8>, stream: bool) -> Result<Vec<u8>, Error> {
    let mut value: Value =
        serde_json::from_slice(&body).map_err(|error| Error::InvalidInput(error.to_string()))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| Error::Protocol("request body must be a JSON object".to_owned()))?;

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
