//! 协议 codec 与字节级协议接口。
//!
//! 通常优先使用 [`crate::converter()`] 门面；本模块用于直接调用单协议 codec
//! 或编写更底层的测试与工具。
mod chat;
mod image;
mod messages;
mod responses;

pub use chat::Chat;
pub use messages::Messages;
pub use responses::Responses;

use crate::framing::SseFrame;
use crate::ir::{AssistantOutput, Conversation, Event, StreamState, Termination};
use crate::report::Report;
use crate::Error;

/// 单个 SSE 帧解码后的事件与可选终止信号。
#[derive(Debug, Clone, Default)]
pub struct StreamDecode {
    /// 由该帧产生的事件。
    pub events: Vec<Event>,
    /// 帧中显式携带的终止原因。
    pub termination: Option<Termination>,
}

/// 单协议请求、响应与流式帧的编解码接口。
pub trait ProtocolCodec: Send {
    /// 将协议请求解码为 IR。
    fn decode_request(&self, body: &[u8]) -> Result<Conversation, Error>;
    /// 将 IR 编码为协议请求。
    fn encode_request(&self, conversation: &Conversation) -> Result<Vec<u8>, Error>;
    /// 将协议响应解码为 IR。
    fn decode_response(&self, body: &[u8]) -> Result<AssistantOutput, Error>;
    /// 将 IR 编码为协议响应。
    fn encode_response(&self, output: &AssistantOutput) -> Result<Vec<u8>, Error>;

    fn decode_request_with_report(
        &self,
        body: &[u8],
        _report: &mut Report,
    ) -> Result<Conversation, Error> {
        self.decode_request(body)
    }

    fn encode_request_with_report(
        &self,
        conversation: &Conversation,
        _report: &mut Report,
    ) -> Result<Vec<u8>, Error> {
        self.encode_request(conversation)
    }

    fn decode_response_with_report(
        &self,
        body: &[u8],
        _report: &mut Report,
    ) -> Result<AssistantOutput, Error> {
        self.decode_response(body)
    }

    fn encode_response_with_report(
        &self,
        output: &AssistantOutput,
        _report: &mut Report,
    ) -> Result<Vec<u8>, Error> {
        self.encode_response(output)
    }

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

    fn decode_stream_frame_with_report(
        &self,
        frame: &SseFrame,
        state: &StreamState,
        _report: &mut Report,
    ) -> Result<StreamDecode, Error> {
        self.decode_stream_frame(frame, state)
    }

    fn encode_stream_event_with_report(
        &self,
        event: &Event,
        state: &StreamState,
        _report: &mut Report,
    ) -> Result<Vec<SseFrame>, Error> {
        self.encode_stream_event(event, state)
    }
}
