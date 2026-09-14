# llmwire 项目设计

> 状态：冻结-稳定　|　版本：v1.0　|　最后更新：2026-09-14
> 权威：结构 / 架构 / API 的最高依据。字段级映射见 `spec/*`，决策缘由见 `decisions/*`。
> 本文**不**包含任何协议的字段表——那些只在 `spec/`。

---

## 0. 定位

**一句话**：`llmwire` 是一个双向、默认无状态、可嵌入任意 host 的 LLM wire protocol 转换核。host 把原始字节交给一个**请求级 `Converter` 对象**，它负责 SSE 分帧、解析、IR 转换，再序列化回目标协议的字节。

**是什么**

- 一个**字节级转换固件**：`&[u8]` 进，`&mut Vec<u8>` 出。
- 一个**请求级有界对象**：构造于请求到达，销毁于响应发完，绝不跨请求存活。
- 一个**诚实的转换器**：任何降级、丢弃、不可表达，都通过 `Report` 显式上报。
- 覆盖 **OpenAI Chat Completions / Anthropic Messages / OpenAI Responses** 三协议互转；M5 计划补图片输入（Gemini、图片输出推迟）。

**不是什么**

- 不是 HTTP 客户端、不建立连接、不绑定 `async` runtime。
- 不做鉴权、限流、重试、熔断、计费、会话存储、路由选渠道。
- 不做"provider 兼容层业务"（如渠道切换、模型前缀解析）。

**与 LiteLLM / New API 的分野**

| 议题 | LiteLLM / New API | llmwire |
|---|---|---|
| 定位 | 库 + Proxy（控制面与转换面耦合） | 纯转换核，控制面全外置 |
| canonical | OpenAI messages 即长期记忆格式 | 独立 IR + `Opaque` 逃生舱 |
| 不支持参数 | 静默丢弃（`drop_params`） | 显式 `Report` |
| thinking/加密块 | 部分路径丢失 | 字节级保真 + 显式策略 |
| 有状态部分 | 与转换逻辑相邻 | 严格隔离，无默认存储 |

## 1. 目标与非目标

**目标**

- **正确性优先**：宁可显式失败，不可静默降级。可被测试自证。
- **通用**：新增协议 = 新增一套 codec，不改 IR、不改其他 codec、不改门面。
- **轻量**：核心依赖仅 `serde` / `serde_json` / `thiserror` / `bitflags`。
- **可嵌入**：能跑在 serverless / edge / WASM；同步 core，异步由 host 包。

**非目标（刻意推迟）**

Gemini、`ResponseStore`（`store` / `previous_response_id` 服务端存储）、服务端内置工具、MCP 拍平、图片输出、音频/视频/文件管理、URL 下载与 Base64 上传托管、请求侧流式。图片输入按 M5 增量引入，不改变字节接缝。

**M5 图片输入边界**：核心只规范化 `RemoteUrl` / `Base64` 与协议包装，不下载远程图片、不上传 Base64、不托管 URL。需要跨资源模式的解析由 host 在调用 `Converter` 前完成；详见 `spec/IMAGE.md` 与 `decisions/0006-image-input-no-transport.md`。

## 2. 不可协商约束

- **INV-1 不伪造**：绝不生成假的 `signature`、`redacted_thinking`、thinking 块，绝不假装成功。
- **INV-2 不重算**：客户端传来的 `cache_control` 断点一律透传，SDK 不新增、不移动、不重算。
- **INV-3 不静默丢弃**：任何不支持的字段/能力都进 `Report`。`Strict` 下 `Fatal` 即 `Err`，`Converted` 下降级但**可见**。
- **INV-4 不落地明文**：`Opaque`（加密/签名块）在日志与 GUI 中**只记 `kind + len`**，不记内容；类型层面不提供 `Display`。
- **INV-5 不碰传输**：无 socket、无 HTTP、无 async runtime 依赖。
- **INV-6 无跨请求状态**：不持有任何跨请求数据；请求内状态随对象销毁丢弃。

## 3. 总体架构

```text
            入站字节                                出站字节
               │                                       ▲
               ▼                                       │
   ┌─────────────────────── Converter（请求级门面）───────────────────────┐
   │  分帧 → 事件 → codec(源) → IR → codec(目标) → 事件 → 分帧            │
   └──────┬──────────────────┬───────────────────┬──────────────┬────────┘
          │                  │                   │              │
      framing/            codec/src            ir/           codec/dst
     (SSE/NDJSON)      (object-safe)        (typed, 纯函数)   (object-safe)
```

**关键洞察**：一旦接缝切在**字节**侧，codec 的关联类型（`Self::Request` 等）不会穿过门面，于是**每个 codec 天然 object-safe**。`Converter` 只持有 `Box<dyn ProtocolCodec>` 的源/目标两个实例 —— **N 套 codec，不是 N²**。

分层职责：

| 层 | 负责 | 不负责 |
|---|---|---|
| `framing/` | 字节 → 完整帧；帧 → 字节（SSE 共享，NDJSON 预留） | 理解事件语义 |
| `codec/` | 帧 → `Vec<Event>`；`Event` → 帧；协议字段 ↔ IR | 分帧、路由、控制面 |
| `ir/` | 规范化类型与不变量；纯函数变换 | IO、状态、协议特有字段名 |
| `converter` | 请求级编排：持 FSM、上下文、`Report` | 协议细节、传输 |

## 4. 公开 API 契约

```rust
pub enum ProtocolId { Chat, Messages, Responses }

pub trait Converter: Send {
    /// 入站请求 → 出站请求。一次性，并据此记下流式模式。
    fn request(&mut self, body: &[u8], out: &mut Vec<u8>) -> Result<(), Error>;

    /// 非流式响应：入站响应体 → 出站响应体。
    fn response(&mut self, body: &[u8], out: &mut Vec<u8>) -> Result<(), Error>;

    /// 流式响应：喂入上游字节片段，追加目标协议字节。
    /// 返回空属正常（未拼出完整帧 / 该帧无输出）。
    fn feed(&mut self, chunk: &[u8], out: &mut Vec<u8>) -> Result<(), Error>;

    /// 上游结束：冲刷尾帧并判定终止原因。
    fn finish(&mut self, out: &mut Vec<u8>) -> Result<Termination, Error>;

    /// 取出并清空本次转换累积的质量报告。
    fn take_report(&mut self) -> Report;
}

/// 工厂：仅做 ProtocolId → codec 的构造映射（N 项），不做路由。
pub fn converter(
    src: ProtocolId,
    dst: ProtocolId,
    caps: Capabilities,
) -> Result<Box<dyn Converter>, Error>;
```

**契约要点**

- `out` 为 **append 语义**：SDK 只追加，不清空。调用方复用 buffer 前自行 `clear()`。
- `Converter: Send`，**不要求 `Sync`**（见 `decisions/0004`）。方法全 `&mut self`。
- 工厂只按协议对**构造**对象；"用哪对、何时调"由 host 决定。
- `caps` 由 host 通过 `Host` 端口提供（见 §5），决定策略而非协议细节。

### 4.1 能力策略

`Capabilities` 描述**入站协议 → 后端协议**这条路由允许采用的转换策略：

```rust
pub struct Capabilities {
    pub mode: Mode,
    pub thinking: ThinkingPolicy,
    pub tool_id: ToolIdPolicy,
    pub passthrough_cache_control: bool,
    pub passthrough_betas: bool,
    pub supported: ParamSet,
}

pub enum Mode { NativePassthrough, Converted, Strict }
pub enum ThinkingPolicy { Passthrough, Adapt, Strip, Reject }
pub enum ToolIdPolicy { Preserve }
```

```rust
bitflags! {
    pub struct ParamSet: u32 {
        const TEMPERATURE = 1 << 0;
        const TOP_P = 1 << 1;
        const TOP_K = 1 << 2;
        const MAX_OUTPUT_TOKENS = 1 << 3;
        const STOP = 1 << 4;
        const SEED = 1 << 5;
        const N = 1 << 6;
        const PRESENCE_PENALTY = 1 << 7;
        const FREQUENCY_PENALTY = 1 << 8;
        const REASONING = 1 << 9;
        const TOOLS = 1 << 10;
        const TOOL_CHOICE = 1 << 11;
        const CACHE_CONTROL = 1 << 12;
        const BETAS = 1 << 13;
    }
}
```

`resolve(inbound, backend, model)` 是目前唯一的路由级能力入口：

- `inbound == backend`：`Mode::NativePassthrough`；否则为 `Mode::Converted`。`Strict` 保留给 host 显式覆盖，阻断所有 `Fatal` 降级。
- `tool_id` 恒为 `ToolIdPolicy::Preserve`；任何未来策略都必须可逆，不得重生成 id。
- `thinking`：目标是 Messages / Responses 时为 `Passthrough`；目标是 Chat 时为 `Strip`。`Adapt` / `Reject` 保留给 host 的模型级覆盖。
- `passthrough_cache_control` 与 `passthrough_betas` 仅在目标为 Messages 时为 `true`。
- `supported: ParamSet` 声明目标协议可表达的 IR 参数集合。`model` 当前不细化该集合，后续由 `Host` 覆盖。
- `ParamSet` 未包含的已出现参数必须进入 `Report`，不得静默丢弃（INV-3）。

## 5. 模块边界与目录结构

```text
llmwire/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── ids.rs          # ProtocolId / OpaqueKind
    ├── error.rs        # Error（thiserror）
    ├── report.rs       # Report / Unmapped / Warning / Severity / UnmappedReason
    ├── caps.rs         # Capabilities / ThinkingPolicy / ToolIdPolicy / ParamSet(bitflags)
    ├── host.rs         # trait Host / StaticHost（能力表端口，不实现控制面）
    ├── framing/
    │   ├── mod.rs      # 帧读取/写入抽象
    │   ├── sse.rs      # SseFramer：按 \n\n 切帧、容忍注释/心跳/[DONE]
    │   └── ndjson.rs   # （Ollama，推迟）
    ├── ir/
    │   ├── mod.rs      # Conversation / Turn / Role
    │   ├── part.rs     # Part / Opaque
    │   ├── tool.rs     # ToolDef / ToolUse / ToolUseKind / ToolId / RawJson / ToolChoice
    │   ├── sampling.rs # Sampling / Reasoning
    │   ├── output.rs   # AssistantOutput / Choice / Finish / StopReason
    │   ├── usage.rs    # Usage
    │   ├── event.rs    # Event / Delta / PartKind
    │   └── state.rs    # StreamState / BlockState
    ├── codec/
    │   ├── mod.rs      # trait ProtocolCodec（object-safe）+ 工厂
    │   ├── chat/
    │   ├── messages/
    │   └── responses/
    └── converter.rs    # Converter 门面实现
```

**边界纪律**：`ir/` 只依赖 `serde` 等基础类型，**不**依赖 `codec/`；`codec/*` 之间**禁止**互相依赖（只能经 IR 通信）。

## 6. 数据流

### 6.1 非流式

```text
host: converter(src, dst, caps)
  → c.request(client_body, out)      // decode(src) → IR → encode(dst) → out 发上游
  → （host 发请求、收完整响应体 body）
  → c.response(body, out)            // decode(dst) → IR → encode(src) → out 发客户端
  → c.take_report()
```

### 6.2 流式

```text
c.request(client_body, out)          // 记下 stream=true 与请求上下文（tools/n/max_tokens…）
loop { chunk = upstream.recv().await
       c.feed(chunk, out)            // 分帧 → 事件 → IR → 目标事件 → 分帧；out 可能为空
       写 out 到客户端 }
c.finish(out)                        // 冲刷尾帧，返回 Termination
c.take_report()
```

### 6.3 错误双信道

- **信道 A —— `Result::Err`**：分帧错误、致命协议错误、`Strict` 下的 `Fatal`。发生在**尚未发出 200** 时。
- **信道 B —— 流内 error 事件**：HTTP 200 已发出后出现的错误，必须**编成目标协议的 error 事件写进 `out`**，不得返 `Err`。同时记入 `Report`。

两信道**不可混用**。详见 `spec/STREAMING.md`。

### 6.4 请求上下文保留

`Converter` 在一次请求内保留：解码后的 `Conversation`、`StreamState`、分帧缓冲、`Report`。用途：响应编码常需请求侧信息（Anthropic 必填 `max_tokens`、`n` 候选数、是否带 tools、`include_usage`）。对象销毁即全部丢弃（INV-6）。

## 7. 关键设计决策

| 决策 | ADR |
|---|---|
| 接缝切在字节侧（`&[u8]` ↔ `&mut Vec<u8>`） | `decisions/0001-bytes-seam.md` |
| 请求级有界对象，单次请求内管完流式/非流式 | `decisions/0002-request-level-object.md` |
| `Report` 升为一等 API，诚实上报 | `decisions/0003-report-first-class.md` |
| `Converter: Send` 而非 `Sync` | `decisions/0004-send-not-sync.md` |
| 独立 IR，不以 OpenAI 形状为长期语义 | `decisions/0005-independent-ir.md` |
| 图片输入只做表示转换，不下载/上传 | `decisions/0006-image-input-no-transport.md` |

## 8. 注意事项与陷阱

- **TRAP-1 分帧必须在 SDK 内闭合**：跨 chunk 的 JSON、注释行、心跳、`[DONE]`、`delta:null`、末包仍带 delta，全由 `framing/sse` 消化。见 `spec/STREAMING.md`。
- **TRAP-2 重序列化会破坏缓存前缀**：出站 JSON 由 SDK 拼装，key 顺序/空格一旦不稳定，上游 prefix cache 失效。故 `RawJson` 的字节权威必须贯穿**整个请求**（tools 定义、system block、tool arguments 皆然）。见 `spec/IR.md`。
- **TRAP-3 `Opaque` 不落地明文**（INV-4）：`Opaque` 只报 `kind + len`；类型上无 `Display`。
- **TRAP-4 tool id 必须字节保真**：`tool_use.id == tool_call_id` 恒定；不改大小写、不截断、不重随机化。见 `spec/IR.md`。
- **TRAP-5 usage 的 `None ≠ 0`**：未知填 `None`，不是 `0`；绝不编造 usage（INV-1）。
- **TRAP-6 `n>1` 不得静默退化为单候选**：IR 用 `Vec<Choice>` 表达；目标协议无法表达时上报 `Report`。
- **TRAP-7 pending 缓冲需设上限**：等不到完整帧时不得无限增长，超限转 `Error`。
- **TRAP-8 上游出口禁用缓冲**：SSE 场景 host 侧须关 gzip/Nginx buffering 并发保活注释；SDK 不负责，但文档明确告知 host。
- **TRAP-9 `finish` 终止判定不可颠倒**：`Explicit > CleanClose > ClientAbort > Timeout > NetworkError`；无终止符的关闭**不视为成功**。

## 9. 依赖与构建约束

```toml
[dependencies]
serde        = { version = "1", features = ["derive"] }
serde_json   = "1"
thiserror    = "2"
bitflags     = "2"

[dev-dependencies]
proptest     = "1"
insta        = "1"
```

- 禁止：`tokio` / `reqwest` / `async-trait` / `anyhow`。
- Rust `edition 2021`；建议 MSRV 与 stable 保持一个次版本内的保守取值。
- 未来可选：`futures::Stream` adapter 以 feature gate 提供，**默认关闭**且不得把 async 依赖带进核心。
- 代码不写注释（除非明确要求）。

## 10. 风险与未决问题

1. `tool_use.id` 透传的真实可行性：需对 New API / LiteLLM / 各家 Anthropic 兼容端点做 golden test；若某上游强校验 `call_` 前缀，需引入**可逆编码**（非重生成）。
2. Anthropic thinking 回传的强制程度：决定 `Strip` 策略是"温和降级"还是"硬失败"。需打真实 API 验证。
3. codex 是否使用 `previous_response_id`：决定 Responses 无状态模式是否够用。
4. 跨协议后 prefix cache 能否命中：本 SDK 一律透传断点、不重算，但不保证跨协议命中。
5. 第三方 Anthropic 兼容端点（DeepSeek / Kimi / GLM / vLLM）的 `/v1/messages` 支持度差异。
6. 图片输入的双支持程度可能随模型、端点和 API 版本变化；M5 用 capabilities 控制，不把 schema 支持等同于模型实际支持。

## 11. 文档地图

见 `README.md §2`。冲突裁决见 `README.md §4`。
