mod chat;
mod messages;
mod responses;

pub use chat::Chat;
pub use messages::Messages;
pub use responses::Responses;

use crate::framing::SseFrame;
use crate::ir::{AssistantOutput, Conversation, Event, StreamState, Termination};
use crate::Error;

#[derive(Debug, Clone, Default)]
pub struct StreamDecode {
    pub events: Vec<Event>,
    pub termination: Option<Termination>,
}

pub trait ProtocolCodec: Send {
    fn decode_request(&self, body: &[u8]) -> Result<Conversation, Error>;
    fn encode_request(&self, conversation: &Conversation) -> Result<Vec<u8>, Error>;
    fn decode_response(&self, body: &[u8]) -> Result<AssistantOutput, Error>;
    fn encode_response(&self, output: &AssistantOutput) -> Result<Vec<u8>, Error>;

    fn decode_stream_frame(
        &self,
        _frame: &SseFrame,
        _state: &StreamState,
    ) -> Result<StreamDecode, Error> {
        Err(Error::Unsupported(
            "stream decoding is not implemented for this codec".to_owned(),
        ))
    }

    fn encode_stream_event(
        &self,
        _event: &Event,
        _state: &StreamState,
    ) -> Result<Vec<SseFrame>, Error> {
        Err(Error::Unsupported(
            "stream encoding is not implemented for this codec".to_owned(),
        ))
    }
}
