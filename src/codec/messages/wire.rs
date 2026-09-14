use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::value::RawValue;

#[derive(Debug, Deserialize)]
pub(crate) struct MessagesRequestIn {
    #[serde(default)]
    pub messages: Vec<MessageIn>,
    pub system: Option<TextOrBlocks>,
    pub tools: Option<Vec<ToolIn>>,
    pub tool_choice: Option<Box<RawValue>>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub stop_sequences: Option<Vec<String>>,
    pub thinking: Option<ThinkingConfigIn>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MessageIn {
    pub role: String,
    pub content: Option<TextOrBlocks>,
}

#[derive(Debug)]
pub(crate) enum TextOrBlocks {
    Text(String),
    Blocks(Vec<Box<RawValue>>),
}

impl<'de> Deserialize<'de> for TextOrBlocks {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = Box::<RawValue>::deserialize(deserializer)?;
        let source = raw.get().trim_start();
        if source.starts_with('"') {
            serde_json::from_str(source)
                .map(Self::Text)
                .map_err(DeError::custom)
        } else if source.starts_with('[') {
            serde_json::from_str(source)
                .map(Self::Blocks)
                .map_err(DeError::custom)
        } else {
            Err(DeError::custom("messages content must be string or array"))
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ThinkingConfigIn {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub budget_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ToolIn {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Option<Box<RawValue>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BlockMetaIn {
    #[serde(rename = "type")]
    pub kind: String,
    pub cache_control: Option<Box<RawValue>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TextBlockIn {
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ImageBlockIn {
    pub source: ImageSourceIn,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ImageSourceIn {
    #[serde(rename = "type")]
    pub kind: String,
    pub media_type: Option<String>,
    pub data: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ToolUseBlockIn {
    pub id: String,
    pub name: String,
    pub input: Box<RawValue>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ToolResultBlockIn {
    pub tool_use_id: String,
    pub content: Option<ToolResultContentIn>,
}

#[derive(Debug)]
pub(crate) enum ToolResultContentIn {
    Text(String),
    Blocks(Vec<Box<RawValue>>),
    Other(Box<RawValue>),
}

impl<'de> Deserialize<'de> for ToolResultContentIn {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = Box::<RawValue>::deserialize(deserializer)?;
        let source = raw.get().trim_start();
        if source.starts_with('"') {
            serde_json::from_str(source)
                .map(Self::Text)
                .map_err(DeError::custom)
        } else if source.starts_with('[') {
            serde_json::from_str(source)
                .map(Self::Blocks)
                .map_err(DeError::custom)
        } else {
            Ok(Self::Other(raw))
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ThinkingBlockIn {
    pub thinking: String,
    pub signature: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RedactedThinkingBlockIn {
    pub data: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct MessagesRequestOut {
    pub model: &'static str,
    pub max_tokens: u32,
    pub messages: Vec<MessageOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<TextOrBlocksOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolOut>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Box<RawValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingConfigOut>,
}

#[derive(Debug, Serialize)]
pub(crate) struct MessageOut {
    pub role: &'static str,
    pub content: TextOrBlocksOut,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum TextOrBlocksOut {
    Text(String),
    Blocks(Vec<Box<RawValue>>),
}

#[derive(Debug, Serialize)]
pub(crate) struct ThinkingConfigOut {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub budget_tokens: u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct ToolOut {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub input_schema: Box<RawValue>,
}

#[derive(Debug, Serialize)]
pub(crate) struct TextBlockOut {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ImageBlockOut {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub source: ImageSourceOut,
}

#[derive(Debug, Serialize)]
pub(crate) struct ImageSourceOut {
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ToolUseBlockOut {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub id: String,
    pub name: String,
    pub input: Box<RawValue>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ToolResultBlockOut {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub tool_use_id: String,
    pub content: TextOrBlocksOut,
}

#[derive(Debug, Serialize)]
pub(crate) struct ThinkingBlockOut {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub thinking: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RedactedThinkingBlockOut {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub data: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MessagesResponseIn {
    #[serde(default)]
    pub content: Vec<Box<RawValue>>,
    pub stop_reason: Option<String>,
    pub stop_sequence: Option<String>,
    pub usage: Option<UsageIn>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UsageIn {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct MessagesResponseOut {
    pub id: &'static str,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub role: &'static str,
    pub content: Vec<Box<RawValue>>,
    pub model: &'static str,
    pub stop_reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequence: Option<String>,
    pub usage: UsageOut,
}

#[derive(Debug, Serialize)]
pub(crate) struct UsageOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u64>,
}
