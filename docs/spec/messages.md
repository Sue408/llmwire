# spec/messages.md — Anthropic Messages codec

> 状态：半稳定　|　最后更新：2026-09-14
> 实现本 codec 前必读：`IR.md` + `STREAMING.md`。
> 端点：`POST /v1/messages`。本文描述本协议与 IR 的**双向映射**。

---

## 1. 对象模型速览

- 对话载体：`messages[]`，`role` **仅 user / assistant**。
- 系统提示：**顶层 `system` 字段**（string 或带 `cache_control` 的 text block 数组）。
- 内容单元：**content block 数组**（`text` / `image` / `tool_use` / `tool_result` / `thinking` / `redacted_thinking`）。
- 工具调用：`content[]` 内 `tool_use` block，参数是**对象**（`input`）。
- 工具结果：user message 内 `tool_result` block。
- 停止语义：`stop_reason`。
- usage：`usage.{input,output}_tokens` + `cache_read_input_tokens` / `cache_creation_input_tokens`。

## 2. 请求映射

### 2.1 Messages → IR

| Messages 字段 | IR | 说明 |
|---|---|---|
| 顶层 `system`（string / block 数组） | `Conversation.system` | block 数组中的 `cache_control` 经 `Opaque` 透传，**不重算**（INV-2） |
| `messages[*].role:"user"` | `Turn{User}` | |
| `messages[*].role:"assistant"` | `Turn{Assistant}` | |
| `content[].type:"text"` | `Part::Text` | |
| `content[].type:"image"` | `Part::Image` | `source.base64` → `ImageRef` |
| `content[].type:"tool_use"` | `Part::ToolUse` | `input`（对象）进 `RawJson`；`id`→`ToolId`（`toolu_` 前缀原样） |
| `content[].type:"tool_result"` | `Part::ToolResult` | `content` 可为 string 或 block 数组 → `ToolResultContent` |
| `content[].type:"thinking"` | `Part::Thinking` | `signature` 进 `Opaque{AnthropicThinkingSignature}` |
| `content[].type:"redacted_thinking"` | `Part::Opaque{AnthropicRedactedThinking}` | 字节保真 |
| `tools[].input_schema` | `ToolDef.parameters` | 进 `RawJson` |
| `tool_choice` | `ToolChoice` | `auto/any/none` → `Auto/Required/None`；`{type:"tool",name}` → `Named(name)` |
| `max_tokens`（**必填**） | `Sampling.max_output_tokens` | |
| `temperature`（0–1） | `Sampling.temperature` | 注意区间与 OpenAI 不同，**不重标度** |
| `top_k` | `Sampling.top_k` | |
| `thinking.budget_tokens` | `Reasoning.budget_tokens` | `enabled` → `Reasoning.enabled` |
| `stop_sequences`（数组） | `Sampling.stop` | |

### 2.2 IR → Messages

| IR | Messages 字段 | 说明 |
|---|---|---|
| `Conversation.system` | 顶层 `system` | 合并为 string 或 block 数组；**不得**产出 `role:"system"` 消息 |
| `Turn{User}` `ToolResult` parts | user message 内 `tool_result` block | 多个聚合进同一 user message |
| `Turn{Assistant}` `ToolUse` parts | `tool_use` block | `input` 用 `RawJson::raw()` 解析；保证稳定 key order |
| `ToolDef` | `tools[]`，schema 放 `input_schema` | `parameters` 用 `raw()` |
| `Sampling.max_output_tokens` | `max_tokens` | 若为 `None`，须由 host 能力表补默认；**不得缺失** |
| `Reasoning.effort` | `thinking.budget_tokens` | 无一一对应时按能力降级并 `Report` |
| `ToolUseKind::Server/Remote` | 原生 tool 语义 | 需保留 id 前缀，不得误判为普通 `tool_use`（见 §6） |

## 3. 响应映射

### 3.1 Messages → IR

| Messages | IR |
|---|---|
| `content[]` blocks | `Choice.parts`（单 choice） |
| `stop_reason` | `Finish{canonical, provider_raw}` |
| `usage.input_tokens` | `Usage.input`（见 usage 规约） |
| `usage.output_tokens` | `Usage.output` |
| `usage.cache_read_input_tokens` | `Usage.cached` |
| `usage.cache_creation_input_tokens` | `Usage.cache_creation` |

### 3.2 IR → Messages

- 顶层填 `id:"msg_*"` / `type:"message"` / `role:"assistant"` / `content[]` / `model` / `stop_reason` / `usage`。
- thinking block 原样回传，**绝不**生成假 `signature`（INV-1）。

## 4. 流式映射

Messages SSE 事件序列：`message_start → content_block_start → content_block_delta* → content_block_stop → message_delta → message_stop`（含 `ping`）。

| Messages 事件 | 内部事件 |
|---|---|
| `message_start` | `MessageStart` + `UsagePatch{input}`（此刻 input 已知，见 `STREAMING.md §7`） |
| `content_block_start`（`text`/`tool_use`/`thinking`） | `PartStart{kind}` + （tool_use 时带 id/name） |
| `content_block_delta.type:"text_delta"` | `PartDelta::Text` |
| `content_block_delta.type:"thinking_delta"` | `PartDelta::Thinking` |
| `content_block_delta.type:"input_json_delta"` | `PartDelta::ToolArguments`（`partial_json`） |
| `content_block_delta.type:"signature_delta"` | `PartDelta` → `Opaque` 追加 |
| `content_block_stop` | `PartStop` |
| `message_delta` | `UsagePatch{output}` + `Finish`（`stop_reason`） |
| `message_stop` | `Finish` 终止 / `Termination::Explicit` |
| `error` | `Event::Error`（信道 B） |
| `ping` | 忽略（不产生事件） |

## 5. StopReason 映射

| Messages `stop_reason` | IR `StopReason` |
|---|---|
| `end_turn` | `EndTurn` |
| `max_tokens` | `MaxTokens` |
| `stop_sequence` | `StopSequence` |
| `tool_use` | `ToolUse` |
| `pause_turn` | `Pause` |
| `refusal` | `ContentFilter` |
| 其它 | `Other(raw)` + `Report` |

## 6. 本协议专属陷阱

- **MSG-TRAP-1**：服务端工具 id 带 `srvtoolu_` 前缀，必须判为 `ToolUseKind::Server`，否则 `web_search_tool_result` 丢失、多轮重建失败（LiteLLM #17746）。
- **MSG-TRAP-2**：`max_tokens` 必填；从 IR 转出时若缺失，须补默认值并记 `Report`，绝不能省略。
- **MSG-TRAP-3**：`cache_control` 断点一律透传，**不新增、不移动、不重算**（INV-2）。
- **MSG-TRAP-4**：`thinking` / `redacted_thinking` 在 assistant trajectory 内必须保留；缺少可能导致 400（待实测，见 `../DESIGN.md §10`）。`Strip` 仅在目标上游非原生时允许。
- **MSG-TRAP-5**：usage 前后分离：`message_start` 给 input，`message_delta` 给 output，流结束时合并（`STREAMING.md §7`）。
- **MSG-TRAP-6**：`input_json_delta.partial_json` 拼接后才可解析，禁止逐 chunk `parse`。
- **MSG-TRAP-7**：`temperature` 区间为 0–1，与 OpenAI 的 0–2 不同；跨协议转换按目标区间**裁剪并上报**，不做静默映射。
- **MSG-TRAP-8**：本协议无 `n`；IR 侧 `n>1` 需在上游能力不支持时上报。

## 7. 契约测试清单（本协议）

- [ ] 纯文本非流式/流式往返
- [ ] tool_use 多轮：`tool_use.id` 字节保真（golden）
- [ ] tool_result 聚合进同一 user message
- [ ] thinking 开/关 + `redacted_thinking` 字节保真
- [ ] `cache_control` 原样透传
- [ ] 并行 tool_use（content_block index）
- [ ] `input_json_delta` 跨 chunk 拼接 + 转义切断
- [ ] `message_start` 缺 input / `message_delta` 合并
- [ ] `ping` 容忍
- [ ] `max_tokens` 缺失补默认
- [ ] 服务端工具 `srvtoolu_` 判定
