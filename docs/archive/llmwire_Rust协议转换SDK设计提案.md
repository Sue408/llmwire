# llmwire —— Rust 轻量级 LLM 协议转换 SDK 设计提案

> 版本：v0.1（设计稿）
> 目标：一个**双向、默认无状态、可嵌入任意 host** 的 LLM wire protocol 转换核
> 范围：OpenAI Chat Completions / OpenAI Responses / Anthropic Messages 三协议互转
> 非范围：HTTP 客户端、async runtime、鉴权、限流、重试、计费、会话存储

---

## 0. 设计立场（先看这个）

本提案建立在三个已经过实践验证的判断上，它们决定了后面所有取舍：

| 判断 | 依据 |
|---|---|
| **转换核必须无状态** | LiteLLM 的转换层（`AnthropicConfig`、流式 iterator）状态只在单次调用内；New API 的 `ConvertRequest/ConvertResponse` 是纯函数；主流反向代理多数不需要跨请求状态。三者的"有状态"都在控制面（key、配额、日志、渠道），而非转换逻辑 |
| **架构第一刀是路由，不是转换** | claude code 说 Anthropic、codex 说 Responses，接同协议后端时应走 **native passthrough**，连解析都不要解析。round-trip 一遍 IR 会磨掉 `anthropic-beta`、cache 断点等原生语义。New API 的建议原话是 "native first, convert second" |
| **不透明块由客户端回传，不由 SDK 托管** | Claude Code 的 trajectory 规则要求 thinking 块在 assistant trajectory 内必须保留。thinking 在 Anthropic 协议里是一等字段，不是需要 escrow 的外挂数据。自造 "thinking store" 是被明确列为反模式的做法 |

由此推出本 SDK 的三条不可协商约束：

1. **不伪造** signature / `redacted_thinking` / 假 thinking 块；
2. **不重算** 客户端传来的 `cache_control` 断点；
3. **不静默丢弃** 不支持的字段——显式上报，生产默认 strict 失败。

---

## 1. Crate 形态

### 1.1 依赖策略

```toml
[dependencies]
serde        = { version = "1", features = ["derive"] }
serde_json   = "1"
thiserror    = "2"
bitflags     = "2"      # Capabilities 参数集

[dev-dependencies]
proptest     = "1"
insta        = "1"      # 事件序列快照
```

**刻意不引入**：`tokio`、`reqwest`、`async-trait`、`anyhow`。

- 不绑定 async runtime：核心是 **`Iterator<Item = Result<Event, Error>>`**，host 想用 async 就自己包一层 `futures::Stream`（feature-gated adapter）。
- 不做 HTTP：SDK 只吃 wire bytes，`Vec<u8>` 进出。这让它可以跑在 serverless / edge / WASM。

### 1.2 模块布局

```
llmwire
├── wire/           # 三个原生 wire type，各自完整、互不裁剪
│   ├── chat/       #   OpenAI Chat Completions
│   ├── responses/  #   OpenAI Responses
│   └── messages/   #   Anthropic Messages
├── ir/             # 中间表示（本 SDK 的核心资产）
├── codec/          # Protocol trait + 各协议实现
├── stream/         # 流式 FSM 与事件集
├── caps/           # 能力矩阵与策略
├── report.rs       # Unmapped / Warning 显式上报
└── host.rs         # 控制面端口（只定义 trait，不实现）
```

**关键：三个 wire type 各自完整定义，不合并成一个"上帝 struct"。** 这是从 New API 学到的分层：`GeneralOpenAIRequest` + `ClaudeRequest` 并存是对的，错的是把 provider-only 字段塞进 canonical。wire type 用 `#[non_exhaustive]` 标记 enum，厂商加新值时不会破坏下游匹配。

---

## 2. IR 设计

### 2.1 为什么不用 OpenAI 形状直接当 IR

LiteLLM 的教训值得抄下来：

> 实现"任意 provider 互转"时，若把 OpenAI messages 当成唯一长期记忆格式，会丢失 Anthropic thinking、server tool、Bedrock trace 等信息。

但完全中立的 IR 成本高、且没必要兼容"理论上存在的第四种协议"。所以取中间路线：

> **IR 形状贴近 OpenAI（因为它是最低公共分母，且生态最大），但通过 `Opaque` 机制让任何非 OpenAI 语义都能无损搭载。**

### 2.2 核心类型

```rust
/// 一次逻辑请求的完整规范化表示
#[derive(Debug, Clone, Default)]
pub struct Conversation {
    pub system: Vec<Part>,          // 顶层 system（Claude/Gemini 形态）
    pub turns: Vec<Turn>,           // 对话体，role 只有 User / Assistant
    pub tools: Vec<ToolDef>,
    pub tool_choice: ToolChoice,    // 语义枚举，不是某家字段名
    pub sampling: Sampling,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role { User, Assistant }   // 没有 System / Tool

#[derive(Debug, Clone)]
pub struct Turn {
    pub role: Role,
    pub parts: Vec<Part>,
}

/// 内容最小单元。用 enum 而非 struct + Option 字段，
/// 让"text 同时带 tool_use"这类非法状态无法被构造。
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Part {
    Text(String),
    Image(ImageRef),
    ToolUse(ToolUse),
    ToolResult(ToolResult),
    Thinking(Thinking),
    /// 逃生舱：无法规范化的原始块。
    /// 语义：不得解读、不得改写、不得摘要，必须字节级回传。
    Opaque(Opaque),
}
```

三个容易被质疑、但必须这么定的选择：

**① content 恒为 block 数组（不是 string）。** 因为 Claude 的 assistant 回复可以是"一段文字 + 一个工具调用"，压成 string 会丢失顺序信息。原则：**IR 取最细粒度，升维无损、降维有损。**

**② role 只有两个。** OpenAI 的 `role:"tool"` 在 IR 里不占角色——工具结果是 **User turn 里的 `ToolResult` part**。这同时是 Claude（`tool_result` block）和 Gemini（`functionResponse` part）的形状。

**③ 合并规则固定（canonical form）。** OpenAI 里连续的 `tool` 消息 + 后续 `user` 消息，一律合并进**同一个 User turn**：

```
[user "查北京天气"]
[assistant tool_use(call_1)]
[tool result(call_1)]  +  [user "那上海呢"]   →  合并为一个 User turn 的两个 part
```

固定成一种形态，契约测试才有确定的期望值。

### 2.3 Opaque：让"不可表达"成为一等公民

```rust
#[derive(Debug, Clone)]
pub struct Opaque {
    /// 来源协议标记，用于反向时路由回正确的 wire 类型
    pub kind: OpaqueKind,
    /// 原始字节。不提供 as_str / 不实现 Display，
    /// 防止调用方误把它当可读内容处理。
    pub bytes: Box<[u8]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OpaqueKind {
    AnthropicRedactedThinking,
    AnthropicThinkingSignature,
    ResponsesEncryptedReasoning,
    GeminiThoughtSignature,
    ProviderSpecific(&'static str),
}
```

`redacted_thinking`、Responses `encrypted_content`、Gemini `thoughtSignature` 全部走这里。**IR 不需要理解它们，只需要不破坏它们。**

设计细节：`Opaque` **故意不实现 `Display`、不提供 `as_str()`**。这是用类型系统阻止误用——你没法不小心把加密 blob 打进日志或拼进 content。

### 2.4 ToolUse 与 RawJson：双表示 + 明确权威

```rust
/// newtype，防止 OpenAI id 与 Anthropic id 在函数签名里混用
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolId(pub Box<str>);

#[derive(Debug, Clone)]
pub struct ToolUse {
    pub id: ToolId,
    pub name: Box<str>,
    /// 权威表示：始终是原始 JSON 字节
    pub arguments: RawJson,
    /// 工具类型，见 §4.2
    pub kind: ToolUseKind,
}

/// OpenAI 侧是 JSON 字符串片段（流式时可能不闭合），
/// Claude/Gemini 侧是对象。两个都存，明确谁是 source of truth。
#[derive(Debug, Clone)]
pub struct RawJson {
    raw: Box<str>,
    parsed: std::sync::OnceLock<Option<serde_json::Value>>,
}

impl RawJson {
    pub fn raw(&self) -> &str { &self.raw }          // 字节级权威
    pub fn parsed(&self) -> Option<&serde_json::Value> {
        self.parsed.get_or_init(|| serde_json::from_str(&self.raw).ok()).as_ref()
    }
}
```

**`raw` 是权威，`parsed` 是惰性派生产物。** 理由：转回 OpenAI 时若走 `serde_json::to_string(&value)`，key 顺序、空格、转义都可能有细微差异——**prompt 前缀不匹配，上游缓存直接失效**。而流式中间态（`{"ci`）根本 parse 不出来，此时 `parsed()` 返回 `None` 是完全合法的状态。

### 2.5 ToolUseKind：LiteLLM 用 bug 换来的教训

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolUseKind {
    /// 客户端执行的普通函数（OpenAI function / Claude tool_use）
    Client,
    /// 服务端内置工具（Claude web_search / code_execution，id 前缀 srvtoolu_）
    Server,
    /// MCP 远程工具（Claude mcp__<server>__<tool>，Responses type:"mcp"）
    Remote { server: Option<Box<str>> },
}
```

这不是洁癖。LiteLLM PR #17746 就是因为把 `srvtoolu_` 前缀的**服务端工具**误判成普通 `tool_use`，导致 `web_search_tool_result` 丢失、多轮重建失败。issue #17254 则表现为流式下尾随 `{}`。

**只用一个 `function` 表达所有工具调用，反向翻译时必然丢信息。**

### 2.6 Usage：None 与 0 的区分是实质性的

```rust
#[derive(Debug, Clone, Copy, Default)]
pub struct Usage {
    /// None = 此刻未知；0 = 确实是零。两者语义不同。
    pub input:  Option<u64>,
    pub output: Option<u64>,
    /// input 的子集（OpenAI 语义）
    pub cached: u64,
    /// 独立项，不计入 input（Claude cache_creation）
    pub cache_creation: u64,
    /// output 的子集
    pub reasoning: u64,
}
```

为什么 `Option` 而不是 `u64`：Claude 在 `message_start` 就得给 `input_tokens`，而 OpenAI 流到末尾才有 usage。那一刻**信息不存在**，填 0 是撒谎，会污染计费和监控。

包含关系必须在 IR 规约里写死，绝不让 codec 自由发挥：

```
IR → Anthropic:  input_anthropic = input - cached
                 cache_read       = cached
                 cache_creation   = cache_creation
Anthropic → IR:  input = input_anthropic + cache_read + cache_creation
                 cached = cache_read
```

### 2.7 StopReason：语义枚举 + 原值并存

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StopReason {
    EndTurn, MaxTokens, StopSequence, ToolUse,
    ContentFilter, Pause, Cancelled,
    /// 无法归类：保留原值，绝不静默降级成 EndTurn
    Other(Box<str>),
}

pub struct Finish {
    pub canonical: StopReason,
    pub provider_raw: Box<str>,   // 双向可追溯
}
```

---

## 3. Codec：N 套实现，不是 N² 套

```rust
/// 协议编解码器。每个协议实现一次，即可与任意其他协议互转。
pub trait Protocol {
    type Request;
    type Response;
    type StreamEvent;

    const ID: ProtocolId;

    fn decode_request(req: Self::Request, caps: &Capabilities, rep: &mut Report)
        -> Result<Conversation, Error>;

    fn encode_request(ir: &Conversation, caps: &Capabilities, rep: &mut Report)
        -> Result<Self::Request, Error>;

    fn decode_response(res: Self::Response, caps: &Capabilities, rep: &mut Report)
        -> Result<AssistantOutput, Error>;

    fn encode_response(out: &AssistantOutput, caps: &Capabilities, rep: &mut Report)
        -> Result<Self::Response, Error>;

    fn decode_stream_event(ev: Self::StreamEvent, fsm: &mut StreamState, rep: &mut Report)
        -> Result<Vec<Event>, Error>;

    fn encode_stream_event(ev: &Event, fsm: &mut StreamState, rep: &mut Report)
        -> Result<Vec<Self::StreamEvent>, Error>;
}
```

**A → IR → B 意味着 3 个协议只需 3 套 codec，而不是 6 套两两映射。** 这正是 IR 存在的全部理由——LiteLLM 的 "OpenAI canonical 单向桥" 在接第二 canonical（Responses）时，被迫在 adapter 里再写一遍映射，就是因为缺少真正的 IR 层。

注意 `decode_stream_event` 返回 `Vec<Event>`：一个上游事件可能展开成多个内部事件（反之亦然），一对一是特例。

### 3.1 Translator：请求级实例

```rust
pub struct Translator<S: Protocol, D: Protocol> {
    caps: Capabilities,
    report: Report,                 // warnings / unmapped 累积，随结果冲刷
    stream: Option<StreamState>,    // 请求内的流式 FSM
    _pd: PhantomData<(S, D)>,
}

impl<S: Protocol, D: Protocol> Translator<S, D> {
    pub fn new(caps: Capabilities) -> Self { ... }

    pub fn request(&mut self, req: S::Request) -> Result<D::Request, Error> {
        let ir = S::decode_request(req, &self.caps, &mut self.report)?;
        D::encode_request(&ir, &self.caps, &mut self.report)
    }

    pub fn response(&mut self, res: S::Response) -> Result<D::Response, Error> { ... }

    /// 事件流转换器：纯 push，内部持有 block index / args buffer / usage 补丁
    pub fn stream<'a, I>(&'a mut self, events: I)
        -> impl Iterator<Item = Result<D::StreamEvent, Error>> + 'a
    where I: Iterator<Item = Result<S::StreamEvent, Error>> + 'a
    { ... }

    pub fn finish(self) -> Report { self.report }   // 冲刷报告
}
```

**生命周期严格 = 单次转换任务。** 构造于请求到达，销毁于响应发完。它持有的状态是**协议事件状态**（block index、arguments buffer、usage 补丁），连接一断整体丢弃——这是局部可丢弃状态，跟"会话状态"是本质不同的两件事。

**warnings 直接进实例状态，返回值保持干净的 `D::Request` 而不是包装类型。** 这是实例模型白送的简化。

---

## 4. 能力矩阵：取代 `drop_params`

New API 报告里那句"静默丢弃是双向协议的第一大反模式"值得当成设计信条。全局 `drop_params` 的问题在于：短期可用，长期导致能力退化且无法测试。

### 4.1 Capabilities

```rust
pub struct Capabilities {
    pub mode: Mode,
    pub thinking: ThinkingPolicy,
    pub tool_id: ToolIdPolicy,
    pub passthrough_cache_control: bool,
    pub passthrough_betas: bool,
    pub supported: ParamSet,          // bitflags，参数级能力声明
}

pub enum Mode {
    /// 同协议直连：完全不解析，零转换
    NativePassthrough,
    /// 跨协议转换，允许已声明的最佳努力降级
    Converted,
    /// 跨协议转换，遇到任何 unmapped 直接 Err（生产默认）
    Strict,
}

pub enum ThinkingPolicy {
    /// 上游是 Anthropic 原生：原样透传，不解读
    Passthrough,
    /// 上游非原生：按上游能力转成 reasoning 字段（如 reasoning_content）
    Adapt,
    /// 上游不支持：从请求中移除（仅此情况允许移除）
    Strip,
    /// 上游要求但无法提供：报错
    Reject,
}

pub enum ToolIdPolicy {
    /// 唯一当前推荐值
    Preserve,
}
```

**`ToolIdPolicy` 只有一个变体是刻意的。** 它记录了一个强约束：*上游 id 不得改大小写、截断或重新随机化*。留成 enum 是为了未来若真遇到 id 格式冲突（如某上游要求 `call_` 前缀），可以加一个**可逆编码**变体——而不是允许破坏性的重生成。

### 4.2 策略由路由决定，不由调用方猜

```rust
pub fn resolve(inbound: ProtocolId, backend: ProtocolId, model: &str) -> Capabilities {
    match (inbound, backend) {
        (A, B) if A == B => Capabilities { mode: Mode::NativePassthrough, .. },
        (ProtocolId::Anthropic, ProtocolId::AnthropicCompatible) => Capabilities {
            thinking: ThinkingPolicy::Passthrough,
            passthrough_cache_control: true,
            passthrough_betas: true,
            ..
        },
        (ProtocolId::Anthropic, _) => Capabilities {
            thinking: ThinkingPolicy::Strip,   // 或 Adapt，按上游能力
            passthrough_cache_control: false,
            passthrough_betas: false,
            ..
        },
        _ => todo!(),
    }
}
```

### 4.3 显式上报

```rust
pub struct Report {
    pub unmapped: Vec<Unmapped>,
    pub warnings: Vec<Warning>,
}

pub struct Unmapped {
    pub field: Box<str>,          // "sampling.top_k"
    pub reason: UnmappedReason,   // UnsupportedByTarget | NotRepresentable | PolicyBlocked
    pub severity: Severity,
}

pub enum Severity { Silent, Degraded, Fatal }
```

`Mode::Strict` 下任何 `Fatal` 直接返回 `Err(Error::UnmappedField)`。**宁可让调用方改代码，也不要让 `n>1` 静默变成"只返回 1 个候选"**——上层"选最优"的逻辑会静默退化，这种 bug 极难归因。

---

## 5. 流式：内部事件集 + 显式 FSM

### 5.1 七个内部事件

```rust
pub enum Event {
    MessageStart { id: Box<str>, model: Box<str> },
    /// block 边界。OpenAI 无此概念，由 codec 合成；Claude 原生具备
    PartStart { index: usize, kind: PartKind },
    PartDelta { index: usize, delta: Delta },
    PartStop  { index: usize },
    /// 增量补丁：字段为 None 表示"该维度尚未知"，可被后续补丁覆盖
    UsagePatch(UsagePatch),
    Finish(Finish),
    Error(Box<Error>),
}

pub enum Delta {
    Text(Box<str>),
    Thinking(Box<str>),
    /// JSON 字符串片段，可能是任意边界、可能不闭合、可能为空
    ToolArguments(Box<str>),
}
```

`PartStart` / `PartStop` 是**必需的**，不是可选优化。OpenAI 没有 block 边界概念，但 Claude 有；按"IR 站细粒度那侧"的原则，IR 保留边界，OpenAI 侧由 codec **合成**（收到首个非空 `delta.content` 时 start，`finish_reason` 时 stop）。合成比凭空恢复可靠。

### 5.2 状态机

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

三条硬规则，全部来自已验证的失败案例：

1. **只能在 `PartStart` 时创建 block**，不能在收到任意 delta 时临时创建。LiteLLM #17254 的尾随 `{}` 就是违反这条。
2. **并行工具调用靠 `index` 区分**，不靠字符串拼接结果反推。
3. **`arguments` 只做字符串累加，绝不在中途 `parse`。** 只有 `PartStop` 或 `Finish` 之后才允许解析。切割点可能落在 `\"` 转义符中间或 `{}` 中间。

### 5.3 usage 时序：承认不对称，不伪造

Claude 的 `message_start` 必须带 `input_tokens`，而 OpenAI 流到末尾才有 usage。这是**协议真实不对称，无解**。

处理：

- `message_start` 时刻 `input` 为 `None` → codec 输出 `0`，但**同时在 report 里记一条 `Degraded`，并在响应头/扩展字段标注 `usage.estimated = true`**。
- 流结束时发出 `UsagePatch` 补齐真实值。

绝不为了填满 `message_start` 而编造 input_tokens 且不标注——那会污染计费，且无从归因。

### 5.4 结束判定优先级

```rust
enum Termination {
    /// 1. 上游显式终止符：[DONE] / message_stop / response.completed / done:true
    Explicit,
    /// 2. 上游正常关闭且已收到 terminal finish
    CleanClose,
    /// 3. 客户端取消
    ClientAbort,
    /// 4. 超时
    Timeout,
    /// 5. 网络异常 —— 永远不伪造成功
    NetworkError,
}
```

连接关闭但无终止符 → 按 `ClientAbort` / `NetworkError` 处理，**不视为正常完成**。

解析器必须容忍：`: ` 注释行与心跳、首 chunk 无 `role`、tool call 中间 chunk 重复 `role`、末包仍带 `delta`、`delta: null`、空 JSON。

---

## 6. 控制面隔离

```rust
/// SDK 只依赖这些端口，不实现它们
pub trait Host {
    fn capabilities(&self, model: &str) -> Option<&Capabilities>;
    fn max_output_tokens(&self, model: &str) -> Option<u32>;
}

/// 默认实现：静态表 + panic on unknown。够用于单测与嵌入式场景
pub struct StaticHost { /* 内置模型能力表 */ }
```

**明确不属于本 SDK 的职责**（这些是 host / 网关 crate 的事）：HTTP 传输、鉴权与密钥轮换、限流与配额、重试与熔断、渠道切换、日志与计费、会话存储。

New API 的建议值得抄：

> 把 New API 的 control plane 反转成端口……这样可以在 serverless/edge 环境真正无状态部署。

### 6.1 唯一可能的有状态端口

```rust
/// 仅当实现 Responses 的 store / previous_response_id 时才需要。
/// 默认不启用；不提供默认实现，避免诱导使用。
pub trait ResponseStore {
    fn get(&self, id: &str) -> Result<Option<StoredResponse>, Error>;
    fn put(&self, resp: StoredResponse) -> Result<(), Error>;
}
```

启用条件（任一）：
1. 实现 Responses `store:true` / `previous_response_id`；
2. 代理主动接管历史（服务端 compaction）。

**若都不成立，内存 Map / 文件 / Redis 都不应是默认方案。** 代价不只是运维复杂度——代理的历史和客户端的历史会变成两套事实来源，任何不一致都可能产生 trajectory 错误。

注意：OpenRouter 的 Responses 文档声明其 API 无状态，`store:true` 或非 null `previous_response_id` 直接返回 400。这证明**无状态 Responses 网关是工业可行的**。

---

## 7. 典型路径

### 7.1 路由优先

```rust
pub enum Strategy { Passthrough, Translate }

pub fn route(inbound: ProtocolId, backend: ProtocolId) -> Strategy {
    if inbound == backend { Strategy::Passthrough } else { Strategy::Translate }
}
```

`Passthrough` 路径下，**连 `serde_json::from_slice` 都不要调用**——直接转发原始字节。claude code 会发送大量 Anthropic 特有语义（`anthropic-beta`、cache 断点、特定 tool 定义），解析再序列化是有损的。

### 7.2 claude code → OpenAI 兼容后端（转换路径，含工具调用多轮）

```
Turn 1  入站  POST /v1/messages  {messages:[user], tools, thinking:{enabled}}
        resolve(Anthropic, OpenAI) → ThinkingPolicy::Strip
        decode: system → Conversation.system；user → Turn{User}
        encode: → ChatRequest{max_completion_tokens（registry 补默认）, messages}
        出站响应 → decode → Finish{ToolUse}
        encode: tool_use.id 原样作为 tool_calls[].id（Preserve）
                arguments 用 RawJson::raw()，字节级还原

Turn 2  入站  [user, assistant(thinking+tool_use), user(tool_result)]
        strip thinking（上游非原生，允许）
        tool_result.tool_use_id → tool 消息 tool_call_id（同值）
        tool_use.id == tool_call_id 恒定成立 → 无需任何映射表

Turn N  同上，全程零跨请求状态
```

### 7.3 关键：thinking 回传为什么不需要存储

因为 `thinking` / `redacted_thinking` 在 Anthropic wire 协议里是 `content` 数组的**原生 block**，Claude Code 按自身 trajectory 规则会原样带回。SDK 的职责只有：

- 上游 Anthropic 原生 → `Opaque` / `Thinking` **原样透传**，不重排、不截断、不改写；
- 上游非原生 → 按 `ThinkingPolicy::Strip` 移除（且仅此情况允许移除）；
- **任何情况下都不生成假 signature。**

---

## 8. 测试策略

### 8.1 Property-based 往返稳定性（核心）

无状态 + 纯函数 ⇒ 可用 proptest：

```rust
proptest! {
    #[test]
    fn roundtrip_chat(ir in arb_conversation()) {
        let req = ChatProtocol::encode_request(&ir, &caps(), &mut Report::new()).unwrap();
        let back = ChatProtocol::decode_request(req, &caps(), &mut Report::new()).unwrap();
        prop_assert_eq!(normalize(&ir), normalize(&back));
    }

    #[test]
    fn roundtrip_anthropic(ir in arb_conversation()) { /* 同上 */ }

    #[test]
    fn roundtrip_opaque(ir in arb_conversation_with_opaque()) {
        // 加密 blob 必须 byte-equal
        prop_assert!(opaque_blocks_equal(&ir, &back));
    }
}
```

生成器要覆盖的边界：空 content、多 part 并存、并行 tool_call、arguments 含转义与超长、thinking + tool_use 交织、`Opaque` 块、`n>1`。

**这比手写契约测试强得多**——它能发现你没想到的组合。而它成立的前提就是无状态：转换若依赖外部存储或时序，property test 就写不了。

### 8.2 Golden Test：tool id 往返

```rust
#[test]
fn tool_id_survives_roundtrip() {
    // 断言：tool_result.tool_call_id == assistant.tool_calls[i].id
    // 这是最容易翻车、且翻车后症状极难归因的一处
}
```

必须自己验——**New API 主线中 OpenAI→Claude 的 `tool_use.id` 究竟是透传还是重映射，源码级未确认**；LiteLLM 主路径是否重造 id 也未见公开证据。透传是推荐做法且有间接证据，但不能假设。

### 8.3 事件序列快照（insta）

对每个协议的流式事件序列做快照测试，覆盖：纯文本、thinking、单工具、并行工具、流中错误、中途断连。

### 8.4 契约测试清单

复用前份报告的 45 条清单，其中必须优先覆盖：tool 参数跨 chunk 拼接、thinking 开/关、`max_output_tokens` 截断、`include_usage`、首个/末个 chunk、重复 role、注释行、缺失 usage、402/429/额度类区分。

---

## 9. 与 LiteLLM / New API 的关键分歧

这份提案**刻意不抄**它们的地方：

| 议题 | LiteLLM / New API | llmwire |
|---|---|---|
| 定位 | 库 + Proxy（控制面与转换面耦合） | **纯转换核**，控制面全外置为端口 |
| canonical | OpenAI messages 即长期记忆格式 | 独立 IR + `Opaque` 逃生舱 |
| 第二 canonical | Responses 需再写一遍映射 | 同一个 IR，第 4 个协议只需加 1 套 codec |
| 不支持参数 | 静默丢弃（`drop_params`） | 显式 `Unmapped`，生产默认 strict 失败 |
| thinking | 部分路径丢失 / 策略未明 | `Opaque` 字节级保真 + 四种显式策略 |
| 服务端工具 | 曾误判 `srvtoolu_`（#17746） | `ToolUseKind` 在类型层面区分 |
| 有状态部分 | 与转换逻辑相邻 | 严格隔离，`ResponseStore` 不提供默认实现 |

---

## 10. MVP 路线

| 阶段 | 交付 | 验收标准 |
|---|---|---|
| **M0** | `ir` 模块 + `Chat` codec（双向） | Chat→IR→Chat proptest 往返通过 |
| **M1** | `Messages` codec + 路由 + `Capabilities` | Chat ↔ Messages 双向；golden test 验 tool id |
| **M2** | 流式 FSM + 事件集 | 三种流式互转；事件序列快照通过；`[DONE]`/`message_stop` 正确 |
| **M3** | `Responses` codec（**仅无状态模式**） | `previous_response_id` 返回明确 `Unsupported` 而非伪造 |
| **M4** | `StaticHost` + 报告/观测 | 45 条契约测试全绿 |

**刻意推迟**：Gemini、`ResponseStore`、服务端工具、MCP 拍平、多模态。这些在 IR 稳定后再加，都是"加一套 codec"的工作量，不会返工。

---

## 11. 仍未解决 / 需要实测确认

诚实列出来，不假装设计能覆盖：

1. **`tool_use.id` 透传的真实可行性**：需对 New API / LiteLLM / 各家 Anthropic 兼容端点做 golden test。若某上游强校验 `call_` 前缀，需引入**可逆编码**（非重生成）。
2. **Anthropic thinking 回传的强制程度**：不同模型 / beta 版本下"缺 thinking 是否真 400"需要打真实 API 验证。这决定 `Strip` 策略是"温和降级"还是"硬失败"。
3. **codex 是否使用 `previous_response_id`**：需抓包确认。这决定 M3 是否要提前。
4. **缓存断点的跨协议语义**：各家 prefix cache 的 key、TTL、计费都不同，本 SDK 一律**透传客户端断点、不重算**，但这不保证跨协议后仍能命中。
5. **第三方 Anthropic 兼容端点的能力边界**：DeepSeek / Kimi / GLM / vLLM 等提供的 `/v1/messages` 支持度需逐家实测。
