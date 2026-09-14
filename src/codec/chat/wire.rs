use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

#[derive(Debug, Deserialize)]
pub(crate) struct ChatRequestIn {
    #[serde(default)]
    pub messages: Vec<MessageIn>,
    pub tools: Option<Vec<ToolIn>>,
    pub tool_choice: Option<Box<RawValue>>,
    pub max_completion_tokens: Option<u32>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub stop: Option<StopIn>,
    pub seed: Option<i64>,
    pub n: Option<u32>,
    pub presence_penalty: Option<f32>,
    pub frequency_penalty: Option<f32>,
    pub reasoning_effort: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MessageIn {
    pub role: String,
    pub content: Option<ContentIn>,
    pub tool_calls: Option<Vec<ToolCallIn>>,
    pub tool_call_id: Option<String>,
    pub refusal: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum ContentIn {
    Text(String),
    Parts(Vec<ContentPartIn>),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ContentPartIn {
    Text {
        text: String,
    },
    ImageUrl {
        image_url: Option<ImageUrlIn>,
        file_id: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct ImageUrlIn {
    pub url: String,
    pub detail: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ToolCallIn {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionCallIn,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FunctionCallIn {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ToolIn {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub function: ToolFunctionIn,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ToolFunctionIn {
    pub name: String,
    pub description: Option<String>,
    pub parameters: Option<Box<RawValue>>,
    pub strict: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum StopIn {
    Text(String),
    List(Vec<String>),
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatResponseIn {
    pub id: Option<String>,
    pub model: Option<String>,
    #[serde(default)]
    pub choices: Vec<ResponseChoiceIn>,
    pub usage: Option<UsageIn>,
    pub extensions: Option<ResponseExtensionsIn>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponseChoiceIn {
    pub index: u32,
    pub message: MessageIn,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponseExtensionsIn {
    pub provider_finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UsageIn {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub prompt_tokens_details: Option<PromptTokensDetailsIn>,
    pub completion_tokens_details: Option<CompletionTokensDetailsIn>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PromptTokensDetailsIn {
    pub cached_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CompletionTokensDetailsIn {
    pub reasoning_tokens: Option<u64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ChatRequestOut {
    pub messages: Vec<MessageOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolOut>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoiceOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<StopOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct MessageOut {
    pub role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<ContentOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallOut>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum ContentOut {
    Text(String),
    Parts(Vec<ContentPartOut>),
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ContentPartOut {
    Text { text: String },
    ImageUrl { image_url: ImageUrlOut },
}

#[derive(Debug, Serialize)]
pub(crate) struct ImageUrlOut {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ToolCallOut {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: FunctionCallOut,
}

#[derive(Debug, Serialize)]
pub(crate) struct FunctionCallOut {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ToolOut {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: ToolFunctionOut,
}

#[derive(Debug, Serialize)]
pub(crate) struct ToolFunctionOut {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters: Box<RawValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum ToolChoiceOut {
    Keyword(&'static str),
    Named(NamedToolChoiceOut),
    Other(Box<RawValue>),
}

#[derive(Debug, Serialize)]
pub(crate) struct NamedToolChoiceOut {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: NamedFunctionOut,
}

#[derive(Debug, Serialize)]
pub(crate) struct NamedFunctionOut {
    pub name: String,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum StopOut {
    Text(String),
    List(Vec<String>),
}

#[derive(Debug, Serialize)]
pub(crate) struct ChatResponseOut {
    pub id: String,
    pub object: &'static str,
    pub created: u64,
    pub model: String,
    pub choices: Vec<ResponseChoiceOut>,
    pub usage: UsageOut,
    pub extensions: ResponseExtensionsOut,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResponseChoiceOut {
    pub index: u32,
    pub message: MessageOut,
    pub finish_reason: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResponseExtensionsOut {
    pub provider_finish_reason: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct UsageOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_details: Option<PromptTokensDetailsOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens_details: Option<CompletionTokensDetailsOut>,
}

#[derive(Debug, Serialize)]
pub(crate) struct PromptTokensDetailsOut {
    pub cached_tokens: u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct CompletionTokensDetailsOut {
    pub reasoning_tokens: u64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatChunkIn {
    pub id: Option<String>,
    pub model: Option<String>,
    #[serde(default)]
    pub choices: Vec<ChatChunkChoiceIn>,
    pub usage: Option<UsageIn>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatChunkChoiceIn {
    pub index: u32,
    pub delta: Option<ChatDeltaIn>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct ChatDeltaIn {
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ChatToolCallDeltaIn>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatToolCallDeltaIn {
    pub index: u32,
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub function: Option<ChatFunctionCallDeltaIn>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatFunctionCallDeltaIn {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ChatChunkOut {
    pub id: String,
    pub object: &'static str,
    pub created: u64,
    pub model: String,
    pub choices: Vec<ChatChunkChoiceOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<UsageOut>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ChatChunkChoiceOut {
    pub index: u32,
    pub delta: ChatDeltaOut,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Serialize, Default)]
pub(crate) struct ChatDeltaOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ChatToolCallDeltaOut>>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ChatToolCallDeltaOut {
    pub index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<&'static str>,
    pub function: ChatFunctionCallDeltaOut,
}

#[derive(Debug, Serialize)]
pub(crate) struct ChatFunctionCallDeltaOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}
