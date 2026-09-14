# TESTING.md — 测试策略

> 状态：半稳定　|　最后更新：2026-09-14
> 本文只讲**策略与结构**。具体用例挂在各 `spec/<protocol>.md §7` 的契约清单里，避免与实现脱节。

---

## 1. 为什么测试是本项目的灵魂

`llmwire` 的价值主张是"正确、诚实、可自证"。无状态 + 纯函数让它**天然可测**：

- 转换不依赖外部存储或时序 → **property-based 往返**可以写。
- 字节进 / 字节出 → **golden 与快照**可以精确断言。
- 所有降级都要上报 → **契约测试**可以验证"诚实"。

**测试不是附属，是设计的验证器。**（对应 `decisions/0002`、`decisions/0003`。）

## 2. 四类测试

### 2.1 Property-based 往返（核心）

工具：`proptest`。无状态 + 纯函数使能。

```rust
proptest! {
    #[test]
    fn roundtrip_chat(ir in arb_conversation()) {
        let req = Chat::encode_request(&ir, &caps(), &mut Report::new())?;
        let back = Chat::decode_request(req, &caps(), &mut Report::new())?;
        prop_assert_eq!(normalize(&ir), normalize(&back));
    }

    #[test]
    fn roundtrip_opaque(ir in arb_conversation_with_opaque()) {
        prop_assert!(opaque_blocks_equal(&ir, &back));   // byte-equal
    }
}
```

生成器必须覆盖的边界：空 content、多 part 并存、并行 tool_call、arguments 含转义与超长、thinking + tool_use 交织、`Opaque` 块、`n>1`、迟到 system。

### 2.2 Golden（字节级断言）

工具：普通 `#[test]`。用于**最容易翻车、翻车后最难归因**的点：

- `tool_result.tool_use_id == assistant.tool_use.id`（`tests/golden_tool_id.rs`）。
- `Opaque.bytes` / `RawJson.raw()` round-trip 后 byte-equal。
- 序列化 key 顺序稳定（prefix cache 相关，`DESIGN.md` TRAP-2）。

### 2.3 事件序列断言

当前使用普通 `#[test]` 对关键事件序列做精确断言，尚未启用 `insta` 快照，因此快照不作为现有验证证据。后续接入快照时，必须覆盖纯文本、thinking、单工具、并行工具、流中错误、中途断连与畸形输入容忍清单（`spec/STREAMING.md §6`）。

### 2.4 契约测试

工具：普通 `#[test]`。以各 `spec/*.md §7` 清单为准，是"协议文档的自动证据"。报告 `reference/LLM接口协议格式深度研究报告.md` 附录 A 的清单为总纲。

## 3. 目录约定

```text
tests/
├── boundaries.rs
├── protocol_matrix.rs
├── roundtrip_chat.rs
├── roundtrip_messages.rs
├── roundtrip_responses.rs
├── golden_tool_id.rs
├── streaming_chat.rs
├── streaming_messages.rs
├── streaming_responses.rs
├── live_cross_protocol.rs
├── converter.rs
└── contract/            # 契约测试
    ├── basic.rs
    ├── tools.rs
    ├── streaming.rs
    └── usage.rs
```

## 4. 契约清单索引（总纲）

摘自 `reference/LLM接口协议格式深度研究报告.md` 附录 A，按模块归位：

| 组 | 用例要点 |
|---|---|
| 基础生成 | 非流式/流式 chat、空/长 system、`max_output_tokens` 截断 |
| 多轮与上下文 | tool result 回传、`tool_call_id` 保真、迟到 system |
| 工具调用 | tool call / 并行 call、`arguments` 跨 chunk 拼接、未知函数 |
| 结构化输出与多模态 | JSON mode、image part（推迟项除外） |
| 流式边界与健壮性 | 首/末 chunk、重复 role、注释行、`[DONE]`、`delta:null`、断连 |
| 错误、限流与计费 | 402/429 区分、缺 usage、`include_usage`、`is_retryable` |

各协议专属用例以 `spec/{chat,messages,responses}.md §7` 为准。

## 5. 运行

```powershell
cargo test -p llmwire                          # 全部
cargo test -p llmwire --test roundtrip_chat    # 单个往返
cargo test -p llmwire --test boundaries      # P0 边界
cargo test -p llmwire --test protocol_matrix  # 3x3 矩阵
cargo test -p llmwire --test live_cross_protocol -- --ignored
cargo test -p llmwire --test golden_tool_id
```

## 6. 反模式（禁止）

- 断言"应该能转换"而不给确定期望值。
- 对 `arguments` 在 `PartStop`/`Finish` 之前 `parse`（违反 `STREAMING.md` STR-3）。
- 把流式的一次性网络错误当作"正常完成"（违反 `STREAMING.md §5`）。
- 测试里放行静默丢弃（违反 INV-3）——应断言 `Report` 有对应条目。

## 7. Live API 与兼容性矩阵

真实 API 测试是 dev-only harness；核心库仍遵守 INV-5，不引入 HTTP 客户端或 async runtime。所有 live tests 默认 `ignored`，普通 `cargo test` 不访问网络。

### 7.1 环境配置

从项目根目录读取 `.env`：

```powershell
Copy-Item .env.example .env
cargo test -p llmwire --test live_chat -- --ignored --nocapture
cargo test -p llmwire --test live_messages -- --ignored --nocapture
cargo test -p llmwire --test live_responses -- --ignored --nocapture
```

计划变量：

```dotenv
LLMWIRE_LIVE_CHAT_URL=https://api.openai.com/v1/chat/completions
LLMWIRE_LIVE_CHAT_API_KEY=
LLMWIRE_LIVE_CHAT_MODEL=

LLMWIRE_LIVE_MESSAGES_URL=https://api.anthropic.com/v1/messages
LLMWIRE_LIVE_MESSAGES_API_KEY=
LLMWIRE_LIVE_MESSAGES_MODEL=
LLMWIRE_LIVE_MESSAGES_VERSION=2023-06-01
LLMWIRE_LIVE_MESSAGES_BETA=


LLMWIRE_LIVE_RESPONSES_URL=https://api.openai.com/v1/responses
LLMWIRE_LIVE_RESPONSES_API_KEY=
LLMWIRE_LIVE_RESPONSES_MODEL=

LLMWIRE_LIVE_ENABLE_THINKING=0
```

`.env` 不进入版本控制；`.env.example` 只保存空 key 与默认端点。

### 7.2 覆盖分层

| 层 | 目标 | 覆盖 |
|---|---|---|
| Smoke | 每种协议最小请求可用 | 请求/响应可解码；usage 不伪造；错误清晰 |
| Compatibility | 真实上游语义兼容 | tool id 字节保真、thinking/signature、`cache_control`、多轮工具、流式终止 |
| Matrix | 多端点支持度记录 | 官方端点、本地网关、兼容端点的差异与已知限制 |

Live 测试只断言协议契约与不变量，不断言模型输出文本。网络错误、429、超时等属于 host 传输层，不得混入 codec 的正常完成语义。

### 7.3 当前验证矩阵

| 协议 | 能力 | 端点 | 状态 | 说明 |
|---|---|---|---|---|
| Chat | 非流式 | 本地网关 | verified | 请求/响应可解码 |
| Chat | 流式显式终止 | 本地网关 | verified | `data: [DONE]` 路径已执行 |
| Messages | 非流式 | 本地网关 | verified | 请求/响应可解码 |
| Messages | cache_control + usage | 本地网关 | verified | 真实请求已执行 |
| Messages | thinking/signature | 本地网关 | skipped | 当前 `LLMWIRE_LIVE_ENABLE_THINKING=0`，不得计为通过 |
| Responses | 非流式 | 本地网关 | verified | 请求/响应可解码 |
| Responses | function_call id | 本地网关 | verified | `call_id` 与 arguments 已校验 |
| Responses | 流式终止 | 本地网关 | verified | `response.completed` 路径已执行 |
| 全部协议 | 官方端点 | 官方 API | not-run | 当前矩阵仅覆盖本地网关 |
| 全部协议 | 跨协议 live | 本地网关 | verified | 6 个有向组合的文本非流式与流式均已执行 |

Responses 真实验证暴露了 `reasoning_text` 与 `encrypted_content` 共存的事件序列；实现已修正为同一 reasoning item 中分别保留明文 thinking 与加密 opaque。
