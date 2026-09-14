# spec/STREAMING.md — 流式事件模型与状态机

> 状态：半稳定　|　最后更新：2026-09-14
> 读者：所有涉及 `feed` / `finish` / FSM 的实现者。**动流式前必读。**
> 本文定义跨协议共享的流式语义。分帧细节在 `framing/sse.rs`；各协议事件映射在 `chat.md` / `messages.md` / `responses.md`。

---

## 1. 五路流式模型（背景）

五种流式模型互不兼容，没有一种解析器能通用：

| 协议 | 传输 | 事件标识 | 增量单位 | 结束标志 | usage 位置 |
|---|---|---|---|---|---|
| OpenAI Chat | SSE `data:` | `object:"chat.completion.chunk"` | 字符串 delta | `data: [DONE]` | `include_usage` 的独立末包 |
| OpenAI Responses | SSE `data:` | `event.type` 语义事件（30+ 种） | 语义 delta | `response.completed` | `response.completed` |
| Anthropic | SSE `event:` 具名 | `message_start`/`content_block_*`/`message_delta`/`message_stop` | block 生命周期 + `partial_json` | `message_stop` | 前后分离：start 给 input，delta 给 output |
| Gemini | SSE（`?alt=sse`，非 body 字段） | 无事件名 | **完整响应对象**（非差分） | 流正常关闭 | 末 chunk `usageMetadata` |
| Ollama | **NDJSON** | 无 | 完整对象 | `done:true` | 末对象 |

llmwire 初版只覆盖前三种，但内部事件集按五路设计。

## 2. 内部事件集

```rust
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Event {
    MessageStart { id: Box<str>, model: Box<str> },
    /// block 边界。OpenAI 无此概念，由 codec 合成；Claude 原生具备。
    PartStart { index: usize, kind: PartKind },
    PartDelta { index: usize, delta: Delta },
    PartStop  { index: usize },
    /// 增量补丁：字段为 None 表示"该维度尚未知"，可被后续补丁覆盖。
    UsagePatch(UsagePatch),
    Finish(Finish),
    Error(Box<Error>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PartKind { Text, Thinking, ToolUse, ToolResult, Opaque }

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Delta {
    Text(Box<str>),
    Thinking(Box<str>),
    /// JSON 字符串片段：任意边界、可能不闭合、可能为空
    ToolArguments(Box<str>),
}
```

**一对多是常态**：一个上游事件可展开成多个内部事件，反之亦然。故 codec 的 `decode_event` 返回 `Vec<Event>`。

**`PartStart`/`PartStop` 是必需的，不是优化**：OpenAI 没有 block 边界概念，但 IR 站在细粒度一侧，故 OpenAI 侧由 codec **合成**（首个非空 `delta.content` 时 start，`finish_reason` 时 stop）。合成比凭空恢复可靠。

## 3. 状态机

```rust
pub struct StreamState {
    blocks: Vec<BlockState>,
    args_buf: BTreeMap<usize, String>,   // index -> 累积的 arguments 字符串
    usage: Usage,
    finished: bool,
}

enum BlockState {
    Open { index: usize, kind: PartKind },
    Closed { index: usize },
}
```

### 三条硬规则

- **STR-1**：**只能在 `PartStart` 时创建 block**，不得在收到任意 delta 时临时创建。（LiteLLM #17254 的尾随 `{}` 即违反此条。）
- **STR-2**：并行工具调用靠 `index` 区分，不得靠字符串拼接结果反推。
- **STR-3**：`arguments` 只做字符串累加，**绝不在中途 `parse`**。只有 `PartStop` 或 `Finish` 之后才允许解析。切割点可能落在 `\"` 转义符或 `{}` 中间。

## 4. feed / finish 语义

```rust
fn feed(&mut self, chunk: &[u8], out: &mut Vec<u8>) -> Result<(), Error>;
fn finish(&mut self, out: &mut Vec<u8>) -> Result<Termination, Error>;
```

- `feed` 把字节接入分帧缓冲；**能拼出完整帧才转换**，拼不出的半截留在内部。
- `feed` 返回空 `out` 是**正常状态**（未拼出帧 / 该帧无输出），不是错误。
- `finish` 冲刷尾帧、判定终止原因，并复位 `finished`。
- **STR-4**：分帧缓冲必须设**上限**；超限返回 `Error`，不得无限增长（`DESIGN.md` TRAP-7）。

分帧职责（`framing/sse.rs`）：按 `\n\n` 切帧、先匹配 `data:`、处理多行 `data`、无 payload 的 `event:`。

## 5. 终止判定（严格按序，不可颠倒）

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Termination {
    Explicit,      // 1. 上游显式终止符：[DONE] / message_stop / response.completed / done:true
    CleanClose,    // 2. 上游正常关闭且已收到 terminal finish
    ClientAbort,   // 3. 客户端取消
    Timeout,       // 4. 超时
    NetworkError,  // 5. 网络异常 —— 永远不伪造成功
}
```

- 连接关闭但**无终止符** → 按 `ClientAbort` / `NetworkError` 处理，**不视为正常完成**。
- `Finish` 事件的 `canonical` 与 `provider_raw` 并存，双向可追溯。

## 6. 畸形输入容忍清单

解析器**必须**容忍（否则标准客户端会挂起或报错）：

- `: ` 注释行与心跳
- 首 chunk 无 `role`
- tool call 中间 chunk 重复 `role`
- 末包仍带 `delta`
- `delta: null`、空 JSON 对象
- 未发送 `[DONE]` 就断开
- tool `arguments` 分片为空串
- `index` 不连续 / 乱序（并行工具）

## 7. usage 时序：承认不对称，不伪造

Claude 的 `message_start` 必须带 `input_tokens`，而 OpenAI 流到末尾才有 usage。这是**协议真实不对称，无解**。

- `message_start` 时刻 `input` 为 `None` → codec 输出 `0` 占位，但**同时在 `Report` 记一条 `Degraded`**，并在响应扩展字段标注 `usage.estimated = true`。
- 流结束时发出 `UsagePatch` 补齐真实值（`None` 字段才可被覆盖）。
- **STR-5**：绝不为了填满 `message_start` 而编造 `input_tokens` 且不标注。

## 8. 错误信道（与 `DESIGN.md §6.3` 一致）

- **信道 A**：`Result::Err` —— 分帧错误、致命协议错误、`Strict` 下 `Fatal`，仅限**尚未发出 200**。
- **信道 B**：流内 `Event::Error` —— 200 已发出后的错误，必须编成**目标协议的 error 事件**写入 `out`，并记 `Report`。
- **STR-6**：两信道不可混用。HTTP 200 后不得返 `Err`。

## 9. 相关文档

- IR 类型：`IR.md`
- 各协议事件映射：`chat.md` / `messages.md` / `responses.md`
- 错误与终止（背景）：`../reference/LLM接口协议格式深度研究报告.md` 第八章、9.3 节
