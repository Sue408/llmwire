use serde_json::Value;

use crate::caps::{Capabilities, ThinkingPolicy};
use crate::codec::ProtocolCodec;
use crate::framing::{encode_frame, SseFrame, SseFramer};
use crate::ir::{Conversation, Delta, Event, PartKind, StreamState, Termination};
use crate::report::{Report, Severity};
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
    if src == ProtocolId::Responses || dst == ProtocolId::Responses {
        return Err(Error::Unsupported(
            "responses codec is not implemented".to_owned(),
        ));
    }

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
        let conversation = self.source.decode_request(body)?;
        let encoded = self.target.encode_request(&conversation)?;
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

        let output = self.target.decode_response(body)?;
        let encoded = self.source.encode_response(&output)?;
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
            let decoded = self.source.decode_stream_frame(&frame, &self.state)?;
            self.state.apply_all(decoded.events.iter().cloned())?;
            self.record_degradations(&decoded.events);

            for event in &decoded.events {
                match self.target.encode_stream_event(event, &self.state) {
                    Ok(frames) => append_frames(out, frames),
                    Err(Error::Unsupported(message)) => {
                        self.report
                            .warn("stream.target_event", message, Severity::Degraded);
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

    fn record_degradations(&mut self, events: &[Event]) {
        if self.dst == ProtocolId::Chat || self.caps.thinking == ThinkingPolicy::Strip {
            for event in events {
                match event {
                    Event::PartDelta {
                        delta: Delta::Thinking(_) | Delta::Opaque(_),
                        ..
                    }
                    | Event::PartStart {
                        kind: PartKind::Thinking | PartKind::Opaque(_),
                        ..
                    } => self.report.warn(
                        "stream.thinking",
                        "target chat cannot represent thinking or opaque stream content",
                        Severity::Degraded,
                    ),
                    Event::Error(error) => {
                        self.report
                            .warn("stream.error", error.to_string(), Severity::Degraded)
                    }
                    _ => {}
                }
            }
        }

        if self.dst == ProtocolId::Messages {
            for event in events {
                if matches!(event, Event::MessageStart { .. }) && self.state.usage().input.is_none()
                {
                    self.report.warn(
                        "usage.input",
                        "messages message_start requires input_tokens; emitted 0 with estimated=true",
                        Severity::Degraded,
                    );
                }
                if let Event::Error(error) = event {
                    self.report
                        .warn("stream.error", error.to_string(), Severity::Degraded);
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
        match self.target.encode_stream_event(&event, &self.state) {
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
        ProtocolId::Responses => unreachable!("responses codec checked by converter factory"),
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
