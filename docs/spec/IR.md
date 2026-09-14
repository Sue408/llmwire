# spec/IR.md — 中间表示规范

> 状态：半稳定　|　最后更新：2026-09-14
> 读者：所有 codec 实现者。**做任意 codec 前必读。**
> 本文定义 IR 这门"内部语言"。协议侧的字段名与结构不在本文，见 `chat.md` / `messages.md` / `responses.md`。

---

## 1. 设计立场

**IR 的价值不在"表达共同点"，而在"优雅地容纳不同点"。**

共同的文本/图片好办，真正的功夫全在差异点（系统消息位置、工具 ID、tool_choice 语义、reasoning 映射、流式事件模型）。因此：

- 形状贴近 OpenAI（它是最大生态与最低公共分母），但**不是** OpenAI 形状的直接复用。
- 任何非 OpenAI 语义通过 `Opaque` 字节级搭载，IR 不理解它，只需不破坏它。
- 原则：**IR 取最细粒度**。升维无损、降维有损；降维时必须上报 `Report`。

## 2. 核心类型

```rust
#[derive(Debug, Clone, Default)]
pub struct Conversation {
    pub system: Vec<Part>,        // 顶层 system（Claude/Gemini/Responses instructions 形态）
    pub turns: Vec<Turn>,         // 对话体，role 只有 User / Assistant
    pub tools: Vec<ToolDef>,
    pub tool_choice: ToolChoice,  // 语义枚举，不是某家字段名
    pub sampling: Sampling,
    pub reasoning: Reasoning,     // 独立字段，不塞 sampling
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role { User, Assistant }   // 没有 System / Tool

#[derive(Debug, Clone)]
pub struct Turn {
    pub role: Role,
    pub parts: Vec<Part>,
}
```

### 2.1 为什么 role 只有两个

OpenAI 的 `role:"tool"` 在 IR 里**不占角色**：工具结果是 **User turn 里的 `ToolResult` part**。这同时是 Claude（`tool_result` block）与 Gemini（`functionResponse` part）的形状。OpenAI 的 `role:"developer"` 与迟到的 `role:"system"` 一律提升进顶层 `system`（见 §4）。

### 2.2 content 恒为 block 数组

`Turn.parts` 永远是数组。因为 Claude 的 assistant 回复可以是"一段文字 + 一个工具调用"，压成 string 会丢顺序。

## 3. Part

```rust
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

#[derive(Debug, Clone)]
pub struct ImageRef {
    pub source: ImageSource,
    pub detail: Option<Box<str>>,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ImageSource {
    RemoteUrl(Box<str>),
    Base64 {
        media_type: Box<str>,
        data: Box<str>,
    },
}

#[derive(Debug, Clone)]
pub struct Thinking {
    pub text: String,
    pub signature: Option<Opaque>,
}
```

`Thinking` 用于 Anthropic 明文 thinking（`{ text, signature: Option<Opaque> }`）；`redacted_thinking`、Responses `encrypted_content`、Gemini `thoughtSignature` 全部走 `Opaque`。

### 3.1 Opaque

```rust
#[derive(Clone)]
pub struct Opaque {
    pub kind: OpaqueKind,      // 来源协议标记，反向时路由回正确的 wire 类型
    pub bytes: Box<[u8]>,      // 原始字节
}

impl std::fmt::Debug for Opaque {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Opaque")
            .field("kind", &self.kind)
            .field("len", &self.bytes.len())
            .finish()
    }
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

**IR-INV-OPAQUE-1**：`Opaque` **不实现 `Display`、不提供 `as_str()`**。用类型系统阻止误用。
**IR-INV-OPAQUE-2**：`Opaque` 的 `Debug`、日志与 GUI 只以 `kind + len` 呈现，不得输出 `bytes`（对应 `DESIGN.md` INV-4）。
**IR-INV-OPAQUE-3**：round-trip 后 `bytes` 必须 byte-equal。

### 3.2 ImageRef

`ImageRef` 用于图片输入，类型定义见 §3；它必须区分**远程资源**和**内联字节**：

- `RemoteUrl` 只表示目标服务端需要自行获取的 `http://` / `https://` URL，字符串 byte-equal。
- `Base64.data` 只保存纯 payload，不含 `data:<media_type>;base64,` 前缀。
- `detail` 保留 OpenAI 图片提示原始值；目标协议无法表达时记 `Report`。
- `data:` URI 在 codec 边界解析为 `Base64`，反向编码时再合成。
- 核心不下载 URL、不上传 Base64、不生成托管 URL；详细规则见 `IMAGE.md`。

**IR-INV-IMG-1**：`RemoteUrl` 不解析、不改写、不重排 URL 字符串。
**IR-INV-IMG-2**：`Base64.data` 不包含 data URI 前缀，`media_type` 独立保存。
**IR-INV-IMG-3**：图片转换核心不碰传输层，不主动获取或托管资源。
**IR-INV-IMG-4**：无法表达的图片字段/表示必须进 `Report`。

## 4. Canonical form（合并规则）

固定成一种形态，契约测试才有确定期望值。

- **CF-1**：OpenAI 中连续的 `tool` 消息与后续 `user` 消息，合并进**同一个 User turn**，成为多个 part。

  ```text
  [user "查北京天气"]
  [assistant tool_use(call_1)]
  [tool result(call_1)] + [user "那上海呢"]   →   一个 User turn 的两个 part
  ```

- **CF-2**：所有 `system` / `developer` 消息提升为顶层 `Conversation.system`。
- **CF-3**：迟到的 system（出现在对话中途）按 CF-2 提升，并记 `Report` 一条 `Degraded`（位置信息丢失）。
- **CF-4**：`ToolResult` part 必须紧邻其 `ToolUse` 所在的 assistant turn 之后（由 CF-1 保证）。

## 5. 工具类型

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolId(pub Box<str>);   // newtype，防止不同协议的 id 在签名里混用

#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: Box<str>,
    pub description: Option<Box<str>>,
    pub parameters: RawJson,       // JSON Schema，字节权威
    pub strict: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct ToolUse {
    pub id: ToolId,
    pub name: Box<str>,
    pub arguments: RawJson,        // 权威表示：始终是原始 JSON 字节
    pub kind: ToolUseKind,
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub tool_use_id: ToolId,
    pub content: ToolResultContent,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ToolResultContent {
    Text(Box<str>),                // OpenAI tool message string
    Parts(Vec<Part>),              // Anthropic tool_result 可为 block 数组
    Object(RawJson),               // Gemini functionResponse.response 为对象
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolUseKind {
    Client,                            // 客户端执行（OpenAI function / Claude tool_use）
    Server,                            // 服务端内置（Claude srvtoolu_ / web_search…）
    Remote { server: Option<Box<str>> }, // MCP 远程（mcp__server__tool）
}

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub enum ToolChoice {
    #[default]
    Auto, Required, None,
    Named(Box<str>),
    /// 无法归类：保留原值，绝不静默降级
    Other(RawJson),
}
```

### 5.1 RawJson —— 双表示，明确权威

```rust
#[derive(Debug, Clone)]
pub struct RawJson {
    raw: Box<str>,
    parsed: std::sync::OnceLock<Option<serde_json::Value>>,
}

impl RawJson {
    pub fn from_raw(raw: impl Into<Box<str>>) -> Self {
        Self { raw: raw.into(), parsed: std::sync::OnceLock::new() }
    }

    pub fn raw(&self) -> &str { &self.raw }              // 字节级权威
    pub fn parsed(&self) -> Option<&serde_json::Value> { // 惰性派生
        self.parsed.get_or_init(|| serde_json::from_str(&self.raw).ok()).as_ref()
    }
}
```

**IR-INV-RAW-1**：`raw` 是权威，`parsed` 仅为惰性派生。转回 OpenAI 时用 `raw()`，**禁止** `serde_json::to_string(parsed)` 反序列化回写（key 顺序/转义差异会破坏 prefix cache）。
**IR-INV-RAW-2**：`parsed()` 返回 `None` 是合法状态——流式中间态（`{"ci`）本就不可解析。

### 5.2 ToolUseKind 是不可省的

LiteLLM PR #17746 因把 `srvtoolu_` 服务端工具误判为普通 `tool_use`，导致 `web_search_tool_result` 丢失、多轮重建失败。**只用一个 `function` 表达所有工具调用，反向翻译必然丢信息。**

**IR-INV-TOOL-1**：`tool_use.id` 必须**字节保真**，不得改大小写、截断、重随机化（`ToolIdPolicy::Preserve`）。
**IR-INV-TOOL-2**：`tool_result.tool_use_id == assistant.tool_use.id` 恒定成立，无需映射表。

## 6. 采样与推理

```rust
#[derive(Debug, Clone, Default)]
pub struct Sampling {
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,          // Claude/Gemini 有，OpenAI Chat 无
    pub max_output_tokens: Option<u32>,
    pub stop: Vec<Box<str>>,         // 统一为数组；OpenAI 单值升维
    pub seed: Option<i64>,
    pub n: Option<u32>,              // 多候选；Claude 无
    pub presence_penalty: Option<f32>,
    pub frequency_penalty: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReasoningEffort { Minimal, Low, Medium, High, None }

#[derive(Debug, Clone, Default)]
pub struct Reasoning {
    pub enabled: bool,
    pub effort: Option<ReasoningEffort>,  // low / medium / high / minimal / none
    pub budget_tokens: Option<u64>,       // Anthropic thinking.budget_tokens
}
```

**映射约定**（细节见各协议 spec）：`reasoning_effort` ⇄ `reasoning.effort` ⇄ `thinking.budget_tokens`。语义枚举无法一一对应时，按目标能力**降级并上报**（`Report`），不得臆造数值。

## 7. 输出

```rust
#[derive(Debug, Clone, Default)]
pub struct AssistantOutput {
    pub id: Option<Box<str>>,        // 供应商响应 ID；None = 未上报
    pub model: Option<Box<str>>,     // target 实际上报的模型名；None = 未上报
    pub choices: Vec<Choice>,        // n / candidateCount
    pub usage: Usage,
}

#[derive(Debug, Clone, Default)]
pub struct Choice {
    pub index: u32,
    pub parts: Vec<Part>,            // 文本 / ToolUse / Thinking / Opaque
    pub finish: Finish,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum StopReason {
    #[default]
    EndTurn, MaxTokens, StopSequence, ToolUse,
    ContentFilter, Pause, Cancelled,
    Other(Box<str>),                 // 无法归类保留原值，绝不降级成 EndTurn
}

#[derive(Debug, Clone, Default)]
pub struct Finish {
    pub canonical: StopReason,
    pub provider_raw: Box<str>,      // 双向可追溯
}
```

**IR-INV-N-1**：`AssistantOutput.choices.len()` 必须反映请求的 `n`；目标协议无法表达多候选时，**上报 `Report`**，不得静默返回单个（`DESIGN.md` TRAP-6）。

**IR-INV-META-1**：响应 `id` / `model` 必须来自 target 上报值。源协议编码 target 未上报的 `model` 时统一写空字符串，不得回填请求模型，也不得发明 `"llmwire"` 等占位名。

## 8. Usage

```rust
#[derive(Debug, Clone, Copy, Default)]
pub struct Usage {
    pub input: Option<u64>,          // None = 未知；0 = 确实是零
    pub output: Option<u64>,
    pub cached: u64,                 // input 的子集（OpenAI 语义）
    pub cache_creation: u64,         // 独立项，不计入 input（Claude）
    pub reasoning: u64,              // output 的子集
}
```

**IR-INV-USAGE-1**：`None` 与 `0` 语义不同；信息不存在时填 `None`，绝不填 `0`。
**IR-INV-USAGE-2**：包含关系在 IR 规约写死，codec 不得自由发挥：

```text
IR → Anthropic:  input_anthropic = input - cached - cache_creation
                 cache_read       = cached
                 cache_creation   = cache_creation
Anthropic → IR:  input = input_anthropic + cache_read + cache_creation
                 cached = cache_read
```

## 9. IR 级不变量汇总

| 编号 | 内容 |
|---|---|
| IR-INV-OPAQUE-1 | `Opaque` 无 `Display`/`as_str()` |
| IR-INV-OPAQUE-2 | 日志/GUI 只记 `kind + len` |
| IR-INV-OPAQUE-3 | round-trip 后 `Opaque.bytes` byte-equal |
| IR-INV-RAW-1 | `RawJson.raw()` 为字节权威，禁用反序列化回写 |
| IR-INV-RAW-2 | `parsed() == None` 合法 |
| IR-INV-TOOL-1 | `ToolId` 字节保真 |
| IR-INV-TOOL-2 | `tool_result.tool_use_id == tool_use.id` |
| IR-INV-USAGE-1 | `None ≠ 0` |
| IR-INV-USAGE-2 | usage 包含关系固定 |
| IR-INV-IMG-1 | RemoteUrl 字节保真 |
| IR-INV-IMG-2 | Base64 payload 与 media type 分离 |
| IR-INV-IMG-3 | 图片 core 不碰传输 |
| IR-INV-IMG-4 | 图片降级必须 Report |
| IR-INV-N-1 | 多候选不得静默退化 |

## 10. 相关文档

- 流式事件与 FSM：`STREAMING.md`
- 各协议映射：`chat.md` / `messages.md` / `responses.md`
- 图片输入规范：`IMAGE.md`
- API 与错误信道：`../DESIGN.md §4、§6`
