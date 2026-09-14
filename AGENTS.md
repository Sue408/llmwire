# AGENTS.md — llmwire 工程入口

> 本文件是**所有 agent / 协作者的第一入口**。先读完本节，再按"文档地图"决定读哪份，不要一上来读全部。

## 项目一句话

`llmwire` 是一个**双向、默认无状态、可嵌入任意 host 的 LLM wire protocol 转换核**：host 把原始字节交给一个**请求级 `Converter` 对象**，它负责 SSE 分帧、解析、IR 转换，再序列化回目标协议的字节。

当前阶段：**文档设计阶段**（仓库内暂无 Rust 代码）。正式实现从 M0 开始，见 `docs/PLAN.md`。

## 状态与约定

- 语言：文档用简体中文；代码标识符、commit message 用英文。
- 代码规范：**不写注释**（除非明确要求）；Rust `edition 2021`。
- 依赖红线：核心**不得**引入 `tokio` / `reqwest` / `async-trait` / `anyhow`。
  - 允许：`serde`、`serde_json`、`thiserror`、`bitflags`。
  - dev：`proptest`、`insta`。
- 提交：仅在明确指示时提交。提交信息用 `type: 简述`（如 `docs: …`、`feat(codec): …`）。

## 常用命令（实现开始后生效）

```powershell
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test -p llmwire
cargo test -p llmwire --test roundtrip      # proptest 往返
cargo insta review                           # 流式事件快照
```

## 文档地图（按需加载，不要全读）

| 文件 | 状态 | 什么时候读 |
|---|---|---|
| `docs/README.md` | 活跃 | 想看文档全貌 / 写作规范 / 权威优先级 |
| `docs/DESIGN.md` | 冻结-稳定 | 定位、架构、模块边界、公开 API、不变量、陷阱 |
| `docs/spec/IR.md` | 半稳定 | 动 IR 类型、做任意 codec 前**必读** |
| `docs/spec/STREAMING.md` | 半稳定 | 动流式（`feed`/`finish`/FSM）前**必读** |
| `docs/spec/chat.md` | 半稳定 | 实现/修改 OpenAI Chat codec |
| `docs/spec/messages.md` | 半稳定 | 实现/修改 Anthropic Messages codec |
| `docs/spec/responses.md` | 半稳定 | 实现/修改 OpenAI Responses codec |
| `docs/PLAN.md` | 活跃 | 接任务 / 看进度 / 找验收标准 |
| `docs/TESTING.md` | 半稳定 | 写测试、补契约用例 |
| `docs/decisions/*.md` | 冻结 | 想知道"为什么这么定"，或要推翻某决策 |
| `docs/reference/*` | 冻结 | 查协议一手细节的证据（不回改成规范） |
| `docs/archive/*` | 已废弃 | **仅**用于追溯历史，不权威，不要据此实现 |

**实现任一 codec 的最小阅读集**：`spec/IR.md` + `spec/STREAMING.md` + 对应的 `spec/<protocol>.md`。

## 权威优先级（冲突时按此裁决）

1. 结构 / 架构 / API → `docs/DESIGN.md`
2. 字段映射 / 语义 → `docs/spec/*.md`
3. 决策缘由 → `docs/decisions/*.md`
4. 协议一手证据 → `docs/reference/*`
5. 历史提案 → `docs/archive/*`（**最低**，可能已过时）

若发现文档冲突：不要猜。按上述优先级执行，并在 `docs/README.md` 的"已知冲突"记录一条。

## 不可协商约束（速览，详见 DESIGN.md §2）

- **INV-1** 不伪造 `signature` / `redacted_thinking` / 假 thinking 块。
- **INV-2** 不重算客户端传来的 `cache_control` 断点。
- **INV-3** 不静默丢弃不支持的字段——显式上报 `Report`。
- **INV-4** `Opaque`（加密/签名块）不落地明文：日志与 GUI 只记 `kind + len`。
- **INV-5** 不碰传输层（无 socket / HTTP / async runtime 绑定）。
- **INV-6** 无跨请求状态；请求内状态随对象销毁而丢弃。
