# spec/responses.md — OpenAI Responses codec

> 状态：半稳定　|　最后更新：2026-09-14
> 实现本 codec 前必读：`IR.md` + `STREAMING.md`。
> 端点：`POST /v1/responses`。本文描述本协议与 IR 的**双向映射**。
> **MVP 范围：仅无状态模式**。`store:true` / `previous_response_id` 明确返回 `Unsupported`，不伪造（见 `../PLAN.md` M3）。

---

## 1. 对象模型速览

- 对话载体：`input`（字符串或 **item 数组**）。
- 系统提示：顶层 `instructions`。
- 内容 part：用户侧 `input_text`，历史 assistant 侧 `output_text`，图片 `input_image`。
- 工具调用：`output[]` 中的 `function_call` item；**`arguments` 是完整 JSON 字符串**。
- 工具结果：`function_call_output` item。
- 停止语义：`status`（`completed/incomplete/failed/cancelled`）+ `incomplete_details`。
- usage：`usage.{input,output,total}_tokens` + `*_tokens_details`。

## 2. 请求映射

### 2.1 Responses → IR

| Responses 字段 | IR | 说明 |
|---|---|---|
| `instructions` | `Conversation.system` | 顶层，每轮独立（**不随 `previous_response_id` 继承**） |
| `input`（string） | `Turn{User}` + `Part::Text` | |
| `input[].type:"message"` | `Turn{role}` | `role ∈ user/assistant/system/developer`；后两者提升进 system |
| `input[].content[].type:"input_text"` | `Part::Text` | |
| `input[].content[].type:"input_image"` | `Part::Image` | |
| `input[].type:"function_call"` | `Part::ToolUse` | `call_id`→`ToolId`；`arguments` 完整字符串进 `RawJson` |
| `input[].type:"function_call_output"` | `Part::ToolResult` | `call_id` 关联 |
| `input[].type:"reasoning"` | `Part::Opaque` / `Thinking` | `encrypted_content` → `Opaque{ResponsesEncryptedReasoning}`，**原样回传** |
| 内置工具 item | `Part::Opaque` | `web_search_call` 等；MVP 不拍平，透传 |
| `tools[].type:"function"` | `ToolDef` | 扁平 `{name, description, parameters}` |
| `tool_choice` | `ToolChoice` | `auto/required/none` + 指定函数 |
| `reasoning.effort` | `Reasoning.effort` | |
| `max_output_tokens`（≥16） | `Sampling.max_output_tokens` | |
| `temperature` / `top_p` | `Sampling` | |
| `store` / `previous_response_id` | — | **无状态模式：拒绝**（`Unsupported`） |

### 2.2 IR → Responses

| IR | Responses 字段 | 说明 |
|---|---|---|
| `Conversation.system` | `instructions` | |
| `Part::Text`（user） | `input[].content[].type:"input_text"` | |
| `Part::Text`（assistant） | `type:"output_text"` | 命名与 Chat 不同 |
| `Part::ToolUse` | `function_call` item | `arguments` 用 `raw()` |
| `Part::ToolResult` | `function_call_output` item | |
| `Part::Opaque` | 对应 item | 字节保真 |
| `ToolDef` | 扁平 function tool | `parameters` 用 `raw()` |

## 3. 响应映射

### 3.1 Responses → IR

| Responses | IR |
|---|---|
| `status` | `Finish`（见 §5） |
| `incomplete_details.reason` | `Finish.provider_raw` |
| `output[].type:"message"` → `content[].type:"output_text"` | `Choice.parts` (`Part::Text`) |
| `output[].type:"function_call"` | `Part::ToolUse` |
| `output[].type:"reasoning"` | `Part::Thinking` / `Opaque` |
| `output_text` | 便捷字段，**不**代表完整结果（多 part/纯工具时不保证） |
| `usage.input_tokens` | `Usage.input` |
| `usage.output_tokens` | `Usage.output` |
| `usage.*_tokens_details.cached_tokens` | `Usage.cached` |
| `usage.*_tokens_details.reasoning_tokens` | `Usage.reasoning` |

### 3.2 IR → Responses

- 顶层填 `id:"resp_*"` / `object:"response"` / `created_at` / `status` / `model` / `output[]` / `usage`。
- `output` 即使空数组也不得省略。

## 4. 流式映射

Responses SSE 是**语义事件**（30+ 种），每帧 `event:` + `data:` 双行，**无 `[DONE]`**，靠 `response.completed` 结束。用 `(output_index, content_index, item_id)` 索引，不可假定严格线性。

| Responses 事件 | 内部事件 |
|---|---|
| `response.created` | `MessageStart` |
| `response.output_item.added` | `PartStart`（`item` 类型决定 `PartKind`；Opaque item 使用 `PartKind::Opaque(OpaqueKind)`） |
| `response.content_part.added` | `PartStart` |
| `response.output_text.delta` | `PartDelta::Text` |
| `response.refusal.delta` | `PartDelta::Text`（+ `Report`） |
| `response.reasoning_summary_text.delta` | `PartDelta::Thinking` |
| `response.function_call_arguments.delta` | `PartDelta::ToolArguments` |
| `response.output_text.done` / `*.done` | `PartStop` |
| `response.completed` | `UsagePatch` + `Finish` + `Termination::Explicit` |
| `response.incomplete` | `Finish{MaxTokens/ContentFilter/Other}` + `Report`（按 `incomplete_details.reason` 映射，**非成功**） |
| `response.failed` | `Event::Error` |
| `error` | `Event::Error`（信道 B） |
| 内置工具事件（`web_search_call.*` 等） | `PartStart` + `PartDelta::Opaque` + `PartStop` 透传 / 记 `Report` |

## 5. status 映射

| Responses `status` | IR `StopReason` |
|---|---|
| `completed` | `EndTurn`（若末 item 为 function_call 则 `ToolUse`） |
| `incomplete`（`max_output_tokens`） | `MaxTokens` |
| `incomplete`（`content_filter`） | `ContentFilter` |
| `incomplete`（其它 reason） | `Other(raw)` + `Report` |
| `cancelled` | `Cancelled` |
| `failed` | 错误，非 `Finish` |
| `queued` / `in_progress` | 中间态，不得映射为成功 |

## 6. 本协议专属陷阱

- **RESP-TRAP-1**：`status:"incomplete"` **不等于成功**；不能一律映射为 HTTP 200 成功。
- **RESP-TRAP-2**：`reasoning.encrypted_content` 不得解码、排序 map key、压缩空格或重序列化；无状态第二轮须原样回传（INV-1、INV-4）。
- **RESP-TRAP-3**：无 `[DONE]`，客户端按 `response.completed` 结束；不得伪造 `data: [DONE]`。
- **RESP-TRAP-4**：`function_call.arguments` 是**完整字符串**（与 Chat 分片不同）；但流式下仍按 `response.function_call_arguments.delta` 累积。
- **RESP-TRAP-5**：`output` item 的 `id` / `call_id` / `output_index` 是定位主键，转换时保留映射。
- **RESP-TRAP-6**：`instructions` 与 `previous_response_id` **不跨轮继承**；无状态模式须显式拒绝对应字段。
- **RESP-TRAP-7**：`include` 请求的字段（如 `web_search_call.action.sources`）可缺省，不得臆造。
- **RESP-TRAP-8**：`max_output_tokens` 最小值 16；越界应校验并报错，不静默裁剪。

## 7. 契约测试清单（本协议）

- [x] 文本非流式/流式往返
- [x] `function_call` 多轮：`call_id` 保真（golden）
- [x] `reasoning.encrypted_content` 字节保真
- [x] `status:completed` / `incomplete` / `cancelled` 三分支
- [x] `response.completed` 结束（无 `[DONE]`）
- [x] `output_index` / `content_index` 定位
- [x] 并行 function_call
- [x] `previous_response_id` 返回 `Unsupported`
- [x] 内置工具 item 透传 / 上报
- [x] `output_text` 便捷字段与 `output[]` 一致性
