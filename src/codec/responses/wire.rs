use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesRequestIn {
    pub instructions: Option<String>,
    pub input: Option<ResponsesInputIn>,
    pub tools: Option<Vec<ResponsesToolIn>>,
    pub tool_choice: Option<Box<RawValue>>,
    pub reasoning: Option<ResponsesReasoningIn>,
    pub max_output_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub store: Option<bool>,
    pub previous_response_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum ResponsesInputIn {
    Text(String),
    Items(Vec<serde_json::Value>),
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesToolIn {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub parameters: Option<Box<RawValue>>,
    pub strict: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesReasoningIn {
    pub effort: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesResponseIn {
    pub id: Option<String>,
    pub model: Option<String>,
    pub status: String,
    pub incomplete_details: Option<ResponsesIncompleteDetailsIn>,
    #[serde(default)]
    pub output: Vec<serde_json::Value>,
    pub usage: Option<ResponsesUsageIn>,
    pub error: Option<ResponsesErrorIn>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesIncompleteDetailsIn {
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesErrorIn {
    pub message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ItemMetaIn {
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MessageItemIn {
    pub role: String,
    pub content: Option<MessageContentIn>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum MessageContentIn {
    Text(String),
    Parts(Vec<serde_json::Value>),
}

#[derive(Debug, Deserialize)]
pub(crate) struct TextPartIn {
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RefusalPartIn {
    pub refusal: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ImagePartIn {
    pub image_url: Option<ImageUrlIn>,
    pub file_id: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum ImageUrlIn {
    Url(String),
    Object { url: String, detail: Option<String> },
}

#[derive(Debug, Deserialize)]
pub(crate) struct FunctionCallItemIn {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FunctionCallOutputItemIn {
    pub call_id: String,
    pub output: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReasoningItemIn {
    pub encrypted_content: Option<String>,
    pub summary: Option<Vec<ReasoningSummaryPartIn>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReasoningSummaryPartIn {
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesUsageIn {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub input_tokens_details: Option<InputTokensDetailsIn>,
    pub output_tokens_details: Option<OutputTokensDetailsIn>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct InputTokensDetailsIn {
    pub cached_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OutputTokensDetailsIn {
    pub reasoning_tokens: Option<u64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResponsesRequestOut {
    pub model: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    pub input: Vec<Box<RawValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ResponsesToolOut>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Box<RawValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ResponsesReasoningOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResponsesToolOut {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters: Box<RawValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResponsesReasoningOut {
    pub effort: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResponsesResponseOut {
    pub id: String,
    pub object: &'static str,
    pub created_at: u64,
    pub status: String,
    pub model: String,
    pub output: Vec<Box<RawValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incomplete_details: Option<ResponsesIncompleteDetailsOut>,
    pub usage: ResponsesUsageOut,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResponsesIncompleteDetailsOut {
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResponsesUsageOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens_details: Option<InputTokensDetailsOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens_details: Option<OutputTokensDetailsOut>,
}

#[derive(Debug, Serialize)]
pub(crate) struct InputTokensDetailsOut {
    pub cached_tokens: u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct OutputTokensDetailsOut {
    pub reasoning_tokens: u64,
}
