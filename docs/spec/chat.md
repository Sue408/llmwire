# spec/chat.md — OpenAI Chat Completions codec

> 状态：半稳定　|　最后更新：2026-09-14
> 实现本 codec 前必读：`IR.md` + `STREAMING.md`。
> 端点：`POST /v1/chat/completions`。本文描述本协议与 IR 的**双向映射**。

---

## 1. 对象模型速览

- 对话载体：`messages[]` 线性数组，`role ∈ {system, developer, user, assistant, tool}`。
- 系统提示：`role:"system"` / `role:"developer"` 的消息（非顶层字段）。
- 内容单元：`content` 为字符串或 content-part 数组。
- 工具调用：`message.tool_calls[]`，参数是 **JSON 字符串**。
- 工具结果：`role:"tool"` 消息，带 `tool_call_id`。
- 停止语义：`choices[].finish_reason`。
- usage：`usage.{prompt,completion,total}_tokens` + `*_tokens_details`。

## 2. 请求映射

### 2.1 Chat → IR

| Chat 字段 | IR | 说明 |
|---|---|---|
| `messages[*].role:"system"` / `"developer"` | `Conversation.system` | 提升为顶层（CF-2）；迟到者记 `Degraded`（CF-3） |
| `messages[*].role:"user"` | `Turn{User}` | |
| `messages[*].role:"assistant"` | `Turn{Assistant}` | `content` 文本进 `Part::Text`；`tool_calls[]` 逐个映射为 `Part::ToolUse` |
| `messages[*].role:"tool"` | `Turn{User}` 内 `Part::ToolResult` | 连续 tool + 后续 user 合并进同一 User turn（CF-1） |
| `tool_call_id` | `ToolResult.tool_use_id` | 字节保真 |
| `content` 单字符串 | `Part::Text` / `ToolResultContent::Text` | |
| `content` part 数组 | 逐 part 映射 | `text`→`Text`；`image_url`→`Image` |
| `tools[].function` | `ToolDef` | `parameters` 进 `RawJson`（字节权威） |
| `tool_choice` | `ToolChoice` | `auto/required/none` → `Auto/Required/None`；`{type:"function",function:{name}}` → `Named(name)` |
| `max_completion_tokens` | `Sampling.max_output_tokens` | 优先 |
| `max_tokens` | `Sampling.max_output_tokens` | 弃用别名，映射并记 `Report`（`Degraded`） |
| `temperature/top_p/seed/presence_penalty/frequency_penalty` | `Sampling` | |
| `stop`（string） | `Sampling.stop` 单元素数组 | 升维 |
| `n` | `Sampling.n` | |
| `reasoning_effort` | `Reasoning.effort` | |
| `stream` | 由 `Converter` 记录模式 | 不进 IR |

### 2.2 IR → Chat

| IR | Chat 字段 | 说明 |
|---|---|---|
| `Conversation.system` | 前置 `role:"system"` 消息 | 首个位置插入；`cache_control` 若存在经 `Opaque` 透传 |
| `Turn{Assistant}` `ToolUse` parts | `message.tool_calls[]` | `arguments` 用 `RawJson::raw()`（IR-INV-RAW-1） |
| `Turn{User}` `ToolResult` parts | `role:"tool"` 消息 | `tool_call_id` = `tool_use_id` |
| `ToolDef` | `tools[].function` | `parameters` 用 `raw()` |
| `Sampling.max_output_tokens` | `max_completion_tokens` | 新模型；旧模型可降级为 `max_tokens` 并记 `Report` |
| `Sampling.stop` | `stop`（单值） | 多值无法表达时**上报**，不得静默取第一个 |
| `ToolUseKind::Server` / `Remote` | 无对应 | 上报 `Report`（`NotRepresentable`） |
| `Part::Thinking` / `Opaque` | 无对应 | 按 `ThinkingPolicy` 处理（见 `../DESIGN.md §4`） |

## 3. 响应映射

### 3.1 Chat → IR

| Chat | IR |
|---|---|
| `choices[i].message` | `Choice{index: i, parts}` |
| `message.content` | `Part::Text` |
| `message.tool_calls[]` | `Part::ToolUse`（`arguments` 存 `RawJson`） |
| `message.refusal` | `Part::Text` + `Report`（语义降级）或 `Opaque` |
| `finish_reason` | `Finish{canonical, provider_raw}` |
| `usage.prompt_tokens` | `Usage.input` |
| `usage.completion_tokens` | `Usage.output` |
| `usage.*_tokens_details.cached_tokens` | `Usage.cached` |
| `usage.*_tokens_details.reasoning_tokens` | `Usage.reasoning` |

### 3.2 IR → Chat

- 顶层必须完整填充 `id`/`object:"chat.completion"`/`created`/`model`/`choices`/`usage`（`DESIGN.md` TRAP-9 对应报告坑 9）。
- `Finish.provider_raw` 放入扩展字段（如 `extensions.provider_finish_reason`），对外标准值由 `canonical` 决定。
- `Part::ToolUse` → `message.tool_calls[]`，`Part::Text` → `message.content`。

## 4. 流式映射

Chat 流：单 `data:` JSON chunk，末 `data: [DONE]`。

| 内部事件 | Chat chunk |
|---|---|
| `MessageStart` | 首 chunk（带 `delta.role:"assistant"`） |
| `PartStart{Text}` | 首个非空 `delta.content` 前**合成** |
| `PartDelta::Text` | `delta.content` |
| `PartStart{ToolUse}` | `delta.tool_calls[i]` 首现（含 `id`/`type`/`function.name`） |
| `PartDelta::ToolArguments` | `delta.tool_calls[i].function.arguments` 分片 |
| `PartStop` | `finish_reason` 出现时**合成** |
| `UsagePatch` | `include_usage` 的独立末包（`choices:[]`） |
| `Finish` | `finish_reason` + `data: [DONE]` |
| `Error` | 信道 B：流内错误 |

反向（IR → Chat 流）：`ToolArguments` 需拆回 `delta.tool_calls[i].function.arguments`，`index` 由 IR `PartStart.index` 提供。

## 5. StopReason 映射

| Chat `finish_reason` | IR `StopReason` |
|---|---|
| `stop` | `EndTurn` |
| `length` | `MaxTokens` |
| `tool_calls` | `ToolUse` |
| `content_filter` | `ContentFilter` |
| `function_call`（legacy） | `ToolUse` + `Report` |
| 其它私有值 | `Other(raw)` + `Report`，**不得**降级为 `EndTurn` |

## 6. 本协议专属陷阱

- **CHAT-TRAP-1**：`max_tokens` 与 `max_completion_tokens` 不是同义词，新模型优先后者；o-series 不兼容 `max_tokens`。
- **CHAT-TRAP-2**：`assistant` 消息的 `tool_calls` 与 `content` 可共存，转换时**不得**只保留 `content`。
- **CHAT-TRAP-3**：`arguments` 是不保证合法 JSON 的前缀流；只在 `PartStop`/`Finish` 后解析（`STREAMING.md` STR-3）。
- **CHAT-TRAP-4**：`n>1` 时 `choices` 多个，必须进 `AssistantOutput.choices`。
- **CHAT-TRAP-5**：空字符串 `delta.content` 是合法增量，不得用 `if delta` 过滤。
- **CHAT-TRAP-6**：`[DONE]` 不是 JSON，须先字符串比较再 JSON 解析。

## 7. 契约测试清单（本协议）

- [ ] 纯文本非流式/流式往返
- [ ] 多轮 tool call：`tool_call_id == tool_use.id`（golden）
- [ ] 并行 tool_calls（`index` 区分）
- [ ] `arguments` 跨 chunk 拼接 + 转义符切断
- [ ] 首个 chunk 无 role / 重复 role
- [ ] `include_usage` 末包 / 缺失 usage
- [ ] `n>1`
- [ ] 注释行 `: keep-alive` 容忍
- [ ] `finish_reason` 私有值
- [ ] 未知字段进 `Report`（INV-3）
