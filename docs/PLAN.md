# PLAN.md — 里程碑与任务

> 状态：活跃　|　最后更新：2026-09-14
> 本文件是接任务 / 看进度 / 找验收标准的**唯一入口**。任务块的字段含义：
> `文档锚点`（先读哪几节）、`要动`（预计改的文件）、`不变量`（必须满足的 INV）、`AC`（验收标准）、`验证`（可运行的命令）。

---

## 进度总览

| 里程碑 | 目标 | 状态 |
|---|---|---|
| **M0** | crate 骨架 + `ir` + `chat` codec（非流式双向） | M0-3 完成 |
| **M1** | `messages` codec + `caps`/`resolve`（非流式双向） | M1-3 完成 |
| **M2** | `framing` + 流式 FSM + `Converter` 门面（流式双向） | M2-5 完成 |
| **M3** | `responses` codec（仅无状态）+ 流内 error 事件 | M3-3 完成 |
| **M4** | `StaticHost` + `Report` 全链路 + 契约测试 | M4-4 完成 |

## 刻意推迟

Gemini codec、`ResponseStore`（`store` / `previous_response_id`）、服务端内置工具、MCP 拍平、多模态输入输出、请求侧流式、`futures::Stream` adapter。理由见 `DESIGN.md §1`。

---

## M0 — 骨架与 Chat 非流式

### M0-1 crate 骨架与依赖红线
- 文档锚点: `DESIGN.md §5、§9`
- 要动: `Cargo.toml`, `src/lib.rs`, `src/ids.rs`, `src/error.rs`
- 不变量: INV-5
- AC: `cargo clippy --all-targets -- -D warnings` 通过；依赖树仅 `serde`/`serde_json`/`thiserror`/`bitflags`
- 验证: `cargo tree -p llmwire`

### M0-2 IR 核心类型
- 文档锚点: `spec/IR.md §2–§8`
- 要动: `src/ir/{mod,part,tool,sampling,output,usage}.rs`
- 不变量: IR-INV-OPAQUE-1、IR-INV-OPAQUE-2、IR-INV-RAW-1、IR-INV-TOOL-1、IR-INV-USAGE-1
- AC: 类型可构造；`Opaque` 无 `Display`/`as_str()`（编译期断言）
- 验证: `cargo test -p llmwire`

### M0-3 Chat codec 非流式双向
- 文档锚点: `spec/chat.md §2、§3、§5`，`spec/IR.md §4`
- 要动: `src/codec/mod.rs`, `src/codec/chat/*`
- 不变量: CF-1、CF-2、IR-INV-RAW-1
- AC: `Chat → IR → Chat` proptest 往返通过；`RawJson` byte-equal
- 验证: `cargo test -p llmwire --test roundtrip_chat`

---

## M1 — Messages codec 与能力矩阵

### M1-1 Capabilities 与 resolve
- 文档锚点: `DESIGN.md §4、§5`
- 要动: `src/caps.rs`
- 不变量: INV-3
- AC: `resolve()` 覆盖 `(Chat|Messages|Responses)²` 全部方向，无 `todo!()`
- 验证: `cargo test -p llmwire caps`

### M1-2 Messages codec 非流式双向
- 文档锚点: `spec/messages.md §2、§3、§5`
- 要动: `src/codec/messages/*`
- 不变量: INV-1、INV-2、IR-INV-TOOL-1
- AC: `Chat ↔ Messages` 非流式往返；`cache_control` 原样
- 验证: `cargo test -p llmwire --test roundtrip_messages`

### M1-3 golden：tool id 往返
- 文档锚点: `spec/IR.md §5.2`
- 要动: `tests/golden_tool_id.rs`
- 不变量: IR-INV-TOOL-2
- AC: `tool_result.tool_use_id == assistant.tool_use.id`，且 id 字节不变
- 验证: `cargo test -p llmwire --test golden_tool_id`

---

## M2 — 流式

### M2-1 framing/sse
- 文档锚点: `spec/STREAMING.md §4、§6`
- 要动: `src/framing/{mod,sse}.rs`
- 不变量: STR-4
- AC: 半帧不 panic；注释行/心跳/无 payload event/多行 data 容忍；缓冲超限报 `Error`
- 验证: `cargo test -p llmwire framing`

### M2-2 事件与 FSM
- 文档锚点: `spec/STREAMING.md §2、§3`
- 要动: `src/ir/{event,state}.rs`
- 不变量: STR-1、STR-2、STR-3、STR-7
- AC: block 只在 `PartStart` 创建；`arguments` 仅在 stop/finish 后解析；Opaque delta 只能追加到已开启 block
- 验证: `cargo test -p llmwire state`

### M2-3 Chat 流式
- 文档锚点: `spec/chat.md §4`
- 要动: `src/codec/chat/stream.rs`
- 不变量: INV-3、STR-1
- AC: 纯文本 / 单工具 / 并行工具流式往返；事件序列快照通过
- 验证: `cargo test -p llmwire --test streaming_chat`

### M2-4 Messages 流式
- 文档锚点: `spec/messages.md §4`
- 要动: `src/codec/messages/stream.rs`
- 不变量: INV-1、INV-2、STR-5
- AC: `Chat ↔ Messages` 流式往返；`message_start`/`message_delta` usage 合并
- 验证: `cargo test -p llmwire --test streaming_messages`

### M2-5 Converter 门面
- 文档锚点: `DESIGN.md §4、§6`，`spec/STREAMING.md §4、§8`
- 要动: `src/converter.rs`, `src/ids.rs`（`ProtocolId`）
- 不变量: INV-6、STR-6
- AC: 门面 API 全通；错误双信道不混用；`take_report()` 有内容
- 验证: `cargo test -p llmwire --test converter`

---

## M3 — Responses（无状态）

### M3-1 Responses 非流式
- 文档锚点: `spec/responses.md §2、§3、§5`
- 要动: `src/codec/responses/*`
- 不变量: INV-1、INV-4
- AC: 文本 + function_call 非流式往返；`encrypted_content` 字节保真
- 验证: `cargo test -p llmwire --test roundtrip_responses`

### M3-2 Responses 流式
- 文档锚点: `spec/responses.md §4`
- 要动: `src/codec/responses/stream.rs`
- 不变量: STR-1、RESP-TRAP-3
- AC: `response.completed` 结束；无 `[DONE]`；事件快照通过
- 验证: `cargo test -p llmwire --test streaming_responses`

### M3-3 无状态拒绝
- 文档锚点: `spec/responses.md §6`
- 要动: `src/codec/responses/*`
- 不变量: INV-3
- AC: `previous_response_id` / `store:true` 返回明确 `Unsupported`，不伪造
- 验证: `cargo test -p llmwire responses_unsupported`

---

## M4 — 收口

### M4-1 StaticHost
- 文档锚点: `DESIGN.md §5`
- 要动: `src/host.rs`
- 不变量: INV-6
- AC: 静态能力表可用；unknown 模型行为明确
- 验证: `cargo test -p llmwire host`

### M4-2 Report 全链路
- 文档锚点: `DESIGN.md §8`，各 `spec/*.md §6`
- 要动: `src/report.rs` + 各 codec 上报点
- 不变量: INV-3、INV-4
- AC: 每个 `Unmapped`/`Warning` 带 `severity` + 来源路径；`Opaque` 只记 `kind + len`
- 验证: `cargo test -p llmwire report`

### M4-3 契约测试
- 文档锚点: `TESTING.md`，各 `spec/*.md §7`
- 要动: `tests/contract/*`
- 不变量: 全部
- AC: 契约清单全绿
- 验证: `cargo test -p llmwire`

### M4-4 Live compatibility matrix
- 文档锚点: `TESTING.md §7`，`DESIGN.md §10`
- 要动: `tests/live_*.rs`, `.env.example`, `TESTING.md`
- 不变量: INV-3、INV-4、INV-5
- AC: 默认全 `ignored`；缺配置不访问网络；Chat / Messages / Responses 各有 smoke；覆盖 tool id、thinking、cache_control、usage 与流式终止；核心依赖不新增 HTTP/async runtime
- 验证: `cargo test -p llmwire --test live_chat -- --ignored --nocapture` 及对应 live tests

---

## 完成定义（DoD）

一个任务完成，必须同时满足：
1. 对应 `AC` 全绿；
2. `cargo fmt --all` 与 `cargo clippy --all-targets -- -D warnings` 无告警；
3. 新增行为有对应测试（proptest / insta / golden / contract 之一）；
4. 若改变了 IR 或流式语义，同步更新 `spec/*` 并在 `README.md` 登记（若涉冲突）。
