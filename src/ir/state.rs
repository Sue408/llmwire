use std::collections::BTreeMap;

use crate::{ids::OpaqueKind, Error};

use super::{
    AssistantOutput, Choice, Delta, Event, Finish, Opaque, Part, PartKind, RawJson, Thinking,
    ToolStart, ToolUse, Usage, UsagePatch,
};

#[derive(Debug, Clone, Default)]
pub struct StreamState {
    blocks: Vec<BlockState>,
    args_buf: BTreeMap<usize, String>,
    usage: Usage,
    finished: bool,
    finish: Option<Finish>,
    message: Option<(Box<str>, Box<str>)>,
    error: Option<Error>,
}

#[derive(Debug, Clone)]
enum BlockState {
    Open(OpenBlock),
    Closed(ClosedBlock),
}

#[derive(Debug, Clone)]
struct OpenBlock {
    index: usize,
    kind: PartKind,
    tool: Option<ToolStart>,
    text: String,
    thinking: String,
    opaque_buf: Vec<u8>,
}

#[derive(Debug, Clone)]
struct ClosedBlock {
    index: usize,
    part: Part,
}

impl StreamState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, event: Event) -> Result<(), Error> {
        match event {
            Event::MessageStart { id, model } => {
                self.message = Some((id, model));
            }
            Event::PartStart { index, kind, tool } => self.open_block(index, kind, tool)?,
            Event::PartDelta { index, delta } => self.apply_delta(index, delta)?,
            Event::PartStop { index } => self.close_block(index)?,
            Event::UsagePatch(patch) => self.apply_usage_patch(patch),
            Event::Finish(finish) => {
                self.close_all_blocks()?;
                self.finish = Some(finish);
                self.finished = true;
            }
            Event::Error(error) => self.error = Some(*error),
        }

        Ok(())
    }

    pub fn apply_all(&mut self, events: impl IntoIterator<Item = Event>) -> Result<(), Error> {
        for event in events {
            self.apply(event)?;
        }
        Ok(())
    }

    pub fn has_message_start(&self) -> bool {
        self.message.is_some()
    }

    pub fn usage(&self) -> Usage {
        self.usage
    }

    pub fn message(&self) -> Option<(&str, &str)> {
        self.message
            .as_ref()
            .map(|(id, model)| (id.as_ref(), model.as_ref()))
    }

    pub fn tool_start(&self, index: usize) -> Option<&ToolStart> {
        self.blocks.iter().find_map(|block| match block {
            BlockState::Open(open) if open.index == index => open.tool.as_ref(),
            _ => None,
        })
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    pub fn next_part_index(&self) -> usize {
        self.blocks
            .iter()
            .map(BlockState::index)
            .max()
            .map_or(0, |index| index + 1)
    }

    pub fn part_indices(&self) -> Vec<usize> {
        self.blocks.iter().map(BlockState::index).collect()
    }

    pub fn part_index_used(&self, index: usize) -> bool {
        self.blocks.iter().any(|block| block.index() == index)
    }

    pub fn open_indices(&self) -> Vec<usize> {
        let mut indices = self
            .blocks
            .iter()
            .filter_map(|block| match block {
                BlockState::Open(open) => Some(open.index),
                BlockState::Closed(_) => None,
            })
            .collect::<Vec<_>>();
        indices.sort_unstable();
        indices
    }

    pub fn open_index(&self, kind: PartKind) -> Option<usize> {
        self.blocks.iter().find_map(|block| match block {
            BlockState::Open(open) if open.kind == kind => Some(open.index),
            _ => None,
        })
    }

    pub fn is_open(&self, index: usize) -> bool {
        self.blocks.iter().any(|block| match block {
            BlockState::Open(open) => open.index == index,
            BlockState::Closed(_) => false,
        })
    }

    pub fn tool_part_index(&self, source_index: u32) -> Option<usize> {
        self.blocks.iter().find_map(|block| match block {
            BlockState::Open(open)
                if open.kind == PartKind::ToolUse
                    && open.tool.as_ref().and_then(|tool| tool.source_index)
                        == Some(source_index) =>
            {
                Some(open.index)
            }
            _ => None,
        })
    }

    pub fn finish(&self) -> Option<&Finish> {
        self.finish.as_ref()
    }

    pub fn tool_arguments(&self, index: usize) -> Option<&str> {
        self.args_buf.get(&index).map(String::as_str)
    }

    pub fn part(&self, index: usize) -> Option<&Part> {
        self.blocks.iter().find_map(|block| match block {
            BlockState::Closed(closed) if closed.index == index => Some(&closed.part),
            _ => None,
        })
    }

    pub fn assistant_output(&self) -> Result<AssistantOutput, Error> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }

        if self
            .blocks
            .iter()
            .any(|block| matches!(block, BlockState::Open(_)))
        {
            return Err(Error::Protocol(
                "stream state has open blocks during output construction".to_owned(),
            ));
        }

        let mut parts = self
            .blocks
            .iter()
            .filter_map(|block| match block {
                BlockState::Closed(closed) => Some((closed.index, closed.part.clone())),
                BlockState::Open(_) => None,
            })
            .collect::<Vec<_>>();
        parts.sort_by_key(|(index, _)| *index);

        Ok(AssistantOutput {
            id: self.message.as_ref().map(|(id, _)| id.clone()),
            model: self.message.as_ref().map(|(_, model)| model.clone()),
            choices: vec![Choice {
                index: 0,
                parts: parts.into_iter().map(|(_, part)| part).collect(),
                finish: self.finish.clone().unwrap_or_default(),
            }],
            usage: self.usage,
        })
    }

    fn open_block(
        &mut self,
        index: usize,
        kind: PartKind,
        tool: Option<ToolStart>,
    ) -> Result<(), Error> {
        if self.blocks.iter().any(|block| block.index() == index) {
            return Err(Error::Protocol(format!(
                "part {index} started more than once"
            )));
        }

        match (kind, tool.as_ref()) {
            (PartKind::ToolUse, None) => {
                return Err(Error::Protocol(format!(
                    "tool part {index} missing tool start metadata"
                )));
            }
            (PartKind::ToolUse, Some(_)) | (_, None) => {}
            (_, Some(_)) => {
                return Err(Error::Protocol(format!(
                    "non-tool part {index} has tool start metadata"
                )));
            }
        }

        if kind == PartKind::ToolUse {
            self.args_buf.insert(index, String::new());
        }

        self.blocks.push(BlockState::Open(OpenBlock {
            index,
            kind,
            tool,
            text: String::new(),
            thinking: String::new(),
            opaque_buf: Vec::new(),
        }));

        Ok(())
    }

    fn apply_delta(&mut self, index: usize, delta: Delta) -> Result<(), Error> {
        let block = match self.blocks.iter_mut().find(|block| block.index() == index) {
            Some(BlockState::Open(block)) => block,
            Some(BlockState::Closed(_)) => {
                return Err(Error::Protocol(format!("part {index} is already closed")));
            }
            None => {
                return Err(Error::Protocol(format!(
                    "part {index} delta arrived before PartStart"
                )));
            }
        };

        match delta {
            Delta::Text(text) => {
                require_kind(block, PartKind::Text, index)?;
                block.text.push_str(&text);
            }
            Delta::Thinking(text) => {
                require_kind(block, PartKind::Thinking, index)?;
                block.thinking.push_str(&text);
            }
            Delta::ToolArguments(arguments) => {
                require_kind(block, PartKind::ToolUse, index)?;
                self.args_buf
                    .get_mut(&index)
                    .ok_or_else(|| {
                        Error::Protocol(format!("tool part {index} missing argument buffer"))
                    })?
                    .push_str(&arguments);
            }
            Delta::Opaque(opaque) => match block.kind {
                PartKind::Opaque(kind) if opaque.kind == kind => {
                    block.opaque_buf.extend_from_slice(&opaque.bytes);
                }
                PartKind::Thinking if opaque.kind == OpaqueKind::AnthropicThinkingSignature => {
                    block.opaque_buf.extend_from_slice(&opaque.bytes);
                }
                PartKind::Opaque(kind) => {
                    return Err(Error::Protocol(format!(
                        "opaque part {index} kind mismatch: expected {kind:?}, got {:?}",
                        opaque.kind
                    )));
                }
                _ => {
                    return Err(Error::Protocol(format!(
                        "opaque delta is not valid for part {index}"
                    )));
                }
            },
        }

        Ok(())
    }

    fn close_block(&mut self, index: usize) -> Result<(), Error> {
        let position = self
            .blocks
            .iter()
            .position(|block| block.index() == index)
            .ok_or_else(|| Error::Protocol(format!("part {index} stopped before PartStart")))?;

        let block = match self.blocks.remove(position) {
            BlockState::Open(block) => block,
            BlockState::Closed(closed) => {
                self.blocks.insert(position, BlockState::Closed(closed));
                return Err(Error::Protocol(format!("part {index} stopped twice")));
            }
        };

        let part = finalize_block(block, self.args_buf.get(&index))?;
        self.blocks
            .insert(position, BlockState::Closed(ClosedBlock { index, part }));
        Ok(())
    }

    fn close_all_blocks(&mut self) -> Result<(), Error> {
        let mut indices = self
            .blocks
            .iter()
            .filter_map(|block| match block {
                BlockState::Open(open) => Some(open.index),
                BlockState::Closed(_) => None,
            })
            .collect::<Vec<_>>();
        indices.sort_unstable();

        for index in indices {
            self.close_block(index)?;
        }
        Ok(())
    }

    fn apply_usage_patch(&mut self, patch: UsagePatch) {
        if self.usage.input.is_none() {
            self.usage.input = patch.input;
        }
        if self.usage.output.is_none() {
            self.usage.output = patch.output;
        }
        if self.usage.cached == 0 {
            self.usage.cached = patch.cached.unwrap_or(0);
        }
        if self.usage.cache_creation == 0 {
            self.usage.cache_creation = patch.cache_creation.unwrap_or(0);
        }
        if self.usage.reasoning == 0 {
            self.usage.reasoning = patch.reasoning.unwrap_or(0);
        }
    }
}

impl BlockState {
    fn index(&self) -> usize {
        match self {
            BlockState::Open(block) => block.index,
            BlockState::Closed(block) => block.index,
        }
    }
}

fn require_kind(block: &OpenBlock, expected: PartKind, index: usize) -> Result<(), Error> {
    if block.kind == expected {
        Ok(())
    } else {
        Err(Error::Protocol(format!(
            "delta kind mismatch for part {index}: expected {:?}, found {:?}",
            expected, block.kind
        )))
    }
}

fn finalize_block(block: OpenBlock, arguments: Option<&String>) -> Result<Part, Error> {
    match block.kind {
        PartKind::Text => Ok(Part::Text(block.text)),
        PartKind::Thinking => Ok(Part::Thinking(Thinking {
            text: block.thinking,
            signature: (!block.opaque_buf.is_empty()).then(|| Opaque {
                kind: OpaqueKind::AnthropicThinkingSignature,
                bytes: block.opaque_buf.into_boxed_slice(),
            }),
        })),
        PartKind::ToolUse => {
            let tool = block
                .tool
                .ok_or_else(|| Error::Protocol("tool part missing start metadata".to_owned()))?;
            let arguments = arguments.cloned().unwrap_or_default();
            Ok(Part::ToolUse(ToolUse {
                id: tool.id,
                name: tool.name,
                arguments: RawJson::from_raw(arguments),
                kind: tool.kind,
            }))
        }
        PartKind::ToolResult => Err(Error::Protocol(
            "streaming tool_result output is not supported".to_owned(),
        )),
        PartKind::Opaque(kind) => Ok(Part::Opaque(Opaque {
            kind,
            bytes: block.opaque_buf.into_boxed_slice(),
        })),
    }
}
