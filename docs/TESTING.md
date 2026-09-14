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

### 2.3 事件序列快照

工具：`insta`。对每个协议的流式事件序列做快照，覆盖：纯文本、thinking、单工具、并行工具、流中错误、中途断连、畸形输入容忍清单（`spec/STREAMING.md §6`）。

```powershell
cargo insta review
```

### 2.4 契约测试

工具：普通 `#[test]`。以各 `spec/*.md §7` 清单为准，是"协议文档的自动证据"。报告 `reference/LLM接口协议格式深度研究报告.md` 附录 A 的清单为总纲。

## 3. 目录约定

```text
tests/
├── roundtrip_chat.rs
├── roundtrip_messages.rs
├── roundtrip_responses.rs
├── golden_tool_id.rs
├── streaming_chat.rs
├── streaming_messages.rs
├── streaming_responses.rs
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
cargo test -p llmwire --test golden_tool_id
cargo insta review                              # 快照审阅
```

## 6. 反模式（禁止）

- 断言"应该能转换"而不给确定期望值。
- 对 `arguments` 在 `PartStop`/`Finish` 之前 `parse`（违反 `STREAMING.md` STR-3）。
- 把流式的一次性网络错误当作"正常完成"（违反 `STREAMING.md §5`）。
- 测试里放行静默丢弃（违反 INV-3）——应断言 `Report` 有对应条目。

## 7. Live API smoke

M0-3 的真 API 冒烟测试默认标记为 `ignored`，配置从项目根目录的 `.env` 读取。

```powershell
Copy-Item .env.example .env
cargo test -p llmwire --test live_chat -- --ignored --nocapture
```

`.env` 至少需要：

```dotenv
LLMWIRE_LIVE_CHAT_URL=https://api.openai.com/v1/chat/completions
LLMWIRE_LIVE_CHAT_API_KEY=...
LLMWIRE_LIVE_CHAT_MODEL=...
```

`.env` 不进入版本控制；`.env.example` 仅保存空 key 和默认端点。
