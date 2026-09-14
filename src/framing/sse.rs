use crate::Error;
use std::fmt;

/// 未配置时允许保留的单个不完整 SSE 帧最大字节数。
pub const DEFAULT_MAX_BUFFER_BYTES: usize = 1024 * 1024;

/// 一个完整 SSE 帧。
#[derive(Clone, PartialEq, Eq)]
pub struct SseFrame {
    /// `event:` 字段。
    pub event: Option<String>,
    /// 合并后的 `data:` 字段。
    pub data: Option<String>,
}

impl fmt::Debug for SseFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SseFrame")
            .field("event", &self.event)
            .field("data_len", &self.data.as_ref().map(String::len))
            .finish()
    }
}

/// 将一个 SSE 帧编码为字节。
pub fn encode_frame(frame: &SseFrame) -> Vec<u8> {
    let mut out = Vec::new();

    if let Some(event) = &frame.event {
        out.extend_from_slice(b"event: ");
        out.extend_from_slice(event.as_bytes());
        out.push(b'\n');
    }

    if let Some(data) = &frame.data {
        for line in data.split('\n') {
            out.extend_from_slice(b"data: ");
            out.extend_from_slice(line.as_bytes());
            out.push(b'\n');
        }
    }

    out.push(b'\n');
    out
}

/// 可增量喂入字节、输出完整 SSE 帧的分帧器。
pub struct SseFramer {
    pending: Vec<u8>,
    max_buffer_bytes: usize,
}

impl Default for SseFramer {
    fn default() -> Self {
        Self::new()
    }
}

impl SseFramer {
    /// 创建使用默认缓冲区上限的分帧器。
    pub fn new() -> Self {
        Self::with_limit(DEFAULT_MAX_BUFFER_BYTES)
    }

    /// 创建指定不完整帧缓冲区上限的分帧器。
    pub fn with_limit(max_buffer_bytes: usize) -> Self {
        Self {
            pending: Vec::new(),
            max_buffer_bytes,
        }
    }

    /// 返回配置的最大缓冲区字节数。
    pub fn max_buffer_bytes(&self) -> usize {
        self.max_buffer_bytes
    }

    /// 返回当前未组成完整帧的字节数。
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// 追加一个字节片段，并输出其中已经完整的帧。
    pub fn feed(&mut self, chunk: &[u8], out: &mut Vec<SseFrame>) -> Result<(), Error> {
        self.pending.extend_from_slice(chunk);
        self.drain_complete_frames(out)?;
        self.ensure_within_limit()
    }

    /// 结束输入，尝试输出尾部帧并清空内部缓冲。
    pub fn finish(&mut self, out: &mut Vec<SseFrame>) -> Result<(), Error> {
        self.drain_complete_frames(out)?;
        self.ensure_within_limit()?;

        if let Some(frame) = parse_frame(&self.pending)? {
            out.push(frame);
        }
        self.pending.clear();
        Ok(())
    }

    fn ensure_within_limit(&self) -> Result<(), Error> {
        if self.pending.len() > self.max_buffer_bytes {
            Err(Error::BufferLimitExceeded)
        } else {
            Ok(())
        }
    }

    fn drain_complete_frames(&mut self, out: &mut Vec<SseFrame>) -> Result<(), Error> {
        let mut frames = Vec::new();
        let mut consumed = 0;

        while let Some((frame_end, next)) = find_frame_end(&self.pending[consumed..]) {
            let frame_start = consumed;
            let frame_end = consumed + frame_end;
            consumed += next;

            if let Some(frame) = parse_frame(&self.pending[frame_start..frame_end])? {
                frames.push(frame);
            }
        }

        if consumed > 0 {
            self.pending.drain(..consumed);
            out.extend(frames);
        }

        Ok(())
    }
}

fn find_frame_end(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut line_start = 0;
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'\n' => {
                let next = index + 1;
                if index == line_start {
                    return Some((line_start, next));
                }
                line_start = next;
                index = next;
            }
            b'\r' => {
                let next = if bytes.get(index + 1) == Some(&b'\n') {
                    index + 2
                } else {
                    index + 1
                };
                if index == line_start {
                    return Some((line_start, next));
                }
                line_start = next;
                index = next;
            }
            _ => index += 1,
        }
    }

    None
}

fn parse_frame(bytes: &[u8]) -> Result<Option<SseFrame>, Error> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| Error::InvalidInput(format!("invalid sse utf-8: {error}")))?;
    let mut event = None;
    let mut data_lines = Vec::new();
    let mut has_data = false;

    for line in text.split(['\n', '\r']) {
        if line.is_empty() || line.starts_with(':') {
            continue;
        }

        let (field, value) = match line.find(':') {
            Some(index) => (&line[..index], &line[index + 1..]),
            None => (line, ""),
        };
        let value = value.strip_prefix(' ').unwrap_or(value);

        match field {
            "event" => event = Some(value.to_owned()),
            "data" => {
                has_data = true;
                data_lines.push(value);
            }
            _ => {}
        }
    }

    if event.is_none() && !has_data {
        return Ok(None);
    }

    Ok(Some(SseFrame {
        event,
        data: has_data.then(|| data_lines.join("\n")),
    }))
}
