mod chat;
mod messages;

pub use chat::Chat;
pub use messages::Messages;

use crate::ir::{AssistantOutput, Conversation};
use crate::Error;

pub trait ProtocolCodec {
    fn decode_request(&self, body: &[u8]) -> Result<Conversation, Error>;
    fn encode_request(&self, conversation: &Conversation) -> Result<Vec<u8>, Error>;
    fn decode_response(&self, body: &[u8]) -> Result<AssistantOutput, Error>;
    fn encode_response(&self, output: &AssistantOutput) -> Result<Vec<u8>, Error>;
}
