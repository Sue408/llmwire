# LLM 接口协议格式深度研究报告

> **交付日期**：2026-09-13　|　**定位**：自研 OpenAI 兼容网关的协议级技术底座
> **覆盖范围**：OpenAI 双协议（Chat Completions / Responses）、Anthropic Claude Messages、Google Gemini、国内 10 家主流厂商、开源推理与网关生态
> **方法论**：以各厂商官方 API 参考文档为一手证据，逐字段标注类型、必填性、取值范围；所有跨厂商结论标注证据等级，不臆造字段与默认值
> **体量**：全文约 17 万字符，含 60+ 字段清单表、40+ 原始报文示例

---

## 第一章　总览：协议全景与阅读指南

### 1.1 一句话结论

**当前 LLM 接口领域不存在"一个协议"，只存在"一个事实标准 + 两个独立设计 + 一片兼容层"。** OpenAI Chat Completions 是事实标准（几乎所有厂商与 SDK 都以其为默认方言），OpenAI Responses 是 OpenAI 自身向"有状态、语义事件、内置工具"演进的新一代协议，Anthropic Claude 与 Google Gemini 则是两个**对象模型完全不同**的独立设计（不是"字段改名"关系），国内厂商与开源生态构成"声称兼容 OpenAI、实则有私有扩展"的兼容层光谱。

对自研网关而言，最重要的判断是：

> **OpenAI 兼容网关可以做到"请求可路由、响应可归一化、计费可统一"，但做不到"任意 feature 无损双向映射"。**

因此正确的架构不是"用一个 JSON Schema 覆盖所有协议"，而是维护 **provider 原生对象 + 规范化内部 IR + 目标协议投影** 三层。

### 1.2 协议族谱系

| 协议族 | 代表端点 | 设计定位 | 与 OpenAI 的关系 | 本报告章节 |
|---|---|---|---|---|
| **OpenAI Chat Completions** | `POST /v1/chat/completions` | 事实标准，线性 `messages[]` + `choices[]` | 基准 | 第二章 |
| **OpenAI Responses** | `POST /v1/responses` | 有状态、items 数组、语义事件、内置工具 | 同厂演进，非简单超集 | 第二章 |
| **Anthropic Claude Messages** | `POST /v1/messages` | content block 模型 + 具名 SSE 生命周期 | 独立设计，部分可映射 | 第三章 |
| **Google Gemini** | `models/{model}:generateContent` | `contents[]/parts[]/candidates[]` | 独立设计，部分可映射 | 第四章 |
| **国内厂商** | 各家 `/v1` 或 `/compatible-mode/v1` | OpenAI 兼容 + 私有扩展（思考、缓存、搜索） | 兼容层，差异在细节 | 第五章 |
| **开源推理生态** | Ollama `/api/chat`、vLLM `/v1`、LiteLLM proxy | 自建与协议转换 | 兼容层 / 转换层 | 第六章 |

### 1.3 五个核心对象模型对照

这是理解全部字段差异的地基——**协议差异的根源是"一段对话被建模成什么"**。

| 概念 | OpenAI Chat | OpenAI Responses | Claude | Gemini |
|---|---|---|---|---|
| 对话载体 | `messages[]` 线性数组 | `input`（字符串或 item 数组） | `messages[]`（`role` 仅 user/assistant） | `contents[]`（`role` 仅 user/model） |
| 系统提示 | `role:"system"` 的 message | `instructions` 顶层字段 | **顶层 `system`**，不是 message | **顶层 `systemInstruction`** |
| 助手角色名 | `assistant` | `assistant` | `assistant` | **`model`** |
| 内容单元 | `content` 字符串或 content-part 数组 | output item + content part | **content block 数组** | **part 数组** |
| 结果容器 | `choices[].message` | `output[]` item 数组 | `content[]` block 数组 | `candidates[].content.parts[]` |
| 工具调用 | `message.tool_calls[]`，参数**JSON 字符串** | `output[]` 中 `function_call` item | `tool_use` block，参数**对象** | `functionCall` part，参数**对象** |
| 工具结果 | `role:"tool"` message | `function_call_output` item | user message 内 `tool_result` block | user content 内 `functionResponse` part |
| 停止语义 | `finish_reason` | `status` + `incomplete_details` | `stop_reason` | `finishReason`（每个 candidate） |
| 用量字段 | `usage.{prompt,completion,total}_tokens` | `usage.{input,output,total}_tokens` | `usage.{input,output}_tokens` + cache 细分 | `usageMetadata.*TokenCount` |

### 1.4 流式模型的五路分野（本报告最关键的一张表）

流式是全协议差异最大、网关最容易出错的部分。**五种流式模型互不兼容，没有一种解析器能通用。**

| 协议                   | 传输格式                                            | 事件如何标识                                                                 | 增量单位                                             | 结束标志                    | usage 在哪里                                          |
| -------------------- | ----------------------------------------------- | ---------------------------------------------------------------------- | ------------------------------------------------ | ----------------------- | -------------------------------------------------- |
| **OpenAI Chat**      | SSE，`data:` 行                                   | 靠 `object:"chat.completion.chunk"` 区分                                  | 字符串级 delta（`delta.content`）                      | `data: [DONE]`          | 需 `stream_options.include_usage`，在 `[DONE]` 前的独立末包 |
| **OpenAI Responses** | SSE，`data:` 行                                   | **`event.type` 语义事件**（约 30+ 种）                                         | 语义 delta（`output_text.delta` 等）                  | `response.completed` 事件 | 通常在 `response.completed` 事件中                       |
| **Claude**           | SSE，**`event:` 行携带具名事件**                        | `message_start` / `content_block_*` / `message_delta` / `message_stop` | block 生命周期 + `partial_json`                      | `message_stop`          | **分散两处**：`message_start` 给输入，`message_delta` 给输出   |
| **Gemini**           | SSE（靠 `?alt=sse` 触发，**不是 body 里的 stream 字段**）   | 无事件名                                                                   | **完整的 `GenerateContentResponse` 对象流**（非差分 delta） | 流正常关闭                   | 最后一个 chunk 的 `usageMetadata`                       |
| **Ollama 原生**        | **NDJSON**（`application/x-ndjson`，无 `data:` 前缀） | 无                                                                      | 完整对象                                             | 对象内 `done:true`         | 末对象性能字段                                            |

三条必须记住的工程结论：

1. **Claude 的 usage 在流式中是"前后分离"的**——输入 token 在第一个事件，输出 token 在倒数第二个事件，必须在流结束时合并；
2. **Gemini 的流式 chunk 是完整响应对象而非差分**——聚合逻辑与 OpenAI 完全不同，不能简单 append；
3. **Gemini 的流式靠 URL 查询参数 `alt=sse` 开启**，而 OpenAI/Claude 靠 body 里的 `stream:true`——网关路由层必须分别处理。

### 1.5 三个"看起来是 200 其实失败了"的陷阱

| 陷阱 | 表现 | 判定方式 |
|---|---|---|
| **Gemini 安全拦截** | HTTP 200，但 `candidates: []` | 检查 `candidates.length===0` 后读 `promptFeedback.blockReason`，转 `content_filter` 或错误 |
| **Responses 未完成** | HTTP 200，但 `status: "incomplete"` | 检查 `status` 与 `incomplete_details.reason`，不能一律映射为成功 |
| **流中错误事件** | HTTP 200 已发送，SSE 中途出现 `error` 事件 | 解析 SSE 错误事件并终止流，不能只依赖 HTTP 状态码 |

### 1.6 阅读指南

| 你的角色 | 建议阅读路径 |
|---|---|
| **自研网关架构师** | 第一章 → 第二章（基准方言）→ 第七章（跨协议映射）→ 第八章（错误重试）→ 第九章（落地清单） |
| **接入 Claude / Gemini** | 第一章 → 第三章 / 第四章 → 第七章（重点看流式转换） |
| **接入国内厂商** | 第一章 → 第五章 → 9.3 节（兼容层陷阱清单） |
| **自建推理服务** | 第一章 → 第六章 → 9.4 节（canonical 设计） |
| **只需要字段速查** | 直接翻各章的字段全表与附录 A 契约测试清单 |

### 1.7 术语与证据等级约定

- **IR（Intermediate Representation）**：网关内部的规范化中间表示，本报告推荐以"OpenAI Chat 超集"作为 canonical。
- **canonical / 规范化**：把各家协议字段统一到内部标准名（如统一为 `max_output_tokens`）。
- **投影（projection）**：把 IR 渲染成目标协议的字段名与结构。
- **证据等级**：`A`＝已抓取厂商官方文档且字段级确认；`B`＝官方文档定位页，详细字段未完全加载；`C`＝官方兼容性表述但缺逐字段规范。C 级结论只用于判断接口形态，不用于断言默认值与全模型支持。
- **版本风险提示**：所有协议文档均为持续更新的活文档。模型可用性、参数支持度、枚举值、默认值会动态变化；**网关应在请求校验层从 OpenAPI/JSON Schema 或运行时模型元数据动态刷新能力矩阵**，不能把本报告中的模型特定数字当作长期承诺。

## 第二章　OpenAI 双协议：Chat Completions 与 Responses

> 这是全行业的事实标准方言，也是自研网关的 canonical 基准。Chat Completions 仍是长期受支持的经典接口，Responses 则是 OpenAI 面向有状态交互、内置工具与语义事件流的新一代接口，两者**不是简单超集关系**。

### 文档版本与证据边界

**依据日期：2026-09-13。** 本报告以 OpenAI 官方 API Overview、Chat/Responses API Reference、Streaming、Function Calling、Structured Outputs、Rate Limits、Error Codes 与 Changelog 为核心证据；Responses 已定位为直接模型请求、工具、音频/图像/文本输入和有状态交互的推荐 API，Chat Completions 仍受长期支持。[1][2]

**版本风险：官方参考页为持续更新的活文档，模型可用性会动态变化。** 因此，请求参数是否支持、枚举值、字段默认值和响应字段存在性均应按“请求时模型 + 请求时 OpenAPI/实际响应”校验，不能仅依赖本文快照。特别是 GPT-5 之后，`reasoning_effort`、`tool_choice.allowed_tools`、`web_search_options`、音频、多模态、Prompt Cache、内置工具等强模型相关；官方迁移文档明确说明，从 GPT-5.4 起，Chat Completions 不支持 `reasoning_effort` 取 `none` 之外的值。[2]

**证据规则：表内“必填”按接口调用的真实前置条件写。** 若字段在官方文档中表述为“仅在某输出形态/模型下必需”，写为条件必填；未公开承诺固定默认值的字段，不臆造默认值。Chat Completions 与 Responses 的 HTTP 错误、请求 ID、限流头是共享行为，但响应对象的字段命名不可混用。[1][5][8]

### 共享传输、鉴权与生命周期

#### HTTP 层

| 项目 | Chat Completions | Responses | 规范要点 |
|---|---|---|---|
| 方法/路径 | `POST /v1/chat/completions` | `POST /v1/responses` | 均为 JSON 请求；路径不可混用 |
| Content-Type | `application/json` | `application/json` | 请求体必须是 JSON；非流式也不得依赖 multipart |
| 鉴权 | `Authorization: Bearer ${API_KEY_OR_TOKEN}` | 同左 | API Key 或 Workload Identity 短期令牌；密钥不得下发客户端 |
| 组织/项目 | `OpenAI-Organization`、`OpenAI-Project` | 同左 | 仅 legacy user key 访问项目等场景需要；决定用量归属 |
| 请求 ID | 响应头 `x-request-id` | 同左 | 全链路排障主键，应落日志 |
| 响应格式 | JSON；流式为 `text/event-stream` | JSON；流式为 `text/event-stream` | `stream=true` 才应建立 SSE 连接 |
| 头总大小 | ≤64KiB | ≤64KiB | 官方建议单值/总值留白；过大可能在到达 API 前失败且无 `x-request-id` |

官方 API Overview 明确：Bearer 是主要认证方式；组织/项目头用于指定用量归属；所有请求头（含 Authorization、常见头、自定义头）合计应低于 64KiB，单独自定义头及其总值建议不超过 60KiB。[1] 自研网关应把 `Authorization`、`OpenAI-Organization`、`OpenAI-Project` 列为**白名单透传头**，默认剥离其它请求头；出口侧可追加 tracing header，但必须避免挤占 64KiB 预算。

**两协议的核心对象边界是“choice vs output items”。** Chat Completions 的 `messages` 是一个线性历史；模型结果放入 `choices[]`，工具调用被压平到 `message.tool_calls`。Responses 的 `input` 可以包含历史输入项与前一轮 `output` 项；结果以 `output[]` 保存，可并列为 `reasoning`、`message`、`function_call`、`function_call_output`、内置工具调用等。迁移文档示例显示，简单消息数组可在两接口间复用，但对话状态、工具和结构化输出形状必须转换。[2]

### Chat Completions API

#### 请求头与非流式请求

| 字段 | 类型 | 必填 | 说明 |
|---|---|---:|---|
| `Authorization` | string | 是 | `Bearer <secret>` |
| `Content-Type` | string | 是 | `application/json` |
| `OpenAI-Organization` | string | 条件 | 多组织/特定归属时使用 |
| `OpenAI-Project` | string | 条件 | 项目级归属时使用 |
| `OpenAI-Beta` | string | 条件 | 仅在调用明确标为 Beta 的能力时按文档设置；双协议通用 |
| `x-request-id` | string | 否 | 客户端一般不应伪造；服务端回显以实际响应头为准 |

#### `POST /v1/chat/completions` 请求 body

| 字段 | 类型 | 必填 | 默认值 | 取值范围/枚举 | 含义与兼容性 |
|---|---|---:|---:|---|---|
| `model` | string | 是 | — | 有效模型 ID | 生成模型；模型决定工具、推理、多模态、音频等能力 |
| `messages` | array | 是 | — | `length≥1` | 完整对话历史；结构与 Roles 见下文 |
| `modalities` | array | 否 | `["text"]` | `["text"]`、`["audio"]`、`["text","audio"]` | 输出模态；请求音频输出时通常须设置 |
| `audio` | object | 条件 | — | `{format,voice}` | `modalities` 含 `audio` 时所需 |
| `reasoning_effort` | string | 否 | 模型默认 | `none`/`minimal`/`low`/`medium`/`high` 等以模型为准 | 推理预算；GPT-5.4+ Chat 仅 `none` 外不支持工具调用，必须以当前模型文档为准 |
| `store` | boolean | 否 | 新账户默认 `true` | `true`/`false` | 是否保存输出以供蒸馏/eval；非流式存储语义与 Responses 不同 |
| `metadata` | object | 否 | `{}` | 最多 16 键值；键≤64、值≤512 | 业务标签；不应存敏感 PII |
| `temperature` | number | 否 | 模型默认 | `[0,2]` | 与 `top_p` 二选一调整，不要同时强约束 |
| `top_p` | number | 否 | 模型默认 | `[0,1]` | 核采样 |
| `frequency_penalty` | number | 否 | 0 | `[-2,2]` | 频率惩罚 |
| `presence_penalty` | number | 否 | 0 | `[-2,2]` | 存在惩罚 |
| `logit_bias` | object | 否 | `{}` | token id→`[-100,100]` | token id 映射 |
| `logprobs` | boolean | 否 | `false` | `true`/`false` | 开启 token 级对数概率 |
| `top_logprobs` | integer | 否 | — | `[0,20]` | 需 `logprobs=true`；实际返回数可能少于请求值 |
| `max_completion_tokens` | integer | 否 | 模型默认 | `≥0`，受上下文和模型约束 | 包含可见输出与推理 token 的上限；推荐用于推理模型 |
| `max_tokens` | integer | 否 | 模型默认 | 历史字段 | **弃用别名**；不与 o 系列兼容，新实现必须优先 `max_completion_tokens` |
| `n` | integer | 否 | 1 | `≥1`；上限模型相关 | 生成多个 choice；会放大计费/延迟 |
| `seed` | integer | 否 | — | int64 范围 | 尽力复现，不构成跨模型/版本的字节级保证 |
| `stop` | string/array | 否 | — | 单字符串，或最多 4 个字符串 | 停止序列；输出不含该序列 |
| `tools` | array | 否 | — | function/function+strict/custom | 替代 legacy `functions` |
| `tool_choice` | string/object | 否 | 有 tools 时 `auto`；否则 `none` | 见工具章节 | 工具选择策略 |
| `parallel_tool_calls` | boolean | 否 | `true` | `true`/`false` | 是否允许单轮并行函数调用 |
| `response_format` | object | 否 | `{type:"text"}` | `text`/`json_object`/`json_schema` | 结构化输出；见第四部分 |
| `service_tier` | string | 否 | `auto` | `auto`/`default`/`flex`/`priority` 等项目允许值 | 服务档位；非法值返回 400 |
| `stream` | boolean | 否 | `false` | `true`/`false` | 启用 SSE |
| `stream_options` | object | 否 | — | `{include_usage,include_obfuscation}` | 仅 `stream=true` 有效 |
| `prediction` | object | 否 | — | `{type:"content",content}` | 预测输出，用于文件再生成等缓存加速 |
| `user` | string | 否 | — | 稳定终端标识 | 官方提示将被 `prompt_cache_key`/safety_identifier 替代；保留仅为兼容 |
| `web_search_options` | object | 否 | — | `{search_context_size,user_location}` | 旧式 Chat 网络搜索配置；可用性以模型为准 |
| `functions` | array | 否 | — | legacy function 定义 | **弃用**，应迁移到 `tools` |
| `function_call` | string/object | 否 | — | `none`/`auto`/`{name}` | **弃用**，应迁移到 `tool_choice` |

`max_tokens` 与 `max_completion_tokens` 不能等同：`max_tokens` 是旧文本预算，官方明确已由 `max_completion_tokens` 替代，且在 o-series 上不兼容；网关在接收到下游“Chat 兼容”请求时，若目标后端仅理解旧字段，应进行显式映射，并记录模型是否支持推理预算。[3] 反之，不得将 Responses 的 `max_output_tokens` 直接写成 Chat 的 `max_completion_tokens`：两者是不同接口字段。

##### `messages[*]` 的 role 与内容

| role | 位置/用途 | content | 特有字段 |
|---|---|---|---|
| `system` | 旧指令；o1+ 推荐改 `developer` | string 或 parts（parts 支持以模型为准） | `name?` |
| `developer` | o1+ 的系统级指令 | string；官方 reference 仅明确 text part | `name?` |
| `user` | 用户输入 | string，或 content-part 数组 | `name?` |
| `assistant` | 上一轮模型输出 | string/parts/`null` | `tool_calls?`、`refusal?`、`audio?` |
| `tool` | 工具结果 | string/parts | `tool_call_id`（必填）、`name?` |

user/assistant content-part 至少包含：`text`（`{type:"text",text,...}`）、`image_url`（`{type:"image_url",image_url:{url,detail?}}`）、`input_audio`、file part。text part 还可携带 `prompt_cache_breakpoint`。官方 image part 的 `detail` 为 `auto`/`low`/`high`；官方明确“image inputs over 8MB will be dropped”，这是网关限流/预处理告警点。[3][9]

**assistant 消息中的 `tool_calls` 与 `content` 可共存。** 历史请求中 `assistant` 必须原样保留 `tool_calls`，工具结果以 `role:"tool"` 紧随其后；不得把 tool_call 改写成自然语言，也不得只保留 `content`。`refusal` 在可用时出现；当策略拒绝时 `content` 可能为 `null` 而 `refusal` 非空，应用层必须优先处理拒绝分支。

##### tool、response_format、audio 子对象

| 子对象 | 字段 | 类型 | 必填 | 说明 |
|---|---|---|---:|---|
| `tools[i]`（function） | `type` | `"function"` | 是 | 固定值 |
|  | `function` | object | 是 | `{name,description?,parameters,strict?}` |
|  | `function.strict` | boolean | 否 | `true` 启用严格函数调用；要求 schema 约束 |
| `tool_choice` | `"auto"`/`"required"`/`"none"` | string | 否 | 分别为可调用、必须调用、禁止调用 |
|  | `{type:"function",function:{name}}` | object | 否 | 强制指定函数 |
|  | allowed_tools 形态 | object | 否 | 限制模型可选工具子集；非所有模型/版本均支持，需运行时校验 |
| `response_format` | `type` | string | 是 | `text`/`json_object`/`json_schema` |
|  | `json_schema` | object | 条件 | 含 `name`、`schema`、`strict?`、`description?` |
| `audio` | `format` | string | 是 | `wav`/`mp3`/`flac`/`opus`/`pcm16` |
|  | `voice` | string/object | 是 | 内置 voice 或 `{id}`；请求音频输出时必填 |

#### 非流式响应对象

```json
{
  "id": "chatcmpl-abc123",
  "object": "chat.completion",
  "created": 1760200000,
  "model": "gpt-5.2",
  "service_tier": "default",
  "system_fingerprint": "fp_xxx",
  "choices": [
    {
      "index": 0,
      "finish_reason": "tool_calls",
      "logprobs": null,
      "message": {
        "role": "assistant",
        "content": null,
        "refusal": null,
        "tool_calls": [
          {
            "id": "call_abc",
            "type": "function",
            "function": {"name": "get_weather", "arguments": "{\"location\":\"Paris\"}"}
          }
        ],
        "audio": null,
        "annotations": []
      }
    }
  ],
  "usage": {
    "prompt_tokens": 120,
    "completion_tokens": 45,
    "total_tokens": 165,
    "prompt_tokens_details": {"cached_tokens": 80, "audio_tokens": 0},
    "completion_tokens_details": {"reasoning_tokens": 20, "audio_tokens": 0}
  }
}
```

| 顶层字段 | 类型 | 出现条件 | 说明 |
|---|---|---|---|
| `id` | string | 总是 | `chatcmpl-*` |
| `object` | string | 总是 | `"chat.completion"` |
| `created` | integer | 总是 | Unix 秒 |
| `model` | string | 总是 | 实际模型；可能与请求别名不同 |
| `service_tier` | string | 条件 | 请求指定/适用时返回 |
| `system_fingerprint` | string | 条件 | 后端配置指纹；可配合 `seed` 诊断 |
| `choices` | array | 总是 | 通常 1；`n>1` 时多个 |
| `usage` | object | 通常/store 等条件下 | 非流式默认通常存在；旧调用或特定配置可能不同，网关不能强断言 |

##### `choices[i]`

| 字段 | 类型 | 说明 |
|---|---|---|
| `index` | integer | choice 序号 |
| `finish_reason` | string/null | `stop`/`length`/`tool_calls`/`content_filter`/`function_call`（legacy）/`null` |
| `logprobs` | object/null | `logprobs=true` 时包含 token、logprob、bytes、`top_logprobs` |
| `message` | object | assistant 输出；**非流式不使用 delta** |

`finish_reason` 语义：`stop` 为自然停止/stop sequence；`length` 触及输出 token 上限；`tool_calls` 模型要调用工具；`content_filter` 输出被安全过滤；官方 Chunk schema 仍列 `function_call` 作为 deprecated 取值。[3] **`length` 不表示任务成功完成**，网关应向上游返回相同 reason，同时记录截断指标；若业务依赖完整结构化输出，应触发补完/重试流程，而不是把半成品 JSON 当作成功。

##### `message`

| 字段 | 类型 | 说明 |
|---|---|---|
| `role` | `"assistant"` | 固定 |
| `content` | string/null | 文本；工具调用时可 `null` |
| `refusal` | string/null | 策略拒绝内容 |
| `tool_calls` | array/null | function/custom tool calls |
| `audio` | object/null | `{id,data,expires_at,transcript}` |
| `annotations` | array | 来源引用等；缺省可能是 `[]` 而非 absent，按实际响应归一 |

##### `usage`

| 字段 | 类型 | 说明 |
|---|---|---|
| `prompt_tokens` | integer | 输入 token；不同模型定义可能略有差异 |
| `completion_tokens` | integer | 输出 token，可能含推理 |
| `total_tokens` | integer | `prompt+completion` |
| `prompt_tokens_details.cached_tokens` | integer | prompt cache 命中量 |
| `prompt_tokens_details.audio_tokens` | integer | 条件字段 |
| `completion_tokens_details.reasoning_tokens` | integer | 条件字段；**推理模型必须单独计费/告警** |
| `completion_tokens_details.audio_tokens` | integer | 条件字段 |

#### Chat Completions SSE

**传输帧是标准 SSE，不是“JSON 每行一个”。** 响应 `Content-Type: text/event-stream`；每个事件通常有 `data: <json>\n\n`。OpenAI 使用 `data: [DONE]` 作为流终止标记；`: keep-alive` 注释行可以出现，客户端必须按 SSE 忽略注释。`[DONE]` 不是 JSON，解析器必须先字符串比较再 JSON 解析。HTTP/2、Nginx、CDN 的缓冲会把增量变成批量，因此网关出口必须禁用响应缓冲并周期性发送注释或实际事件。

```http
HTTP/1.1 200 OK
Content-Type: text/event-stream
Cache-Control: no-cache
Connection: keep-alive
x-request-id: req_abc

data: {"id":"chatcmpl-abc","object":"chat.completion.chunk","created":1760200000,"model":"gpt-5.2","choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]}

data: {"id":"chatcmpl-abc","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}

data: {"id":"chatcmpl-abc","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"get_weather","arguments":""}}]},"finish_reason":null}]}

data: {"id":"chatcmpl-abc","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"loc"}}]},"finish_reason":null}]}

data: {"id":"chatcmpl-abc","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}

data: {"id":"chatcmpl-abc","object":"chat.completion.chunk","created":1760200000,"model":"gpt-5.2","usage":{"prompt_tokens":20,"completion_tokens":10,"total_tokens":30},"choices":[]}

data: [DONE]
```

##### chunk 字段与 delta 规则

| chunk 字段 | 类型 | 说明 |
|---|---|---|
| `id`/`object`/`created`/`model`/`system_fingerprint` | scalar | 每个相关 chunk 可重复；不得按首个 chunk 提前关闭状态 |
| `choices` | array | 通常为 1；`include_usage` 的末包可为 `[]` |
| `choices[i].index` | integer | 与 `n` 对应 |
| `choices[i].delta` | object | 相对增量；不是完整 message |
| `choices[i].finish_reason` | string/null | 在终止 chunk 出现；末包之前通常 `null` |
| `choices[i].logprobs` | object/null | token 级增量；与 message logprobs 结构不同 |
| `usage` | object/null | 仅 `stream_options.include_usage=true` 的末包通常非空；中断时可能丢失 |

delta 的首包常含 `{"role":"assistant"}`；之后 `content` 是 UTF-16/字符语义的增量片段；空字符串 delta 是合法增量，不能跳过。tool_call 增量使用 `tool_calls[]` 的 `index` 作为合并主键：`id`、`type`、`function.name` 通常在 `index` 首次出现时给出，`function.arguments` 为**不保证合法 JSON 的前缀流**。`arguments` 可在同一 index 上多次到达，必须以字符串拼接：

```python
state: dict[int, dict] = {}
text = ""
args_buffer: dict[int, str] = {}

for chunk in parse_sse(req):
    if chunk is DONE:
        break
    for c in chunk.get("choices", []):
        d = c.get("delta") or {}
        if d.get("content") is not None:
            text += d["content"]
        for tc in d.get("tool_calls") or []:
            i = tc["index"]
            slot = state.setdefault(i, {"id": None, "type": "function", "function": {"name": "", "arguments": ""}})
            if tc.get("id"): slot["id"] = tc["id"]
            if tc.get("type"): slot["type"] = tc["type"]
            fn = tc.get("function") or {}
            if "name" in fn: slot["function"]["name"] += fn["name"]
            if "arguments" in fn:
                slot["function"]["arguments"] += fn["arguments"]
                args_buffer[i] = slot["function"]["arguments"]
        if c.get("finish_reason"):
            final_reason = c["finish_reason"]
```

**参数 JSON 不完整是设计事实，不是错误。** 在 `finish_reason="tool_calls"` 之前不应要求 `json.loads(arguments)`；收到 done/terminal chunk 后仍可能因转义、截断、`content_filter` 或连接中断而不合法。生产逻辑应：1）保留原始拼接字符串；2）仅作为工具调用参数解析；3）解析失败时分模型重试、返回工具错误、或向上游暴露“参数无法解析”状态，而不能静默传空对象。注意 `\u2028`/`\u2029`、控制字符、base64 和转义序列跨越 chunk 边界的情形；不要自行在网关里修复 JSON。

### Responses API

#### 设计差异：items 与持久 response

**Responses 把“模型过程”变成一等输出。** Chat Completions 将一个 assistant turn 压成一条 `message`；Responses 的 `output[]` 可按顺序保存 `reasoning`、`message`、`function_call`、内置工具调用等。这样能表达推理摘要、并行工具、图像生成、文件搜索、MCP、shell/code interpreter 等过程，但也意味着“把最后一个 assistant 文本塞回 messages”的 Chat 模式不可直接复用。[2][4]

`store` 默认为 `true`；`previous_response_id` 让服务端把前序响应纳入下一轮。二者互斥使用场景：`previous_response_id` 绑定已存储响应；若 `store=false` 或无 ZDR 保留能力，应手工把前一轮完整 `output` 作为 `input` 传回。官方 `reasoning.encrypted_content` 的 `include` 说明直接指出：无状态多轮必须带回 reasoning item，否则推理上下文可能丢失。[4]

#### `POST /v1/responses` 请求 body

| 字段 | 类型 | 必填 | 默认值 | 取值范围 | 含义 |
|---|---|---:|---:|---|---|
| `model` | string | 条件 | 客户端默认/请求级 | 模型 ID | 可直接传，也可由 SDK 客户端默认值提供；纯代理层建议强制客户端必填 |
| `input` | string/array | 否 | — | 文本或 input item 数组 | 等价于单条 user text；数组用于多轮/多模态/工具历史 |
| `instructions` | string | 否 | — | developer 指令 | 每轮独立；`previous_response_id` 不会自动继承 |
| `previous_response_id` | string | 否 | — | `resp_*` | 服务端串联；与 `conversation` 语义相关，不可混用同一请求 |
| `store` | boolean | 否 | `true` | `true`/`false` | 是否持久化 response |
| `stream` | boolean | 否 | `false` | `true`/`false` | 语义事件流 |
| `parallel_tool_calls` | boolean | 否 | `true` | `true`/`false` | 单轮函数并行 |
| `tools` | array | 否 | — | function/内置工具 | 见工具章节 |
| `tool_choice` | string/object | 否 | `auto` | `auto`/`required`/`none`/指定工具 | Responses 工具语义 |
| `text` | object | 否 | — | `{format:{type,json_schema?,verbosity?}}` | 结构化输出/文本约束 |
| `reasoning` | object | 否 | 模型默认 | `{effort,summary}` | 仅适用推理模型；非法值报错 |
| `max_output_tokens` | integer | 否 | 模型默认 | `≥16`，且受模型上限约束 | 包含可见输出和推理 token |
| `temperature` | number | 否 | 模型默认 | `[0,2]` | 非推理采样参数；推理模型可能忽略 |
| `top_p` | number | 否 | 模型默认 | `[0,1]` | 同上 |
| `truncation` | string | 否 | `disabled` | `auto`/`disabled` | `auto` 丢弃中间项；`disabled` 超窗返回 400 |
| `metadata` | object | 否 | `{}` | 16 键值；键≤64、值≤512 | 与 Chat 相同限制 |
| `user` | string | 否 | — | 稳定终端标识 | 缓存/安全标识；可能被新字段替代 |
| `service_tier` | string | 否 | `auto` | 项目允许档位 | 档位策略 |
| `include` | array | 否 | — | 见下文 | 请求附加输出 |
| `prompt` | object | 否 | — | `{id,version?,variables?}` | 可复用 prompt 模板；Chat Completions 无此能力 |
| `background` | boolean | 否 | `false` | `true`/`false` | 异步 response |
| `moderation` | object | 否 | — | 输入/输出 moderation 配置 | 企业策略；结构以文档为准 |
| `max_tool_calls` | integer | 否 | 模型默认 | `≥0` | 单次 response 内置工具调用总上限 |
| `conversation` | string/object | 否 | — | conversation id/配置 | Conversations API；与 `previous_response_id` 不直接等同 |
| `context_management` | array | 否 | — | 上下文管理配置 | 受模型/服务端演进影响，建议运行时校验 |
| `prompt_cache_key` | string | 否 | — | 稳定缓存键 | 官方推荐替代 Chat `user` |
| `prompt_cache_options` | object | 否 | — | `{mode,ttl,breakpoints?}` | gpt-5.6+；默认 1 隐式断点、最多写 4、匹配最近 80；TTL 当前仅 30m |

官方 Responses create 明确：`input` 可为文本/图像/文件；`instructions` 与 `previous_response_id` 不会跨轮继承；`metadata` 为 16 键值；`max_output_tokens` 上限覆盖可见与推理 token；`include` 可请求 `web_search_call.action.sources`、`code_interpreter_call.outputs`、`computer_call_output.output.image_url`、`file_search_call.results`、`message.input_image.image_url`、`message.output_text.logprobs`、`reasoning.encrypted_content`。[4]

##### `input` item 类型（最小字段集）

| item `type` | 关键字段 | 用途 |
|---|---|---|
| `message` | `role`：`user`/`assistant`/`system`/`developer`；`content`：string/parts | 基本对话；与 Chat `messages` 最接近的迁移入口 |
| `input_text` | `text` | 输入文本 part |
| `input_image` | `image_url`/`file_id`；`detail`：`low`/`high`/`auto` | 图像输入；data URI 或 file id |
| `input_audio` | `data`/`format`/`file_id?` | 音频输入 |
| `input_file` | `file_id`/`filename`/`file_url`/`file_data` | 文档/文件输入；支持类型持续扩展 |
| `reasoning` | `id?`、`summary?`、`encrypted_content?` | 无状态多轮必须原样回填 |
| `function_call` | `call_id`、`name`、`arguments` | 前轮模型调用；回填时保持原始 item |
| `function_call_output` | `call_id`、`output`；`type?` | 工具结果；可含文本/图像/file content |
| 内置工具调用项 | `web_search_call`、`file_search_call`、`code_interpreter_call`、`computer_call`、`image_generation_call`、`mcp_call`、`shell_call` 等 | 输入/输出项由工具类型决定；不能伪造未执行的内置调用 |

**`input` 与 `messages` 的兼容仅限“简单消息”。** 官方迁移示例说明，不含函数和复杂多模态时，Chat 的 message 数组可作为 Responses `input` 复用；一旦存在 `tool_calls`、`refusal`、音频、reasoning 或内置工具，应按 item schema 映射。[2] 网关的最佳策略是维护“可逆中间表示”：内部以 Responses item 为主，向 Chat 下游渲染时生成合规 `messages`，但保留 `call_id↔tool_call_id`、reasoning、annotations 的原始映射。

#### 非流式 response 对象

```json
{
  "id": "resp_abc",
  "object": "response",
  "created_at": 1760200000,
  "status": "completed",
  "model": "gpt-5.2",
  "instructions": "You are helpful.",
  "output": [
    {"id":"rs_1","type":"reasoning","summary":[],"encrypted_content":null},
    {"id":"msg_1","type":"message","role":"assistant","status":"completed",
     "content":[{"type":"output_text","text":"Paris weather unavailable.","annotations":[]}]}
  ],
  "output_text": "Paris weather unavailable.",
  "parallel_tool_calls": true,
  "tool_choice": "auto",
  "tools": [],
  "temperature": 1,
  "top_p": 1,
  "usage": {
    "input_tokens": 120,
    "output_tokens": 30,
    "total_tokens": 150,
    "input_tokens_details": {"cached_tokens": 80},
    "output_tokens_details": {"reasoning_tokens": 12}
  },
  "metadata": {}
}
```

| 顶层字段 | 类型 | 条件 | 说明 |
|---|---|---|---|
| `id` | string | 总是 | `resp_*` |
| `object` | string | 总是 | `"response"` |
| `created_at` | integer | 总是 | Unix 秒 |
| `status` | string | 总是 | `queued`/`in_progress`/`completed`/`failed`/`incomplete`/`cancelled` |
| `error` | object | 条件 | `{code,message}`；`failed` 时存在 |
| `incomplete_details` | object | 条件 | `incomplete` 原因，如 `max_output_tokens`；不应由网关臆造 reason |
| `model` | string | 总是 | 实际模型 |
| `output` | array | 总是 | 即使空数组也不得省略 |
| `output_text` | string | 便捷字段 | 拼接首个/适用文本；多 part、拒绝、纯工具调用场景不保证代表完整结果 |
| `parallel_tool_calls`/`tool_choice`/`tools` | 原类型 | 条件回显 | 便于审计 |
| `usage` | object | 通常 | 同上；异步/中断可能未最终化 |

##### `output` item 核心类型

| item | 必含字段 | 其它重要字段 |
|---|---|---|
| `reasoning` | `type:"reasoning"`、`id` | `summary[]`（`summary_text`）、`encrypted_content`、`content[]` |
| `message` | `type:"message"`、`id`、`role:"assistant"`、`content[]` | `status`：`in_progress`/`completed` |
| `output_text` content | `type:"output_text"`、`text` | `annotations[]`、`logprobs[]` |
| `refusal` content | `type:"refusal"`、`refusal` | 安全拒绝；`message.content` 可同时有其他 part 以模型为准 |
| `function_call` | `type:"function_call"`、`call_id`、`name`、`arguments` | 与 Chat `tool_calls` 不同：Responses 是 output item，`arguments` 为完整字符串 |

annotation 类型包括 `url_citation`、`file_citation`、`container_file_citation` 等，具体由工具/模型返回；网关应保留 `type` 并做开放 map 归一，禁止将未知 annotation 丢弃。

#### Responses SSE：语义事件而非字节增量

**Responses 事件可以无损聚合为最终 response，但反之不一定。** 事件携带 `response_id`、`type`、`sequence_number`、生命周期字段；output 用 `output_index`、content part 用 `content_index`、工具调用用 `item_id`/`call_id` 定位。聚合规则是 `added → delta* → done`：对象先出现骨架，文本/参数持续追加，done 给出最终快照。不能假定事件严格按单一线性顺序；应建立 `(output_index, content_index, item_id)` 索引。

| 事件 | payload 关键字段 | 生命周期/含义 |
|---|---|---|
| `response.created` | `response` | response 已创建；对象快照 |
| `response.in_progress` | `response` | 开始生成 |
| `response.output_item.added` | `output_index`、`item` | 新 output item 骨架 |
| `response.output_item.done` | `output_index`、`item` | item 完成快照 |
| `response.content_part.added` | `output_index`、`content_index`、`part` | message part 创建 |
| `response.content_part.done` | `output_index`、`content_index`、`part` | part 完成 |
| `response.output_text.delta` | `output_index`、`content_index`、`delta` | 文本片段；UTF-8/JSON 编码文本增量 |
| `response.output_text.done` | `output_index`、`content_index`、`text` | 最终文本；优先以此而非逐 delta 校验 JSON |
| `response.output_text.annotation.added` | `output_index`、`content_index`、`annotation`/`annotation_index` | 引用追加 |
| `response.refusal.delta` | 索引、`delta` | 拒绝文本增量 |
| `response.refusal.done` | 索引、`refusal` | 拒绝完成 |
| `response.reasoning_summary_part.added` | `output_index`、`summary_index` | 推理摘要 part 开始 |
| `response.reasoning_summary_text.delta` | `output_index`、`summary_index`、`delta` | 摘要文本增量 |
| `response.reasoning_summary_text.done` | `output_index`、`summary_index`、`text` | 摘要 part 完成 |
| `response.reasoning_summary_part.done` | 索引 | 摘要 part 生命周期结束 |
| `response.reasoning_text.delta`/`.done` | 索引、`delta`/`text` | 完整推理文本（可用性/权限受限） |
| `response.function_call_arguments.delta` | `item_id`、`output_index`、`call_id`、`delta` | 参数 JSON 前缀；可能为空串 |
| `response.function_call_arguments.done` | `item_id`、`output_index`、`call_id`、`arguments` | 最终参数；不要假设 delta 已能解析 |
| `response.function_call.delta`/`.done` | call item 字段 | 自定义/扩展工具调用；具体事件可用性需文档校验 |
| `response.custom_tool_call.input.delta/.done` | tool 索引、input | custom tool 输入流 |
| `response.web_search_call.in_progress`/`.searching`/`.completed` | call 字段、`action?`、`results?` | 搜索开始、搜索中、完成 |
| `response.file_search_call.in_progress`/`.searching`/`.completed` | queries/results | 文件检索；results 依赖 `include` |
| `response.code_interpreter_call.in_progress`/`.code.delta`/`.code.done`/`.interpreting`/`.completed` | code/output | 代码解释器生命周期 |
| `response.image_generation_call.in_progress`/`.generating`/`.partial_image`/`.completed` | image 字段 | 图像生成；partial 非最终 |
| `response.mcp_list_tools.in_progress`/`.succeeded`/`.failed` | tools/error | MCP 工具发现 |
| `response.mcp_call.in_progress`/`.arguments.delta`/`.arguments.done`/`.completed`/`.failed` | call/arguments | MCP 调用生命周期 |
| `response.shell_call.in_progress`/`.command.added`/`.command.done`/`.output.delta`/`.output.done` | command/output | 托管 shell；权限/可用性受限 |
| `response.completed` | `response` | 完整最终对象；通常含 usage，但应以实际字段为准 |
| `response.incomplete` | `response`、`sequence_number` | `incomplete_details` 必读；不等价于成功 |
| `response.failed` | `response`/`error` | 终态失败；根据 error 决定重试 |
| `error` | `code`、`message`、`type?` | 流中错误；之后可能关闭流，但仍应先处理已发射 item |

```http
data: {"type":"response.created","response":{"id":"resp_abc","object":"response","status":"in_progress","model":"gpt-5.2","output":[]}}

data: {"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"rs_1","summary":[],"encrypted_content":null}}

data: {"type":"response.reasoning_summary_part.added","output_index":0,"summary_index":0}

data: {"type":"response.reasoning_summary_text.delta","output_index":0,"summary_index":0,"delta":"Plan"}

data: {"type":"response.reasoning_summary_text.done","output_index":0,"summary_index":0,"text":"Plan route"}

data: {"type":"response.output_item.done","output_index":0,"item":{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"Plan route"}]}}

data: {"type":"response.output_item.added","output_index":1,"item":{"type":"message","id":"msg_1","role":"assistant","status":"in_progress","content":[]}}

data: {"type":"response.content_part.added","output_index":1,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}

data: {"type":"response.output_text.delta","output_index":1,"content_index":0,"delta":"Hi"}

data: {"type":"response.output_text.annotation.added","output_index":1,"content_index":0,"annotation_index":0,"annotation":{"type":"url_citation","url":"https://example.com","title":"Example"}}

data: {"type":"response.output_text.done","output_index":1,"content_index":0,"text":"Hi there"}

data: {"type":"response.content_part.done","output_index":1,"content_index":0,"part":{"type":"output_text","text":"Hi there","annotations":[{"type":"url_citation","url":"https://example.com","title":"Example"}]}}

data: {"type":"response.output_item.done","output_index":1,"item":{"type":"message","id":"msg_1","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hi there","annotations":[{"type":"url_citation","url":"https://example.com","title":"Example"}]}]}}

data: {"type":"response.completed","response":{"id":"resp_abc","status":"completed","output":[{}],"usage":{"input_tokens":20,"output_tokens":5,"total_tokens":25}}}
```

聚合伪代码：

```python
resp = {"output": []}
for ev in sse_events():
    if ev["type"] == "response.output_item.added":
        put(resp["output"], ev["output_index"], ev["item"])
    elif ev["type"] == "response.content_part.added":
        item = resp["output"][ev["output_index"]]
        put(item["content"], ev["content_index"], ev["part"])
    elif ev["type"] == "response.output_text.delta":
        part = resp["output"][ev["output_index"]]["content"][ev["content_index"]]
        part["text"] = (part.get("text") or "") + ev["delta"]
    elif ev["type"] == "response.function_call_arguments.delta":
        item = resp["output"][ev["output_index"]]
        item["arguments"] = (item.get("arguments") or "") + ev["delta"]
    elif ev["type"].endswith(".done"):
        apply_done(ev)
    elif ev["type"] == "response.completed":
        resp.update(ev.get("response") or {})
```

**流中断不可简单续传。** Responses 没有 Chat 那样统一的“从最后 chunk 继续”标准游标；如果 `response.completed` 未收到，可用 `response_id`/持久化对象查询或重建，但若 `store=false` 或 ZDR 环境，reasoning/工具中间态可能不可恢复。网关应区分：HTTP 未建立、流中连接断开、收到 `error`/`response.failed`/`response.incomplete`，分别采用查询、有限重建、不可重试错误策略。

### 横向主题

#### 多轮、缓存与截断

| 主题 | Chat Completions | Responses |
|---|---|---|
| 多轮状态 | 应用拼接完整 `messages` | `previous_response_id`（服务端）或手工回传完整 `input` |
| 历史最小单元 | `role` + content/tool_calls/tool_call_id | item，包括 reasoning/工具/内置调用 |
| assistant prefill | 最后一条 assistant 可带 `content`，配合 stop 续写；tool_calls 轮不可半改写 | 通过完整 item 输入；使用 prediction 等能力前需模型支持 |
| 上下文窗口 | 由模型定义；超出报错 | `truncation:"auto"` 丢中间；`disabled` 超窗报 400 |
| Prompt cache | `prompt_cache_key`/相同前缀/结构化前缀；命中体现于 `cached_tokens` | 相同；`prompt_cache_breakpoint` 显式断点，gpt-5.6+ 支持 `prompt_cache_options` |
| 无状态多轮 | 所有历史必须应用自管 | `store=false` 时应将前轮完整 `output` 重新作为 `input`，包括 reasoning item |

**缓存命中是业务结构的结果，不是只靠 header。** OpenAI 按稳定前缀匹配；系统提示、工具定义、历史顺序、多余空格、动态时间戳都会破坏缓存。网关若要承诺缓存键，应在日志中计算前缀哈希、记录 `prompt_tokens_details.cached_tokens`，并对 `instructions+tools+history` 的顺序做规范化。Chat `usage.prompt_tokens_details.cached_tokens` 与 Responses `usage.input_tokens_details.cached_tokens` 是同一观察口径，但字段名不同。

#### 工具调用全生命周期

##### 函数 schema 与 choice

```json
{
  "type": "function",
  "function": {
    "name": "get_weather",
    "description": "Get current weather for a location.",
    "strict": true,
    "parameters": {
      "type": "object",
      "properties": {"location": {"type": "string", "description": "City, Country"}},
      "required": ["location"],
      "additionalProperties": false
    }
  }
}
```

两协议 function schema 形态一致：`type:"function"`、`function.{name,description,parameters,strict?}`；Chat 的 `tools[i].function` 与 Responses 的 `tools[i].function` 都是嵌套对象，但 Responses 的 output 使用 `function_call` item，Chat 使用 `message.tool_calls`。Strict mode 要求 `additionalProperties:false`，且 `properties` 中所有字段都进入 `required`；这不是“建议”，否则 Structured/严格函数调用会失败。[6]

`tool_choice`：

| 值 | Chat | Responses | 语义 |
|---|---|---|---|
| `auto` | 默认（有 tools） | 默认 | 模型可选调用 0~N |
| `required` | 支持 | 支持 | 至少/必须调用工具 |
| `none` | 支持 | 支持 | 禁止工具调用 |
| `{type:"function",function:{name}}` | 支持 | 支持 | 强制特定函数 |
| `allowed_tools` 对象 | 部分模型/演进中 | 部分模型/演进中 | 限定可用工具子集；未证实前不应硬编码 |

##### 一次完整函数调用序列

请求 1（Chat）：

```json
{
  "model":"gpt-5.2",
  "messages":[
    {"role":"developer","content":"Use tools."},
    {"role":"user","content":"Weather in Paris?"}
  ],
  "tools":[{"type":"function","function":{"name":"get_weather","parameters":{"type":"object","properties":{"location":{"type":"string"}},"required":["location"],"additionalProperties":false}}}]
}
```

响应 1：

```json
{
  "choices":[{"index":0,"finish_reason":"tool_calls",
    "message":{"role":"assistant","content":null,
      "tool_calls":[{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{\"location\":\"Paris\"}"}}]}}]
}
```

回填请求 2：

```json
{
  "model":"gpt-5.2",
  "messages":[
    {"role":"developer","content":"Use tools."},
    {"role":"user","content":"Weather in Paris?"},
    {"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{\"location\":\"Paris\"}"}}]},
    {"role":"tool","tool_call_id":"call_1","content":"{\"temp_c\":15}"}
  ]
}
```

对应 Responses：请求 1 用 `input`，模型 `output` 出现 `function_call`；请求 2 将**原 `function_call` item 与新增 `function_call_output` item**追加到 `input`：

```json
{
  "model":"gpt-5.2",
  "input":[
    {"type":"message","role":"user","content":[{"type":"input_text","text":"Weather in Paris?"}]},
    {"type":"function_call","call_id":"call_1","name":"get_weather","arguments":"{\"location\":\"Paris\"}"},
    {"type":"function_call_output","call_id":"call_1","output":"{\"temp_c\":15}"}
  ]
}
```

官方函数调用指南明确：Chat 追加 assistant（含 tool_calls）+ tool 消息；Responses 将 `response.output` 加入 `input`，并为每个 function_call 增加 `function_call_output`，`call_id` 是关联主键。[6] **并行工具时，`call_id`/`tool_call_id` 必须一一对应；多轮循环必须保留调用顺序。**

##### 参数增量与幻觉工具处理

| 风险 | 直接原因 | 工程对策 |
|---|---|---|
| `arguments` JSON 分片 | 模型按 token 流输出 | 仅按 index/call_id 拼接，永不逐 delta 解析 |
| 空字符串 delta | 合法 keep-alive/边界 | 不能当作终止，也不能用 `if delta` 过滤 |
| 转义切断 | `"\\"`, emoji、base64、控制字符 | 保留原始 bytes；done 后再 JSON 解析 |
| index 不连续/乱序 | 并行工具、生命周期事件顺序 | 用 map 而非 list append；缺失 index 报警 |
| 未知函数 | 模型训练/提示漂移 | 明确拒绝、返回 tool error，或终止 agent；不得把 `name` 当白名单 |
| 重复 call_id | 重试/代理重复 | 网关内去重；不能假设上游唯一 |
| 工具执行失败 | 业务异常/超时 | 将错误作为 output；不要让模型无限循环 |

幻觉调用不存在的函数：网关应维护 `name→schema` 白名单；若响应 `name` 未注册，最佳实践是返回结构化错误 output，而非 HTTP 500——因为模型输出已经生成。对于严格业务，可在下一轮注入“工具不可用”消息并要求修正；对安全敏感工具必须服务端二次校验参数与权限。

#### 推理、思考链与多轮回传

| 字段/对象 | Chat Completions | Responses |
|---|---|---|
| 配置 | `reasoning_effort` | `reasoning:{effort,summary}` |
| 可见思考 | 无标准 `reasoning_content` 字段；DeepSeek 等第三方扩展不属于 OpenAI 规范 | `reasoning` item：`summary`/`encrypted_content`；完整 reasoning 可用性受限 |
| 计费 | `completion_tokens_details.reasoning_tokens` | `output_tokens_details.reasoning_tokens` |
| 多轮 | 以 messages 形式保存模型输出；具体保留策略由实现决定 | 无状态/store=false/ZDR 必须带回 reasoning item，`encrypted_content` 需 `include` 请求 |
| 下游兼容 | `reasoning_content` 是第三方字段，不应硬编码为官方字段 | 向 Chat 兼容层转换时，可将 reasoning 渲染为系统/developer hidden block，但会改变缓存/安全语义 |

官方 `include` 文档把 `reasoning.encrypted_content` 明确用于“stateless multi-turn / store=false / zero data retention”的 reasoning item 回传。[4] 这意味着网关**不得 base64 解码、修改、摘要替换、选择性删除其中的部分内容**：即使解密，内部结构也是未公开契约。若无法透传（例如下游仅支持 Chat `messages`），应保留为不可见、不可压缩、排序稳定的历史块；否则应拒绝降级并记录 reasoning loss。

#### 结构化输出

| 形态 | Chat `response_format` | Responses `text.format` |
|---|---|---|
| 普通文本 | `{type:"text"}` | `{type:"text",verbosity?}` |
| JSON mode | `{type:"json_object"}` | `{type:"json_object"}`（旧式，不保证 schema） |
| Strict schema | `{type:"json_schema",json_schema:{name,schema,strict?,description?}}` | `{type:"json_schema",schema:{...},strict?}` |
| 推荐范围 | GPT-4o+ 支持能力为准 | 同上；`json_schema` 优于 `json_object` |

官方 Structured Outputs 要求：对象必须 `additionalProperties:false`；严格函数/输出 schema 的 `required` 必须完整；最多约 100 嵌套层；字符串/名称/enum/const 总长度≤120,000；enum 合计≤1000；单个字符串 enum >250 项时总长度≤15,000。不支持 `allOf`、`not`、`dependentRequired`、`dependentSchemas`、`if/then/else`；微调模型额外不支持字符串 `minLength/maxLength/pattern/format`、数字边界、`patternProperties`、数组 `minItems/maxItems` 等。违反会返回错误，而不是静默忽略。[7]

**流式 structured output 必须等待 done/completed 再校验。** Chat `logprobs`/`top_logprobs` 与 JSON 解析互不影响；`content` delta 只是前缀。Responses 的 `output_text.delta` 同理。网关若提供“JSON 增量解析”，只能是体验层；正式契约应以 `response.output_text.done`/`response.completed` 或最终非流式对象为准。

#### 多模态输入与输出

| 模态 | Chat 输入 | Responses 输入 |
|---|---|---|
| 图像 | content-part `image_url`：`{url,detail}`；detail `auto`/`low`/`high` | `input_image`：`image_url`/`file_id`，`detail` |
| 音频 | `input_audio`：`{data,format}` + text parts | `input_audio`：`data`/`format`/`file_id` |
| 文件/PDF | file content part；支持矩阵随模型变化 | `input_file`：`file_id`/`filename`/`file_url`/`file_data` |
| 图像输出 | 通常不可直接作为文本生成输出 | `image_generation` 工具调用项 + partial/final 事件 |
| computer_use | 非原生 Chat 工具标准项 | `computer_call`/`computer_call_output`；截图按工具定义回传 |

图像 data URI 必须为 `data:<mediatype>;base64,<data>`，或使用 HTTPS/OpenAI file URL。`detail:high` 增加 token 成本，`auto` 由模型选择；网关应按业务预算做默认策略并记录实际 prompt token。官方明确 Chat 图片输入超过 8MB 会被丢弃，故在代理层做预检是合理的，但不得把“网关允许”误解为“模型保证处理”。[3]

### 错误、限流与重试

#### HTTP 状态码与典型 code

| 状态码 | `error.type`/常见 `code` | 触发场景 | 可重试 |
|---|---|---|---|
| 200 | — | 成功；SSE 也是 200 后持续流 | — |
| 400 | `invalid_request_error`；`invalid_value`、`string_above_max_length`、`context_length_exceeded`、`model_not_found`、`invalid_api_key`（部分）、`invalid_service_tier` | 参数非法、JSON schema 不支持、超上下文、`service_tier` 不允许 | **否**；修改请求后再试 |
| 401 | `authentication_error`；`invalid_api_key`、`incorrect_api_key`、`invalid_authentication`、IP/组织不匹配 | 密钥无效/撤销、组织/项目不匹配、IP allowlist | 否（可刷新凭证后重试） |
| 403 | `permission_denied_error`；`unsupported_country_region` | 地区/权限/合规限制 | 否 |
| 404 | `not_found_error` | `previous_response_id`、file、model 等不存在 | 否；查询参数错误 |
| 408 | `request_timeout` | 请求读取超时 | 可谨慎重试，需幂等风险处理 |
| 409 | `conflict_error` | 并发状态冲突（含 WebSocket/资源操作） | 视操作而定 |
| 422 | `unprocessable_entity_error` | 语义校验失败、参数组合不支持 | 否 |
| 429 | `rate_limit_error`/`insufficient_quota`；`rate_limit_exceeded`、`slow_down`、`tokens_rate_limit_exceeded`、`credit_balance_exhausted`、`organization/project_spend_limit_reached`、`organization_usage_limit_reached` | 请求/Token/队列速率；配额/额度/预算 | **区分**：纯限流可退避；额度/配额/预算不可自动重试 |
| 500 | `internal_server_error`/`server_error` | OpenAI 服务端错误 | 是，有限次数 |
| 503 | `service_unavailable_error`；`server_is_overloaded` | 模型过载、下游不可用 | 是，遵循 Retry-After |

官方 Error Codes 给出：`invalid_service_tier` 时 `param=service_tier`；401 可能源于撤销 key、组织/项目不匹配、权限不足；429 中 `credit_balance_exhausted`、`organization_spend_limit_reached`、`project_spend_limit_reached`、`organization_usage_limit_reached` 属于额度/预算，重试不会恢复。[5] 因此网关必须读取 `error.code` 与 `error.type`，不能用“所有 429 都指数退避”的单一规则。

典型错误响应：

```json
{
  "error": {
    "message": "The requested or resolved service tier is not allowed for this project.",
    "type": "invalid_request_error",
    "param": "service_tier",
    "code": "invalid_service_tier"
  }
}
```

```json
{
  "error": {
    "message": "You exceeded your current quota, please check your plan and billing details.",
    "type": "insufficient_quota",
    "code": "insufficient_quota"
  }
}
```

官方错误结构为 `{error:{message,type,param,code}}`；并非每个错误都同时具有 `param` 和 `code`。网关应把四字段全部记录，但向用户返回时进行脱敏/映射，避免泄露内部模型、文件路径或 prompt 片段。

#### 限流响应头

| Header | 含义 | 单位/格式 |
|---|---|---|
| `x-ratelimit-limit-requests` | 当前窗口最大请求数 | 整数 |
| `x-ratelimit-limit-tokens` | 当前窗口最大 input/output token 预算 | 整数 |
| `x-ratelimit-remaining-requests` | 剩余请求数 | 整数 |
| `x-ratelimit-remaining-tokens` | 剩余 token 预算 | 整数 |
| `x-ratelimit-reset-requests` | 请求桶重置时间 | `1s`、`6m0s`、`1d` 等持续时间 |
| `x-ratelimit-reset-tokens` | token 桶重置时间 | 同上 |
| `x-ratelimit-limit-project-tokens`/`remaining`/`reset` | 项目范围 token 限额 | 适用时出现 |
| `Retry-After` | 建议最小等待秒数 | 429 临时限流常见 |
| `x-request-id` | 请求唯一标识 | 所有排障必须记录 |

官方 Rate Limits 示例给出 `x-ratelimit-limit-tokens:150000`、`remaining-requests:59`、`remaining-tokens:149984`、`reset-requests:1s`、`reset-tokens:6m0s`，以及 project-token 头。[8] 持续时间格式是“数量+单位”，不是固定 epoch；网关应解析 `Ns/Nm0s/Nh/Nd` 并配合单调时钟，同时尊重 `Retry-After`。`limit-tokens` 与 usage tier 的关系是账户/项目容量策略，不是客户端可猜测常量，必须实时读取响应头。Chat 与 Responses 共享该传输层，不存在两套标准；唯一差异是请求是否携带 project/organization 和具体模型桶。

#### 推荐重试状态机

| 错误类别 | 默认行为 | 约束 |
|---|---|---|
| 网络/连接/408 | 指数退避 + 抖动，最多 2~5 次 | 长输出可能已计费；不可无限重试 |
| 429 `rate_limit`/`slow_down` | 优先 `Retry-After`，否则 backoff | 检查预算类 code；预算错误立即停止 |
| 429 额度/预算 | 不重试 | 告警并回传业务错误 |
| 500/503 | backoff，限次重试 | 遵守 `Retry-After`；区分幂等性 |
| 400/401/403/404/422 | 不重试 | 修正请求/权限/资源后再试 |
| SSE 中途断开 | 查询 response（Responses）或按业务重建 | Chat 无标准 cursor；不得假设未计费 |

官方指南建议：429 时把 `Retry-After` 作为最小值并加小随机抖动；SDK 已自动重试符合条件的限流错误；自建 HTTP 层在 header 缺失/无效时回退指数退避，并限制尝试次数与总耗时。[8] TypeScript SDK 默认 `maxRetries=2`，默认超时 10 分钟，可覆盖。[5] **网关应统一设置客户端 `maxRetries`，避免应用层 SDK 与边缘重试叠加成倍数放大。**

**LLM 没有原生幂等键。** 即使同一 `x-request-id` 也不应被网关假定为服务端去重；网络失败可能在模型已开始计费、已产生工具副作用、已部分写入存储后发生。正确设计是：短期连接层重试；业务层对“已持久化 response_id/tool result”做幂等；对不可重放的工具（付款、写库）使用工具服务端幂等键；流式客户端允许接收重复事件但输出层去重 `(output_index, content_index, item_id)`。

> OpenAI 双协议侧的网关实现坑位清单已整合至 **9.2 节**，此处不重复展开。

## 第三章　Anthropic Claude Messages API

> Claude 的核心特征：`system` 是顶层字段而非 message、`content` 是 block 数组、SSE 使用**具名事件 + block 生命周期**、`max_tokens` 必填。

### 文档依据与版本说明

> **核验基准：2026-09-13。**本文仅将本次已成功抓取的 Anthropic 官方 API 参考、Google AI for Developers / Google Cloud Vertex AI 官方文档作为硬证据。模型名称、模型级上下文长度、单字段最大值、beta 枚举会持续变化；网关应在请求校验层从 OpenAPI/JSON Schema 或运行时 `model` 元数据动态刷新，不能把本文中的模型特定数字视作长期承诺。

- **Anthropic API 基线版本头：`anthropic-version: 2023-06-01`**。官方 Getting Started 将其列为请求必需头，并给出该日期样例；它同时承担 API 版本选择，而非“仅鉴权”[12]。文档未声明“最新文档更新日期”的机器可读字段，本文不以推测日期替代。
- **重要版本与能力：2023-06-01 基础 Messages；工具调用（`tool_use`/`tool_result`）于 2024-10-01 前后成为稳定能力；Prompt Caching 于 2024-12-01 前后正式可用；extended thinking、引用、MCP 连接器、内置 web_search/code_execution 中部分为 beta，须通过 `anthropic-beta` 启用**。这些是官方能力页的历史性锚点；请求级生效与否应以实际请求返回的 400/404 为准。
- **Google 入口：Google AI Studio REST 为 `generativelanguage.googleapis.com/{version}/models/{model}:generateContent`；Vertex AI 为 `{location}-aiplatform.googleapis.com/v1/projects/{project}/locations/{location}/publishers/google/models/{model}:generateContent`**。Google AI Studio 可用 API Key 或 Bearer，Vertex 使用 OAuth Bearer，请求 URL、认证、字段命名与部分能力不完全同构[14]。本文以 **v1beta REST 参考**为主，部分成熟对象也会在 v1 中出现；**不要假定 v1beta 字段在 v1 中一定存在**。
- **Gemini 核心 API 版本：`v1beta` / `v1`；流式不是 body 中的 `stream:true`，而是 `?alt=sse`**。官方文本生成文档明确使用 `streamGenerateContent?alt=sse&key=...`[13]。
- **结论先行：三者不是“字段改名”关系，而是三个对象模型。**OpenAI 的 `messages[]/tool_calls/choices[]/stream delta`；Anthropic 的 `messages[]/content blocks/SSE 生命周期事件`；Gemini 的 `contents[]/parts/candidates[]`、完整对象增量流。网关若要无损转换，最低限度必须维护“原始 provider 对象 + 规范化内部 IR + 目标 provider 投影”，而不是试图让一个 JSON Schema 覆盖三套协议。

### 总体协议差异：对象模型决定网关架构

| 维度 | Anthropic Claude Messages | Google Gemini | 工程含义 |
|---|---|---|---|
| 主路径 | `POST /v1/messages` | `POST ...models/{model}:generateContent` | Claude 模型在 body；Gemini 在 URL |
| 输入消息 | `messages[]`，`role∈{user,assistant}` | `contents[]`，`role∈{user,model}` | `assistant↔model` 必须映射 |
| 系统提示 | 顶层 `system`，**不是 role** | 顶层 `systemInstruction`；历史页也出现 `system_instruction` snake_case | 不能以 OpenAI `system` message 硬塞 |
| 多模态/工具单元 | message 内 `content: string` 或 content block 数组 | content 内 `parts[]` | 数组可多类型共存，须按 `type`/`oneof` 分支 |
| 工具调用输出 | 模型：`tool_use` block；应用：`tool_result` block，位于 user message | 模型：`functionCall` part；应用：`functionResponse` part | ID、参数、结果对象均不同 |
| 流式 | 具名 SSE 事件、按 `index` 的 block 生命周期 | `GenerateContentResponse` 完整对象流，使用 `alt=sse` | 聚合算法完全不同 |
| 停止语义 | `stop_reason` 单值 | `finishReason` 每 candidate | OpenAI `finish_reason` 与二者均非一一映射 |
| 思考链 | `thinking`/`redacted_thinking`，`signature` | `thought`/`thoughtSignature` | 跨协议不能当作普通 text 透传 |

**协议兼容性判断：OpenAI 兼容网关可以做到“请求可路由、响应可归一化”，但做不到“任意 feature 无损双向映射”。**例如 Anthropic 的 `content_block_start/delta/stop` 和 Gemini 的 thought signature 强约束，会要求网关保存 provider 原生事件，而只向 OpenAI 客户端投影子集；反向请求则必须补齐原协议缺少的语义。

#### 1.1 端点、版本与请求头

| Header | 必填 | 取值/格式 | 规范含义 |
|---|---:|---|---|
| `content-type` | 是 | `application/json` | JSON 请求体 |
| `anthropic-version` | 是 | 日期串，如 `2023-06-01` | API 版本；官方明确其为必需[12] |
| `x-api-key` | 条件必填 | API Key | 直接使用 Workspace Key 时的认证方式 |
| `authorization` | 条件必填 | `Bearer <token>` | OAuth/工作台代理认证；与 key 通常二选一 | 
| `anthropic-beta` | 否 | 功能标识；多个用逗号分隔，如 `prompt-caching-2024-12-01,extended-thinking-2025-xx` | Beta 开关；精确取值必须按特性文档，不得猜测 |
| `anthropic-user-profile-id` | 否 | 字符串 | 代表最终用户，需要相应 beta |
| `request-id` / 响应 `request-id` | 否/响应必有 | 请求可自定义；响应可回显 | 官方错误页称每个响应均有 `request-id` header；排查必须记录 |

**`anthropic-version` 不是建议头。**官方 Getting Started 将 `anthropic-version` 标记为 Yes，并给出 `2023-06-01`；若网关丢弃或覆盖此头，下游行为取决于服务端默认版本，不能作为兼容性策略[12]。对代理层建议：请求携带客户端原始值；缺失时设置组织基线；将“版本不一致”列入可观测告警。

**`anthropic-beta` 是功能、而非模型版本。**请求级 beta 也可通过 body 字段表达，但官方推荐并更稳定的形式仍是 header。多值应**逗号分隔、不重复设置 header**（HTTP 折叠值受代理影响，实现不确定），且要绑定到具体请求，不可全局缓存。重试时必须复用同一 beta 集合，否则可能因能力/格式变化导致非幂等。

#### 1.2 `POST /v1/messages` 请求 body 全字段

```json
POST /v1/messages HTTP/1.1
Host: api.anthropic.com
Content-Type: application/json
x-api-key: sk-ant-...
anthropic-version: 2023-06-01
anthropic-beta: prompt-caching-2024-12-01

{
  "model": "claude-opus-4-5",
  "max_tokens": 4096,
  "system": [
    {"type":"text","text":"You are a helpful analyst.","cache_control":{"type":"ephemeral"}}
  ],
  "messages": [
    {"role":"user","content":"Summarize the attached report."}
  ],
  "temperature":1,
  "stream":false,
  "tools":[
    {"name":"get_weather","description":"Get current weather","input_schema":{"type":"object","properties":{"location":{"type":"string"}},"required":["location"]}}
  ],
  "tool_choice":{"type":"auto"}
}
```

| 字段 | 类型 | 必填 | 默认 | 取值范围/约束 | 说明 |
|---|---|---:|---:|---|---|
| `model` | string | 是 | — | 有效模型 ID | 精确模型影响能力、窗口、计费 |
| `max_tokens` | integer | **是** | — | `≥1`；上限随模型 | **硬输出上限，不是选项**；缺失即非法 |
| `messages` | array<MessageParam> | 是 | — | 交替 user/assistant；最多官方称 100,000 | 连续同 role 可能被合并 |
| `system` | string 或 array<SystemBlock> | 否 | — | text、cache_control | **顶层字段，不能用 role=system** |
| `temperature` | number | 否 | 1 | 通常 [0,1]；启用 thinking 时必须为 1 | 高 temperature 不保证有效 |
| `top_p` | number | 否 | — | nucleus；thinking 时不可与 temperature 同时指定 | 与 OpenAI 语义接近但不可假定相等 |
| `top_k` | integer | 否 | — | Anthropic 特有 | OpenAI 请求无对应字段 |
| `stop_sequences` | array<string> | 否 | — | 每项字符串 | 命中后 `stop_reason=stop_sequence` |
| `stream` | boolean | 否 | false | true/false | 切换 SSE 协议 |
| `metadata` | object | 否 | — | `{user_id}` | 用于安全/审计；不要放敏感 PII |
| `tools` | array<Tool> | 否 | — | 模型有限制 | `name/description/input_schema` |
| `tool_choice` | object | 否 | auto | `auto/any/tool/none` | 见 1.6 |
| `thinking` | object | 否 | disabled | `{type:"enabled",budget_tokens}`；也可 disabled/adaptive | 与 temperature、max_tokens 有约束 |
| `service_tier` | string | 否 | — | 如 `auto`、`standard_only`（以运行时枚举为准） | 容量/数据驻留选择 |
| `mcp_servers` | array | 否 | — | beta | 远程 MCP 配置；不得伪造为普通 tools |
| `container` | string | 否 | — | beta/模型相关 | 代码执行容器标识 |
| `context_management` | object | 否 | — | beta | 上下文自动管理；结构按特性版本 |

**`max_tokens` 必填是 Claude 最容易被 OpenAI 网关遗漏的错误。**OpenAI 常有模型默认 `max_completion_tokens`，但 Claude Messages 请求中明确需要 `max_tokens`；网关的规范化层若收到 OpenAI 请求，应依据模型配置推导默认值并写入 body，而不是透传缺失值。

**`system` 可为字符串或 block 数组。**字符串是单 text block 的简写。数组可携带 cache_control、citation 配置；当 block 数组中出现 cache_control 时，数组形式不可再压缩成字符串。

#### 1.3 Content block 类型：请求侧

| Block `type` | 关键字段 | 使用位置/约束 |
|---|---|---|
| `text` | `text:string`、`cache_control?` | user/assistant；请求/响应 |
| `image` | `source:{type:"base64",media_type,data}` 或 url/file；`cache_control?` | 仅请求；url/file 在 count_tokens 可能不被支持 |
| `document` | `source:{...}`、PDF；可含 `citations` 配置 | user；不是 OpenAI `file` 的简单等价物 |
| `tool_use` | `id`、`name`、`input`（已解析对象）、`type` | **assistant message**；多块可并行 |
| `tool_result` | `tool_use_id`、`content:string\|blocks`、`is_error?:boolean`、`cache_control?` | **user message**；回填工具执行结果 |
| `thinking` | `thinking`、`signature` | assistant；启用 extended thinking 的历史回传 |
| `redacted_thinking` | `data` | assistant；不透明加密块，**必须原样回传** |
| `search_result` / 服务器工具结果块 | 特定 `tool_use_id`/content | 内置工具；普通应用很少手工构造 |
| `server_tool_use` | `id/name/input` | 模型请求服务器工具；响应侧再配对结果块 |

**`tool_result` 的 role 规则是网关常见 bug。**模型生成 `tool_use` 后，应用**追加一条 `role:"user"` message**，其 `content` 是包含 `tool_result` 的数组；不能把 `tool_result` 塞进 assistant message，也不能拆成“每个结果一个 user turn”而破坏顺序。若同一 assistant turn 有 N 个 `tool_use`，通常一条 user message 的 `content` 中放 N 个对应 `tool_result`。

#### 1.4 Content block 类型：响应侧

响应 `content` 是 block 数组；客户端必须按 `type` 分支，不能假设 index 0 是 text、也不能假设 `text` 字段永远存在。

| 响应 `type` | 字段 | 说明 |
|---|---|---|
| `text` | `text` | 正常文本 |
| `thinking` | `thinking`、`signature` | 推理文本与校验签名 |
| `redacted_thinking` | `data` | 不可解码，不能丢弃、修改 |
| `tool_use` | `id`、`name`、`input`（object） | **`input` 是对象，不是 JSON 字符串** |
| `server_tool_use` | `id/name/input` | 内置工具调用；后续结果块由服务生成 |

**thinking 多轮回传是强约束。**官方 token counting 页面对 thinking 的示例显示：前一轮 assistant 的 `thinking` 块必须按原顺序、未修改地返回，并保留 `signature`；否则可能产生 400。无法取得原签名的历史不应尝试重建 thinking，而应开启新回合或关闭 thinking。Redacted thinking 的 `data` 为不透明密文，压缩/重序列化可能破坏它。

#### 1.5 Prompt Caching：断点、TTL 与计费

| 规则 | 官方事实 | 网关动作 |
|---|---|---|
| 显式断点 | `cache_control:{type:"ephemeral"}` 放在 block 上 | 仅允许在可缓存位置设置 |
| 自动缓存 | 顶层 `cache_control` 由服务把断点放到最后一个可缓存 block | 多轮历史增长时适合默认策略 |
| 可缓存前缀 | tools、system、messages，按请求顺序到断点 | 前缀必须完全一致才可命中 |
| TTL | 默认 5 分钟；可选 1 小时 | 每次读写刷新，但不应在网关虚构 1h |
| 断点数 | 当前公开文档常见上限为 4 个；最小可缓存 token 数以模型页为准 | 超出应拒绝或规范化，而非静默丢弃 |
| 计费 | `cache_creation_input_tokens`、`cache_read_input_tokens` 分别报告 | 命中时按读缓存 token 计费，创建/读费率不同 |

**前缀缓存的关键不是“相同 prompt”，而是“从请求起点到断点的字节/block 完全一致”。**system、tools、messages 的顺序、空白、字段排序、cache_control 位置变化都可能使命中失效。网关若要代理缓存，建议：保留 provider 原生请求序列化；仅对用户可控前缀计算 cache key；禁止把请求时间戳、request-id、动态 tool_result 插入缓存前缀。

#### 1.6 工具定义与 `tool_choice`

| Anthropic `tools[]` | OpenAI 对照 | 说明 |
|---|---|---|
| `name` | `function.name` | 唯一标识 |
| `description` | `function.description` | 模型选择依据 |
| `input_schema` | `function.parameters` | **都是 JSON Schema，但约束集不同，不能直接复制** |
| `cache_control` | 无 | Claude block 级缓存 |

```json
{
 "tools":[{"name":"get_weather","description":"Get current weather","input_schema":{"type":"object","properties":{"location":{"type":"string"}},"required":["location"]}}],
 "tool_choice":{"type":"auto"}
}
```

| `tool_choice.type` | 语义 | OpenAI 近似 |
|---|---|---|
| `auto` | 模型自主选择工具或文本 | `auto` |
| `any` | 必须调用至少一个工具 | `required` |
| `tool` | 必须调用指定工具，`{name}` | `required` + 单工具约束 |
| `none` | 不调用工具 | `none` |

**这不是完全等价。**OpenAI 的 `tool_choice: {type:"function",function:{name}}` 与 Claude `{"type":"tool",name}` 接近；但 Claude `any` 允许从多个工具中选一个，OpenAI `required` 仅要求至少一次调用。翻译时应保留“允许集”和“强制指定名称”两个维度，而不是只映射字符串。

#### 1.7 非流式响应

```json
{
  "id":"msg_01...",
  "type":"message",
  "role":"assistant",
  "model":"claude-opus-4-5",
  "content":[
    {"type":"text","text":"The capital is Paris."},
    {"type":"tool_use","id":"toolu_01","name":"get_weather","input":{"location":"Paris"}}
  ],
  "stop_reason":"tool_use",
  "stop_sequence":null,
  "usage":{
    "input_tokens":120,
    "output_tokens":38,
    "cache_creation_input_tokens":0,
    "cache_read_input_tokens":512,
    "server_tool_use":{"web_search_requests":0}
  }
}
```

| 响应字段 | 类型 | 说明 |
|---|---|---|
| `id` | string | message ID |
| `type` | `"message"` | 固定值 |
| `role` | `"assistant"` | **恒为 assistant** |
| `model` | string | 实际模型 |
| `content` | array | block 列表 |
| `stop_reason` | enum | 见下表 |
| `stop_sequence` | string? | 仅 stop_sequence 命中时有值 |
| `usage.input_tokens` | int | 本次输入 token |
| `usage.output_tokens` | int | 输出 token |
| `usage.cache_creation_input_tokens` | int | 首次写入缓存部分 |
| `usage.cache_read_input_tokens` | int | 读命中部分 |
| `usage.cache_creation` | object | 含 5m/1h 细项（版本相关） |
| `usage.server_tool_use` | object | 服务器工具用量 |

| `stop_reason` | 含义 | OpenAI 映射（建议） |
|---|---|---|
| `end_turn` | 正常结束 | `stop` |
| `max_tokens` | 达到 `max_tokens` | `length` |
| `stop_sequence` | 命中停止串 | `stop` |
| `tool_use` | 需要调用工具 | `tool_calls` |
| `pause_turn` | 需要等待/中断恢复 | `tool_calls` 或自定义 `pause` |
| `refusal` | 策略拒绝 | `content_filter`/自定义 |

#### 1.8 流式 SSE：具名事件而非纯 data

`Content-Type: text/event-stream`。每个事件通常有 `event:<type>` 与 `data:<json>` 两行；JSON `data` 的 `type` 与 SSE `event` 名称对应。标准生命周期为：

1. `message_start`
2. 对每个 content block：`content_block_start` → 0..n `content_block_delta` → `content_block_stop`
3. 一个或多个 `message_delta`
4. `message_stop`
5. 中间可插入 `ping`；错误可为 `error`

| 事件 | payload 关键字段 | 含义 |
|---|---|---|
| `message_start` | `message:{id,type,role,model,content:[],usage}` | 初始化；`usage.input_tokens` 通常在此 |
| `content_block_start` | `index`、`content_block` 初始块 | 建立 block 槽位；文本/工具/思考类型在此确定 |
| `ping` | — | 保活；不可当消息完成 |
| `content_block_delta` | `index`、`delta` | 增量；见下表 |
| `content_block_stop` | `index` | 该 block 完成 |
| `message_delta` | `delta:{stop_reason,stop_sequence}`、`usage:{output_tokens,...}` | **最终输出 usage 常在此** |
| `message_stop` | — | 流结束 |
| `error` | `{type:"error",error:{type,message}}` | 中途错误；即使 HTTP=200 也可能发生 |

**delta 子类型：**

| `delta.type` | 字段 | 拼接方式 |
|---|---|---|
| `text_delta` | `text` | 按 `index` 追加 |
| `thinking_delta` | `thinking` | 按 index 追加；不能混入 text |
| `signature_delta` | `signature` | 追加到对应 thinking block；先有 thinking 后有签名 |
| `input_json_delta` | `partial_json` | **字符串片段拼接，不是 JSON 增量对象** |
| `citations_delta` | citation 部分 | 按文档结构追加 |

```http
event: message_start
data: {"type":"message_start","message":{"id":"msg_01","type":"message","role":"assistant","model":"claude-opus-4-5","content":[],"usage":{"input_tokens":25,"output_tokens":0}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me analyze"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"..."}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01","name":"get_weather","input":{}}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"location\":"}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"Paris\"}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":1}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":120}}

event: message_stop
data: {"type":"message_stop"}
```

**`input_json_delta` 的拼接陷阱：不要在每片到达时 `JSON.parse`。**正确方式是按 `index` 维护 `text`/`thinking`/`signature` 缓冲区和一个 `partial_json` 字符串；仅在 `content_block_stop` 后，或业务确需提前调用时，使用容错解析（如 `json5`/状态机/截断安全解析）。官方示例明确展示工具调用的 `name/id` 在 start，参数通过 `partial_json` 渐进到达[10]。

**伪代码：**

```text
blocks = []
for each SSE event:
  if event == "message_start":  req_id, input_usage = message.usage
  if event == "content_block_start": blocks[index] = content_block
  if event == "content_block_delta":
      d = delta
      if d.type == "text_delta": blocks[index].text += d.text
      elif d.type == "input_json_delta": blocks[index].partial_json += d.partial_json
      elif d.type == "thinking_delta": blocks[index].thinking += d.thinking
      elif d.type == "signature_delta": blocks[index].signature += d.signature
  if event == "content_block_stop":
      if blocks[index].type == "tool_use":
          blocks[index].input = safe_parse(blocks[index].partial_json) # 可能延迟到 stop
  if event == "message_delta": final_reason = delta.stop_reason; output_usage = usage
  if event == "error": raise provider_error
emit OpenAI-compatible chunk mapping each state transition; on stream end, build final Message.
```

**与 OpenAI 的关键差异：**OpenAI 通常发送 `choices[0].delta`，客户端累加 text/tool_calls 片段；Anthropic 有明确的 `index`、block 开始/结束与不同 delta 子类型，`usage` 还被拆到 `message_start` 和 `message_delta`。若网关对外暴露 OpenAI 流，必须自己合成 `usage`、`finish_reason`，并在关闭 SSE 前发送等效 `done` 事件；反向代理则要保留原始 Anthropic 事件，不能只转发 text。

#### 1.9 工具调用完整生命周期

```http
POST /v1/messages
{
 "model":"claude-opus-4-5","max_tokens":1024,
 "messages":[{"role":"user","content":"What is the weather in Paris?"}],
 "tools":[{"name":"get_weather","description":"Get current weather","input_schema":{"type":"object","properties":{"location":{"type":"string"}},"required":["location"]}}],
 "tool_choice":{"type":"auto"}
}
```

响应包含 `tool_use`：

```json
{"role":"assistant","content":[{"type":"tool_use","id":"toolu_01","name":"get_weather","input":{"location":"Paris"}}]}
```

应用执行后追加 user message：

```json
{
 "messages":[
  {"role":"user","content":"What is the weather in Paris?"},
  {"role":"assistant","content":[{"type":"tool_use","id":"toolu_01","name":"get_weather","input":{"location":"Paris"}}]},
  {"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_01","content":"18°C, sunny"}]}
 ]
}
```

- **成对规则**：每个 `tool_use_id` 在后续 user turn 中应有对应 `tool_result`；同一 assistant turn 的多个 tool_use 可在一条 user message 中返回。
- **并行调用**：assistant `content` 有多个 `tool_use` 块；执行顺序应用侧决定，但结果顺序须可关联到 `tool_use_id`。
- **错误自愈**：`is_error:true` 的内容仍应结构化返回；模型可能基于错误重试或修改参数。
- **服务器工具/MCP**：`web_search`、`code_execution` 等返回服务器工具结果块；`mcp_servers` 由 beta 头启用，调用命名与结果回传遵循具体特性版本。未经实测时，网关应透传原始块而不是“归一化”为普通 tool_result。

#### 1.10 `/v1/messages/count_tokens`

`POST /v1/messages/count_tokens`，鉴权/版本头同 Messages。Body 支持 `model`、`system`、`messages`、`tools` 等，响应核心为 `{"input_tokens":N}`。官方明确：

- token 数是**估算**，与实际创建 message 时可能略有差异；
- 自动系统优化加入的 token **不计入计费**；
- 部分能力（某些服务器工具、MCP、url/file 媒体源）在 count 时返回 `invalid_request_error`；图像/PDF 应发 base64 计数[11]。

网关可用它做前置预算、路由与截断；但不能把返回值当作缓存 key 或严格配额计数。

#### 1.11 错误、限流与重试

**错误信封：**

```json
{"type":"error","error":{"type":"not_found_error","message":"..."},"request_id":"req_011CSHoEeqs5C35K2UUqR7Fy"}
```

| HTTP | `error.type` | 处理 |
|---:|---|---|
| 400 | `invalid_request_error` | 不重试；修正参数 |
| 401 | `authentication_error` | 检查 key；不重试同一 key |
| 402 | `billing_error` | 支付/配额，不应盲重试 |
| 403 | `permission_error` | 权限/workspace |
| 404 | `not_found_error` | URL/资源 |
| 409 | `conflict_error` | 状态冲突；谨慎重试 |
| 413 | `request_too_large` | 缩小/截断；Messages/Count Token 文档上限 32MB |
| 429 | `rate_limit_error` | 令牌桶、月度花费上限、workspace 上限都可能触发 |
| 500 | `api_error` | 幂等请求可退避重试 |
| 504 | `timeout_error` | 长请求优先流式；仅幂等重试 |
| 529 | `overloaded_error` | 退避重试 |

**限流头：**

| Header | 含义 |
|---|---|
| `anthropic-ratelimit-requests-limit` | 请求容量 |
| `anthropic-ratelimit-requests-remaining` | 剩余请求 |
| `anthropic-ratelimit-requests-reset` | RFC 3339 全量恢复时间 |
| `anthropic-ratelimit-tokens-limit/-remaining/-reset` | 输入 token 维度 |
| `anthropic-ratelimit-input-tokens-*` | 输入 token 明确维度 |
| `anthropic-ratelimit-output-tokens-*` | 输出 token 维度 |
| `retry-after` | **秒数，不是 HTTP-date** |

**限流头不能当作实时精确账本。**令牌桶会持续补充；官方说明组织级限额可由 Workspace 自定义，额度不是固定整点清零。429 时头可能更新，突发配额也可能使头与瞬时用量不一致；网关应使用“剩余量 + reset + Retry-After”做**建议性**限流，而非强一致计数器。

**重试策略：**仅对 408、429、5xx、流中 `overloaded_error` 等瞬态错误退避；默认指数退避并加 jitter，设置上限。429 中“月度花费上限”可能没有 `retry-after`，早重试必败。Anthropic 官方 SDK 默认两次指数退避并尊重 `retry-after`；长请求重试需确保原请求幂等：建议生成客户端 `request-id`、固定 `anthropic-beta`、固定 body，但**不要**在 tool 执行后无脑重放，避免工具副作用。

## 第四章　Google Gemini API

> Gemini 的核心特征：`role` 只有 `user`/`model`、模型名在 URL 里、靠 `?alt=sse` 而非 body 开启流式、流式 chunk 是**完整响应对象而非差分**、存在"HTTP 200 但 `candidates` 为空"的安全拦截形态。

#### 2.1 两套入口与版本

| 入口 | Base URL | 鉴权 | 适用 |
|---|---|---|---|
| Google AI Studio | `https://generativelanguage.googleapis.com/{version}` | `x-goog-api-key`、API Key query `?key=` 或 Bearer | 快速使用；REST 参考主要此处 |
| Vertex AI | `https://{location}-aiplatform.googleapis.com/v1` | `Authorization: Bearer $(gcloud auth print-access-token)` | GCP 项目、服务账号、企业治理 |

官方生成内容文档给出 Vertex 的 `Authorization: Bearer` 示例；Google AI Studio 快速开始示例使用 `x-goog-api-key` 与 `?key=`[14]。**对自研网关的最稳妥实现：每个上游配置显式 `auth_mode`，不自动在 header 与 query 之间猜测。**

核心方法：

- `models.generateContent`：`GenerateContentRequest → GenerateContentResponse`
- `models.streamGenerateContent`：**请求相同 body，URL 增加 `?alt=sse`**
- `models.countTokens`
- `models.embedContent`
- `cachedContents.create`/交互式缓存；Interactions/Live API 是更新形态，与普通 `generateContent` 的工具托管、状态历史模型不同，**不把 Live API 的 WebSocket/实时协议当作 REST SSE**。

#### 2.2 `generateContent` 请求 body 全字段

```json
POST /v1beta/models/gemini-3-pro:generateContent?key=${API_KEY HTTP/1.1
Content-Type: application/json

{
 "contents":[
  {"role":"user","parts":[{"text":"Compare these reports."},{"inline_data":{"mime_type":"application/pdf","data":"<base64>"}}]}
 ],
 "systemInstruction":{"role":"system","parts":[{"text":"You are a concise analyst."}]},
 "generationConfig":{
  "temperature":0.7,"topP":0.95,"topK":40,"maxOutputTokens":2048,"candidateCount":1,
  "stopSequences":["###"],"responseMimeType":"application/json","responseSchema":{"type":"object"},
  "thinkingConfig":{"thinkingBudget":4096,"thinkingLevel":"MEDIUM"},
  "presencePenalty":0,"frequencyPenalty":0,"seed":42,"responseLogprobs":false
 },
 "tools":[{"function_declarations":[{"name":"get_weather","description":"Get current weather","parameters":{"type":"object","properties":{"location":{"type":"string"}},"required":["location"]}}]}],
 "toolConfig":{"functionCallingConfig":{"mode":"AUTO","allowedFunctionNames":[]}},
 "safetySettings":[{"category":"HARM_CATEGORY_HARASSMENT","threshold":"BLOCK_NONE"}],
 "cachedContent":"projects/p/locations/l/cachedContents/c"
}
```

| 字段 | 类型 | 必填 | 说明 |
|---|---|---:|---|
| `contents` | array<Content> | 是 | 当前对话；单轮也须数组 |
| `contents[].role` | `"user"`/`"model"` | 条件 | 通常应交替；省略时按模型/历史规则 |
| `contents[].parts` | array<Part> | 是 | oneof 联合体 |
| `systemInstruction` | Content | 否 | 顶层系统提示；`role` 被忽略 |
| `generationConfig.temperature` | number | 否 | 采样温度 |
| `generationConfig.topP` | number | 否 | nucleus |
| `generationConfig.topK` | number | 否 | top-k |
| `generationConfig.maxOutputTokens` | integer | 否 | 对应 Claude `max_tokens`、OpenAI `max_completion_tokens` |
| `generationConfig.candidateCount` | integer | 否 | **1–20**；Anthropic 无 `n` |
| `generationConfig.stopSequences` | array<string> | 否 | 停止串 |
| `generationConfig.responseMimeType` | string | 否 | 如 `application/json` |
| `generationConfig.responseSchema` | object | 否 | JSON Schema 子集 |
| `generationConfig.thinkingConfig` | object | 否 | `thinkingBudget`/`thinkingLevel` |
| `generationConfig.presencePenalty` | float | 否 | Gemini 特有 |
| `generationConfig.frequencyPenalty` | float | 否 | Gemini 特有 |
| `generationConfig.seed` | integer | 否 | 确定性努力，不保证可复现 |
| `generationConfig.responseLogprobs` | bool | 否 | 启用 logprobs；`logprobs` 对部分模型弃用 |
| `generationConfig.responseModalities` | array | 否 | 如 `["AUDIO"]`；模型相关 |
| `generationConfig.speechConfig` | object | 否 | 语音/多 speaker |
| `tools` | array<Tool> | 否 | `function_declarations` 或内置工具 |
| `toolConfig.functionCallingConfig` | object | 否 | `mode`、`allowedFunctionNames` |
| `safetySettings` | array<SafetySetting> | 否 | category/threshold |
| `cachedContent` | string | 否 | 缓存资源名 |
| `labels` | map<string,string> | 否 | 请求标签 |

**命名风格风险提示：**JSON REST 常用 `systemInstruction`、`generationConfig`、`responseMimeType`；某些历史文档/SDK 出现 snake_case。代理收到未知字段时，应基于目标 API 版本的 schema 归一化，不要机械把 OpenAI `system` 复制到 `system_instruction`。

#### 2.3 Part 类型全表

| Part oneof | 字段 | 承载内容 |
|---|---|---|
| `text` | string | 文本 |
| `inline_data`/`inlineData` | `mime_type`+`data`（base64） | 图片、音频、PDF 等原始字节 |
| `file_data`/`fileData` | `file_uri`+`mime_type` | Files API 引用 |
| `functionCall` | `id?`、`name`、`args`（对象） | 模型请求函数 |
| `functionResponse` | `id?`、`name`、`response`（对象） | 应用返回结果 |
| `executableCode` | 代码对象 | 代码执行 |
| `codeExecutionResult` | 结果对象 | 代码执行结果 |
| `thought` | boolean | 思考 part 标识 |
| `thoughtSignature` | string | 签名 |
| `videoMetadata` | 起止、fps | 视频段 |
| `mediaResolution` | enum | 媒体解析度 |

**`functionResponse.response` 是对象，不是字符串。**这是与 OpenAI `tool` message 的内容字符串、Claude `tool_result.content` 字符串/block 最显著的差异。网关规范化建议：保留 `{name,response:{...}}`；向 OpenAI 投影时序列化 `response`；反向构造 Gemini 时必须解析为 JSON 对象。

#### 2.4 非流式响应与“空 candidates”

```json
{
 "candidates":[
  {"index":0,
   "content":{"role":"model","parts":[{"text":"Paris is the capital."}]},
   "finishReason":"STOP",
   "safetyRatings":[{"category":"HARM_CATEGORY_HARASSMENT","probability":"NEGLIGIBLE","blocked":false}],
   "citationMetadata":{"citations":[]}
  }
 ],
 "promptFeedback":{"blockReason":null,"safetyRatings":[]},
 "usageMetadata":{"promptTokenCount":30,"candidatesTokenCount":10,"totalTokenCount":40,"cachedContentTokenCount":0,"thoughtsTokenCount":0,"toolUsePromptTokenCount":0},
 "modelVersion":"gemini-3-pro","responseId":"..."
}
```

| 字段 | 含义 |
|---|---|
| `candidates[]` | 候选；可能因安全/提示问题为空 |
| `candidates[].content` | `role`+`parts` |
| `candidates[].finishReason` | 停止原因；流式通常在最终 chunk |
| `candidates[].safetyRatings` | 类别/概率/是否拦截 |
| `candidates[].citationMetadata` | 引用起止、URI、标题等 |
| `candidates[].groundingMetadata` | Search grounding、chunks、queries |
| `candidates[].avgLogprobs`/`logprobsResult` | 对数概率 |
| `promptFeedback` | 提示侧过滤；`blockReason` 存在时常无 candidates |
| `usageMetadata` | token 使用；见下表 |
| `modelVersion` | 实际模型版本 |
| `responseId` | 响应标识 |

**Gemini 重大差异：`candidates` 可以是空数组。**官方明确“只有 prompt 有问题时才返回空候选，此时检查 promptFeedback”[14]。OpenAI 网关若只读取 `choices[0]`，必须改为：先判断 `candidates.length===0`，再由 `promptFeedback.blockReason` 转换为 OpenAI `error` 或 `finish_reason:"content_filter"`。不能从 `null` content 中读取 text。

| `finishReason` | 含义 |
|---|---|
| `STOP` | 自然停止/停止序列 |
| `MAX_TOKENS` | `maxOutputTokens` 耗尽 |
| `SAFETY` | 安全策略；content 可能空 |
| `RECITATION` | 引用/复述 |
| `OTHER` | 其他 |
| `BLOCKLIST` | 禁用词 |
| `PROHIBITED_CONTENT` | 禁止内容 |
| `SPII` | 敏感 PII |
| `MALFORMED_FUNCTION_CALL` | 函数调用不可解析 |
| `LANGUAGE`/`IMAGE_*`/`NO_IMAGE`/`UNEXPECTED_TOOL_CALL`/`TOO_MANY_TOOL_CALLS`/`MISSING_THOUGHT_SIGNATURE`/`ESCALATION`/`MALFORMED_RESPONSE` | 模型/图像/工具/签名特定终止 |

`usageMetadata` 关键字段：`promptTokenCount`、`cachedContentTokenCount`、`candidatesTokenCount`、`toolUsePromptTokenCount`、`thoughtsTokenCount`、`totalTokenCount`、`promptTokensDetails`。官方类型说明称 `totalTokenCount` 为 prompt、candidates、tool_use prompt、thoughts 之和[17]。

#### 2.5 `alt=sse` 流式：完整对象增量

```http
POST /v1beta/models/gemini-3-pro:streamGenerateContent?alt=sse&key=$KEY HTTP/1.1
Content-Type: application/json

{"contents":[{"role":"user","parts":[{"text":"Count 1 to 5."}]}]}
```

```http
event: message
data: {"candidates":[{"content":{"parts":[{"text":"1"}]},"index":0}]}

event: message
data: {"candidates":[{"content":{"parts":[{"text":" 2"}]},"index":0}]}

event: message
data: {"candidates":[{"content":{"parts":[{"text":" 3 4 5"}]},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":12,"totalTokenCount":22}}
```

**Gemini 每个 chunk 是完整 `GenerateContentResponse` 的子集，不是“自首个 chunk 以来的累计快照”，也不是 OpenAI 那样的独立 delta。**官方说明：`streamGenerateContent` 返回“a stream of GenerateContentResponse instances”[14]。常见实现中 text 是追加式；但对于 function call、thought、code execution，必须以 part 类型和 chunk 出现顺序做结构化合并，不能简单把 `parts[]` 整个替换。

**聚合伪代码：**

```text
candidates = {}
for chunk in SSE:
  for cand in chunk.candidates:
    slot = candidates.setdefault(cand.index, {parts:[]})
    for part in cand.content.parts:
      merge_part(slot.parts, part)   # text append; functionCall/code keep last or accumulate by id
    cand.finishReason && (slot.finishReason = cand.finishReason)
    cand.safetyRatings && (slot.safetyRatings = cand.safetyRatings)
  chunk.usageMetadata && (usage = chunk.usageMetadata)
  chunk.promptFeedback && (promptFeedback = chunk.promptFeedback)
```

**流式中必须特别处理：**`thoughtSignature`、多个 `functionCall` 的部分参数、`executableCode`/`codeExecutionResult` 的顺序、最终 `finishReason` 与 `usageMetadata`。Gemini 文档页当前未承诺每个 chunk 都设置 `event:` 行；网关应同时支持有/无 `event:` 前缀，但始终按 SSE `data:` 解析 JSON。安全拦截可能使后续 chunk 无 content；不要因 text 为空提前关闭 OpenAI 流。

#### 2.6 多轮、思考签名与 Context Caching

**thought signature 是强约束。**Part 可用 `thought:true` 标记思考内容，`thoughtSignature` 用于校验。官方请求 schema 将其列为 Part 字段[14]。跨轮回传时必须保留包含 thought/thoughtSignature 的 part，**不能把 thinking 简化为普通 text 或删除 signature**。缺失签名可能导致 `MISSING_THOUGHT_SIGNATURE`/请求错误；因此 OpenAI 兼容 API 不应把 thought 暴露在普通 `assistant` message 中再回读为 text。

**`contents` 顺序：**历史页建议使用 user/model 交替[13]；REST schema 称 `role` 支持 user/model，但 systemInstruction 的 role 被忽略。工具回合的一般模式：

1. model part `functionCall`
2. 下一个 user part `functionResponse`
3. 必要时继续 model、user

不能把 `functionResponse` 放入 `model` role；同一回合允许多 function call/response，但须按模型要求配对。

**Context Caching：**Google AI 文档区分隐式与显式缓存。Gemini 2.5+ 默认隐式缓存，不保证节省；显式缓存创建 `cachedContents` 资源，默认 TTL **1 小时**，后续请求用 `cachedContent` 引用。最小隐式缓存 token 数有模型差异（官方文档列 Gemini 2.5 Flash 1024、2.5 Pro 4096）[22]。**这与 Anthropic 的 5 分钟 prompt cache TTL 相似但不兼容**，网关不能复用“cache_control block”转换逻辑。

#### 2.7 工具全生命周期

```http
POST /v1beta/models/gemini-3-pro:generateContent?key=$KEY
{
 "contents":[
  {"role":"user","parts":[{"text":"Weather in Paris?"}]},
  {"role":"model","parts":[{"functionCall":{"name":"get_weather","args":{"location":"Paris"}}}]},
  {"role":"user","parts":[{"functionResponse":{"name":"get_weather","response":{"temperature":18}}}]}
 ],
 "tools":[{"function_declarations":[{"name":"get_weather","description":"Get current weather","parameters":{"type":"object","properties":{"location":{"type":"string"}},"required":["location"],"additionalProperties":false}}]}],
 "toolConfig":{"functionCallingConfig":{"mode":"ANY","allowedFunctionNames":["get_weather"]}}
}
```

| 项目 | 规范 |
|---|---|
| `function_declarations[].parameters` | OpenAPI 3.0 子集；`type/properties/required/enum/description` 可用，`additionalProperties`、`$ref` 等支持随版本变化 |
| `functionCallingConfig.mode` | `AUTO`（默认）/ `ANY` / `NONE`；`MODE_UNSPECIFIED` 不应主动使用 |
| `allowedFunctionNames` | 仅 `ANY`/`VALIDATED` 有意义 |
| `functionCall.args` | **对象** |
| `functionResponse.response` | **对象** |
| `stream_function_call_arguments` | Vertex 函数调用文档的可选布尔配置；决定是否增量 |

**`MALFORMED_FUNCTION_CALL` 不是让网关忽略工具的理由。**它可能由 schema/参数不匹配、响应格式错误、流式拼接问题引起。网关应：校验 JSON Schema 子集、保留 part 顺序、避免重命名 `args`、不在响应中丢失 `name`。若工具执行失败，返回结构化 error 对象而非把异常字符串塞入 `response` 顶层；是否可恢复由模型决定。

内置工具差异很大：`googleSearch`/`googleSearchRetrieval`、`urlContext`、`codeExecution`、`computerUse`、`fileSearch` 在 Gemini 的 `tools`/`toolConfig` 中有专门结构；返回 grounding、url context、代码执行 part。Anthropic 对应的 web_search/code_execution 是服务器工具/beta，回传块类型不同。统一 IR 应分别保留“调用源”和“结果块原生形式”。

#### 2.8 结构化输出、多模态与音频

- `responseMimeType:"application/json"` 与 `responseSchema`：schema 为 JSON Schema 子集；Gemini 文档提到 `propertyOrdering` 等约定，但具体支持范围随模型变化。
- 多模态输入：`text`、`inlineData.{mimeType,data}`、`fileData.{fileUri,mimeType}`；视频可用 `videoMetadata`。
- 音频输出：`responseModalities` 包含 audio、`speechConfig.voiceConfig/multiSpeakerVoiceConfig`；不是所有模型/入口均支持。
- TTS/实时语音：Live API 属于独立交互模型；不要与 REST `generateContent` 的 `responseModalities:AUDIO` 混为同一 URL/帧协议。

#### 2.9 错误与限流

Google 使用 `google.rpc.Status` 风格：

```json
{"error":{"code":429,"message":"...","status":"RESOURCE_EXHAUSTED","details":[{"@type":"type.googleapis.com/google.rpc.RetryInfo","retryDelay":"30s"}]}}
```

| `status` | HTTP | 是否可退避 |
|---|---:|---|
| `INVALID_ARGUMENT` | 400 | 否 |
| `FAILED_PRECONDITION` | 400/409 | 条件修复后 |
| `PERMISSION_DENIED` | 403 | 否 |
| `NOT_FOUND` | 404 | 否 |
| `RESOURCE_EXHAUSTED` | 429 | 是，遵守 retryDelay |
| `DEADLINE_EXCEEDED` | 504/408 | 仅幂等请求 |
| `INTERNAL` | 500 | 谨慎 |
| `UNAVAILABLE` | 503 | 是 |
| `CANCELLED` | 499 | 客户端取消 |

**`429 RESOURCE_EXHAUSTED` 不等于 Anthropic 的瞬时 429。**Google 官方限流文档说明维度包括 RPM、TPM、RPD，并且实验/预览模型可能更严格；花费型限额按 10 分钟滚动窗口评估[21]。`details` 中的 `RetryInfo.retryDelay` 优先于猜测；也可读取 `Retry-After`。

**“200 但空 candidates”的安全拦截：**HTTP 成功不代表业务成功。网关判定顺序应为：

1. `candidates.length===0` → 读 `promptFeedback.blockReason`/safetyRatings；转 OpenAI error 或 `finish_reason:"content_filter"`。
2. `candidates.length>0` 但 content parts 为空 → 检查 candidate `finishReason`/safetyRatings。
3. 真实生成内容 → 正常投影。

**重试语义：**官方建议 429、408、5xx 指数退避+抖动并设置上限；400/403 不重试[20]。Gemini 没有本文已证实的通用“幂等 token”规范，长请求、工具副作用、Live/流式取消尤其要避免自动重放；`DEADLINE_EXCEEDED`/`CANCELLED` 前需判断是否已有模型输出。

## 第五章　国内主流厂商接口协议

> 国内厂商普遍提供 OpenAI 兼容端点，但"能用 OpenAI SDK 打通"不等于"协议等价"。差异集中在：思考字段命名、流式 usage 是否返回、`[DONE]` 是否发送、缓存统计字段、错误码体系、私有扩展参数。

### 研究范围、证据状态与使用约束

**检索截止日：2026-09-13。** 本报告优先采用各平台官方 API 文档；页面抓取成功的 DeepSeek、阿里云百炼、火山方舟、月之暗面 Kimi、MiniMax、讯飞星火、智谱 GLM-5、百度千帆、腾讯混元、阶跃星辰，以及 Ollama、LiteLLM 资料，均明确标注来源。vLLM 官方实时页在本次抓取环境受限，故只采用已抓取到的版本化官方镜像及明确 CLI/字段事实；部署时应以锁定版本的 `docs.vllm.ai` 再次生成 schema snapshot。**本报告不能替代逐模型、逐区域、逐版本的集成测试**：尤其 `response_format.json_schema`、`tools`、思考回传、缓存统计和流式末包，官方通用文档常只覆盖部分模型。

**兼容并非二进制，而是“可迁移、可解析、可计费”三个层次。** 多数国内厂商的 OpenAI 兼容，仅意味着 OpenAI SDK 能完成认证、发送 `messages`、接收 `choices[]`；不等于私有思考字段、缓存字段、限流语义、错误码、SSE 心跳和多模态结构都被 OpenAI SDK 自动处理。自研网关因此必须定义内部 canonical 模型，再将厂商原始事件映射出来，而不是相信 `openai.OpenAI(base_url=...)` 成功就代表协议等价。

**证据等级约定：** A＝本次已抓取厂商/项目官方文档；B＝官方文档定位页或索引，详细字段未完全加载；C＝官方兼容性表述但缺逐字段规范。C 级结论只用于接口形态，不用于断言参数默认值和全模型支持。

#### 文档版本与检索状态表

| 厂商/项目 | 检索日期 | 文档版本/页面状态 | 主要端点 | 证据等级 |
|---|---:|---|---|---|
| DeepSeek | 2026-09-13 | 官方 Chat API/错误码 | `https://api.deepseek.com`（兼容路径 `/v1`） | A |
| 阿里云百炼 Qwen | 2026-09-13 | 官方兼容说明；区域域名已迁移 | `https://{WorkspaceId}.cn-beijing.maas.aliyuncs.com/compatible-mode/v1` | A |
| 火山方舟 | 2026-09-13 | 官方对话/错误码/思考回传 | `https://ark.cn-beijing.volces.com/api/v3` | A |
| Moonshot Kimi | 2026-09-13 | 官方 API 概述 | `https://api.moonshot.cn/v1` | A |
| 智谱 GLM | 2026-09-13 | GLM-5 官方文档 | `https://open.bigmodel.cn/api/paas/v4` | A |
| 百度千帆 ERNIE | 2026-09-13 | 官方错误码；V2 兼容层未逐字段抓取 | `https://qianfan.baidubce.com/v2` | A（部分） |
| 腾讯混元 | 2026-09-13 | 官方 API 3.0 参考 | `hunyuan.tencentcloudapi.com`，Action `ChatCompletions` | A |
| MiniMax | 2026-09-13 | 官方 Chat API 索引（2026-06-09） | `https://api.minimax.io/v1` | A/B |
| 讯飞星火 | 2026-09-13 | 官方 HTTP 文档 | `https://spark-api-open.xf-yun.com/v1` | A |
| 阶跃星辰 StepFun | 2026-09-13 | 官方 Chat/streaming 文档 | `https://api.stepfun.com/v1` | A |
| Ollama | 2026-09-13 | 官方 API/OpenAI 文档 | `http://localhost:11434`、OpenAI `/v1` | A |
| vLLM | 2026-09-13 | 官方实时页抓取受限；版本镜像已核验 | `http://host:8000/v1` | B |
| LiteLLM | 2026-09-13 | 官方 Usage/Proxy 文档 | proxy 默认 `http://localhost:4000/v1` | A |

#### 1. DeepSeek：最贴近 OpenAI，但思考回传与缓存是强约束

**DeepSeek 的“兼容”足以直接复用 OpenAI 调用骨架，却不能把推理会话当成无状态映射。** 官方 Chat Completions 文档给出 `POST /chat/completions`、JSON 请求、Bearer 认证，以及标准 `choices`、`usage` 结构；真正的实现差异集中在 `thinking`、推理内容回传、自动前缀缓存和异常 HTTP 语义。[23] 建议将其作为 canonical 协议的参考实现，但将所有思考模型调用放入独立的会话状态机。

**接入层只有少量私有扩展，主要风险在运行时。** 固定请求头为 `Content-Type: application/json` 和 `Authorization: Bearer <TOKEN>`。官方模型示例包括 `deepseek-flash`、`deepseek-v4-pro`；具体可用模型应以控制台模型卡为准。兼容路径通常写作 `https://api.deepseek.com/v1/chat/completions`，`/beta` 只应在需要明确 beta 行为时配置，不应成为生产默认 base URL。

| 层 | 字段/行为 | 类型/取值（官方页面所见） | 与 OpenAI 关系 | 网关处理 |
|---|---|---|---|---|
| Header | `Authorization` | `Bearer <key>` | 一致 | 直通；禁止日志明文 |
| Header | `Content-Type` | `application/json` | 一致 | 强制 |
| Body | `model` | string，必填 | 一致 | 路由键；转换前校验 |
| Body | `messages` | object[]，必填 | 一致 | 按 provider normalization |
| Body | `thinking` | object，`reasoning_effort: none/low/high/max` | OpenAI 无此标准字段 | 放入 provider extra；不要删除 |
| Body | `max_tokens` | integer，1–384K | OpenAI 有 `max_completion_tokens` | 按模型映射；不强行双写 |
| Body | `temperature` | number，≤2，默认 1 | 一致 | 透传；思考模式实际可能忽略 |
| Body | `top_p` | number，≤1，默认 1 | 一致 | 透传 |
| Body | `response_format` | object，可空 | 部分一致 | 按模型快照测试 `json_schema` |
| Body | `tools`/`tool_choice` | 标准 OpenAI 结构 | 一致 | 保留完整 schema |
| Body | `stream_options.include_usage` | boolean | OpenAI 兼容扩展 | 默认注入，保障计费 |
| Response | `usage.prompt_tokens_details.prompt_cache_hit_tokens` | integer | 私有扩展 | 映射为 canonical `cache_read_tokens` |
| Response | `usage.prompt_tokens_details.prompt_cache_miss_tokens` | integer | 私有扩展 | 映射为 canonical `cache_creation_tokens` |
| Response | `usage.completion_tokens_details.reasoning_tokens` | integer | OpenAI Responses 风格扩展 | 可计费统计，不从主 token 重复扣除 |
| Response | `system_fingerprint` | string | OpenAI 风格 | 可选透传 |

**思考模式的关键不是字段名，而是“是否必须把历史 reasoning 拼回”。** DeepSeek 官方说明：`thinking` 可通过 `{"thinking":{"type":"enabled"}}` 启用，并使用 `reasoning_effort` 控制强度；thinking 默认开启，`high` 为默认强度。`minimal` 映射 `low`，`medium/xhigh` 映射 `high`。非思考参数 `temperature`、`top_p`、`presence_penalty`、`frequency_penalty` 在 thinking 模式下即使不报错也无效。[24] 推理内容位于 assistant message 中，与 `content` 同级的 `reasoning_content`；**是否回传取决于当轮请求是否带 `tools`**：带 `tools` 时应保留历史 reasoning，不带 `tools` 时不会拼回，回传也可能被忽略。这条规则必须在网关多轮 reducer 中实现，不能只保存 `assistant.content`。

```json
{
  "model": "deepseek-v4-pro",
  "messages": [
    {"role":"user","content":"比较 9.11 和 9.8"},
    {"role":"assistant","content":"","reasoning_content":"...完整思考...","tool_calls":[...]}
  ],
  "tools":[{"type":"function","function":{}}],
  "thinking":{"type":"enabled"},
  "reasoning_effort":"high",
  "stream_options":{"include_usage":true}
}
```

**流式事件可直接规范化为 OpenAI delta，但必须容忍注释与空字段。** DeepSeek 使用 `text/event-stream`；事件形如 `data: {JSON}`，最后发送 `data: [DONE]`。官方明确表示，设置 `include_usage` 后，除最后一个 chunk 外其余 chunk 的 `usage` 为 `null`，最终 chunk 才包含统计。[23] 实测还应忽略 `: keep-alive` 之类注释行，不能把非 `data:` 行当 JSON 解析。思考阶段 `delta.content` 可为空，`delta.reasoning_content` 连续增量；网关若只拼接 `content`，会把长思考误判为无输出。

```http
data: {"id":"...","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}
data: {"choices":[{"delta":{"reasoning_content":"先看小数"},"finish_reason":null}]}
data: {"choices":[{"delta":{"content":"9.8 更大"},"finish_reason":null}]}
data: {"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":...,"completion_tokens":...,"prompt_tokens_details":{"prompt_cache_hit_tokens":...}}}
data: [DONE]
```

**错误码是最强的“不要盲目重试”信号。** DeepSeek 官方列出 400 格式错误、401 认证失败、402 余额不足、422 参数错误、429 请求速率达到上限。[25] 因此 402 必须立即进入计费/人工处置，429 可采用退避+切换渠道；400/422 在请求未变化时不应无限重试。`finish_reason` 的私有取值仍应以当前模型响应为准，网关不要以白名单拒绝未知值，而应将其归一为 `stop`、`tool_calls` 或内部 `provider_specific`。

**上下文缓存是自动前缀缓存，不是应用层 session API。** DeepSeek usage 直接返回 `prompt_cache_hit_tokens` 与 `prompt_cache_miss_tokens`；应用无需手动创建 cache key。要提升命中率，应将稳定 system、tool schema 与共享前缀放在消息最前，并避免在前缀中插入可变 user 数据。文档未承诺固定前缀必然命中，故网关不能把缓存命中作为计费抵扣的唯一依据。

#### 2. 阿里云百炼（通义千问 Qwen）：兼容层较完整，但区域与请求头是首要故障点

**百炼适合作为“多供应商聚合入口”，但 API Key、区域与 workspace 必须绑定校验。** 官方当前推荐 workspace-specific 域名：北京为 `https://{WorkspaceId}.cn-beijing.maiyuncs.com/compatible-mode/v1`，OpenAI 兼容 Chat 路径为 `/chat/completions`；此外还有美国弗吉尼亚、新加坡、日本东京端点。[26] 旧全局域名在迁移说明中仍可见，但生产配置应优先当前推荐域名，否则后续可能因路由变化产生 404/401。

**Authorization 语义虽然像 OpenAI，区域错配却会伪装成 key 无效。** 请求头为 `Authorization: Bearer <DashScope API Key>` 与 `Content-Type: application/json`。官方明确：API Key 与区域绑定，跨区域调用会被 401，错误可能是 `invalid_api_key`；这不是 key 本身失效。[26] 因此网关应把 `(workspace/region, api_key)` 作为不可拆分凭证单元，不能将同一 key 跨 region 复用，也不应把该 401 当作“统一刷新 token”的条件。

| Header | 场景 | 官方说明 | 网关规则 |
|---|---|---|---|
| `Authorization: Bearer` | 所有调用 | 使用百炼 API Key | 必填；仅传入一次 |
| `X-DashScope-SSE: enable` | 流式调用 | 官方 TPM 预留文档列为流式必填 | 仅在 `stream=true` 注入 |
| `X-DashScope-Async: enable` | 异步批处理 | 枚举 `enable` | 不应由普通同步网关透传 |
| `X-DashScope-WorkSpace` | 子业务空间 | workspace 域名/隔离 | 由渠道配置决定，禁止用户任意覆盖 |
| `X-DashScope-OssResourceDownload` | 特定资源下载 | 私有控制 | 仅在明确能力白名单中传递 |
| `X-Request-Id`/平台 request id | 排障 | 错误响应提供 Request ID | 在日志上下文化，不承诺响应头名称 |

**请求 body 的基础面基本对齐 OpenAI，但 Qwen 私有能力不能靠通用 SDK 枚举保证。** 官方称请求参数与 OpenAI 接口对齐，支持 `stream`、`tools`、`response_format`；当前可见的 `response_format` 支持值为 `json_object` 与 `text`。[26] 这不自动证明每个 Qwen/Omni/VL 模型都支持 `json_schema`、图像细节、联网或工具并行调用。网关应建立模型能力矩阵；未知模型默认只承诺“请求可发送”，不承诺结构化输出成功。

**`enable_search`、Qwen3 `enable_thinking` 等扩展应以模型快照为单位。** 通用兼容文档未完整枚举全部私有字段，故不能把“百炼支持 Qwen”直接外推为所有模型都接受同一顶层开关。推荐做法是：canonical 请求使用标准 `tools`、`response_format`、`stream_options`；将 `enable_search`、`thinking`、`extra_body` 放入 `provider_params.qwen`，仅在模型支持标记开启时合并。这样若上游把能力移入标准 `tools` 或 `reasoning`，旧字段不会污染新模型。

**响应与流式的核心是 workbench-domain 补丁，而不是字段重写。** 非流式 `usage` 含 `prompt_tokens`、`completion_tokens`、`total_tokens`；网关应验证后映射到 canonical。[26] 官方抓取页未完整展示 `prompt_tokens_details.cached_tokens` 的全局结构，故不能断言所有模型均返回。流式必须按 `X-DashScope-SSE` 语义处理：发送 `data:`、可能的 keep-alive 注释、最终 `[DONE]`。实测矩阵至少覆盖：首个 chunk 的 `role`、中间 chunk 重复 `role`、无 content 的 tool delta、`usage` 是否出现在末包。百炼的兼容层建议客户端保留 `stream_options.include_usage=true`，若上游不返回，则由网关 fallback 统计。

**错误映射需同时处理 HTTP 与业务 code。** 官方兼容说明给出 400 无效请求、401 key 错误、429 限流/配额、500 服务错误、503 引擎过载。[26] 但这些只是 OpenAI 映射层错误，原生 DashScope 调用可能使用 `code`/`message`/`request_id` 结构；网关应以 `Accept` 的响应格式和实际 `Content-Type` 为依据做双解析器。429 需区分 RPM、TPM、并发和配额；403/401 不应重试；500/503 可有限重试。

#### 3. 火山方舟（豆包 Seed / DeepSeek/GLM 等接入点）：多协议并存，思考回传规则最复杂

**方舟不是单一模型协议，而是“同一推理接入点下的 Chat、Responses、Anthropic 三套表面”。** OpenAI 兼容 Chat 为 `POST https://ark.cn-beijing.volces.com/api/v3/chat/completions`，Authorization 为 `Bearer $ARK_API_KEY`；官方同时提供 Access Key 鉴权。[27] 但模型是否可用、是否为第三方直供、区域和 endpoint 配置均影响字段，不能仅根据 `doubao-*` 前缀假定行为。

**请求头私有扩展是幂等追踪与流量治理入口。** 官方 OpenAI SDK 示例明确可传 `X-Client-Request-Id`。[28] 推荐网关始终生成标准 UUID 并同时写入内部追踪头与 `X-Client-Request-Id`，但不得假定上游一定回显。调用示例说明也展示了图片 `image_url.url`、视频、音频等多模态输入，具体结构应按模型卡核验；网关不能把 OpenAI 文本 Chat 类型强制覆盖为单一 `string`。

| 维度 | Chat API 事实 | 工程含义 |
|---|---|---|
| Base URL | `.../api/v3/chat/completions` | 与 OpenAI `/v1` 仅路径前缀不同 |
| Auth | `Authorization: Bearer`；另有 AK 路径 | 渠道凭证须按认证方式分支 |
| Request ID | `X-Client-Request-Id` | 用于去重、日志关联；非重放保证 |
| 多模态 | `messages.content` 支持文本/图片/视频/音频对象 | canonical 至少保留 `type/text/image_url/file_id` |
| 思考字段 | `choices[].message.reasoning_content` | 流式 delta 映射同样字段 |
| 思考回传 | 取决于模型族和明文/加密 | **不能统一拼接 reasoning_content** |

**Seed 思考内容存在“明文原文”和“加密原文”两条规则，直接决定多轮质量。** 方舟官方 Agent 文档给出 OpenAI 兼容 Chat 使用 `choices[].message.reasoning_content`；对于豆包明文模型，需将该内容原文拼回下一轮请求。[29] 但 `doubao-seed-2-1-pro-260628`、`doubao-seed-2-1-turbo-260628` 等加密模型返回的并非可替代完整思考的明文：只回传 `reasoning_content` 摘要会导致推理质量下降。对这类模型，官方要求回传加密思考原文；若网关将其丢入普通文本 history 或仅保存 `content`，后续会话可能语义断裂。因此方舟渠道应保存 `thinking_blob`、版本、模型 ID 和回传策略，不能把 `reasoning_content` 简单映射为可显示文本。

**流式 SSE 应按标准 OpenAI chunk 处理，但多 chunk 重复 `role` 与注释行必须容忍。** 官方流式示例使用 `stream:true`，Python SDK 直接遍历 `chunk.choices[0].delta.content`。[27] 这足以证明兼容消费方式成立，但未在抓取页完整承诺所有模型的 `data: [DONE]`、首个 role、usage 末包；故方舟应纳入高优先级契约测试。参考 DeepSeek/GLM 行为，增量期间 `delta.content=""` 且 `reasoning_content` 递增是合理预期。

```http
data: {"id":"...","choices":[{"delta":{"role":"assistant","content":""},"finish_reason":null}]}
data: {"choices":[{"delta":{"reasoning_content":"逐步判断"},"finish_reason":null}]}
data: {"choices":[{"delta":{"content":"结论"},"finish_reason":null}]}
data: {"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{}}
data: [DONE]
```

**限流与欠费必须分开，因为二者重试策略相反。** 方舟官方错误表显示：429 `RateLimitExceeded.EndpointRPMExceeded` 表示 RPM 超限，可稍后重试；400 `AccountOverdueError` 表示欠费，需充值；403 `OperationDenied.ServiceNotOpen`、404 `ModelNotOpen` 表示服务或模型未开通；400 `SensitiveContentDetected` 要求更换 prompt。[30] 所有错误均附 Request ID，排障 API 应将原始 request id、endpoint、model、region 一并保留。

| HTTP | Type | Code | 可重试 | 网关动作 |
|---:|---|---|---:|---|
| 400 | BadRequest | `InvalidParameter` | 否 | 校验 schema，记录参数路径 |
| 400 | BadRequest | `SensitiveContentDetected` | 否 | 返回内容策略错误，不重试 |
| 400 | BadRequest | `AccountOverdueError` | 否 | 切换至其他渠道/告警，不指数退避 |
| 403 | Forbidden | `OperationDenied.ServiceNotOpen` | 否 | 开通服务/模型白名单 |
| 404 | NotFound | `ModelNotOpen` | 否 | 校验模型路由 |
| 429 | TooManyRequests | `RateLimitExceeded.EndpointRPMExceeded` | 是 | 限流退避、切换 endpoint |
| 5xx | - | 服务错误 | 条件 | 最多有限重试，隔离不健康 endpoint |

#### 4. Moonshot Kimi：OpenAI/Responses/Anthropic 三轨，私有扩展必须留在 extra_body

**Kimi 的 OpenAI 兼容路径足以承载标准 Chat，但 `thinking` 与 `partial` 证明它并非严格子集。** 官方概述列出 `https://api.moonshot.cn/v1/chat/completions`、`/v1/responses`、`https://api.moonshot.cn/anthropic/messages` 三条路径。[31] 生产网关若目标是 OpenAI canonical，应使用 `/v1`；不要用 Responses 或 Anthropic 路径混入同一模型路由，除非明确将其转为独立能力。

**认证简单，专有字段却有明确位置规则。** `Authorization: Bearer $MOONSHOT_API_KEY` 适用于所有 API。官方明确：`thinking` 通过 SDK `extra_body` 传递；`partial` 是 assistant message 上的字段，不是顶层请求参数。[31] 因此 canonical provider request 可定义 `reasoning: {...}`，映射至 Kimi 时合并为 `extra_body.thinking`；若直接将 `thinking` 放在顶层，未来厂商若改用标准字段会造成冲突。

```json
{
  "model":"kimi-k2-0905-preview",
  "messages":[
    {"role":"assistant","partial":true,"content":"之前被截断的回复..."},
    {"role":"user","content":"继续"}
  ],
  "stream":true,
  "extra_body":{"thinking":{"type":"enabled"}}
}
```

**当前官方概述不能证明旧版 Kimi 的所有历史限制现已消失。** 历史版本确有 function calling、联网搜索、上下文缓存方面差异；本次抓取到的当前官方页确认了 `/v1/chat/completions`、Bearer、思考扩展和错误结构，但**未完整确认 `enable_search` 当前是否仍是全局参数、具体模型 `json_schema` 支持或 `stream_options` 全行为**。因此本报告不把这些项写作 Kimi 的确定全局能力，而建议按模型快照实测。任何“Kimi 长期不支持 tool”的旧结论都不应作为 2026 年网关事实。

**错误对象只有部分标准结构被明确承诺。** 官方说明请求失败返回包含 `error.type` 与 `error.message` 的 JSON。[31] 本次抓取未获得完整 Kimi 数字错误码表和限流响应头，因此不能臆造 429 子 code、余额 code 或 `x-ratelimit-remaining`。网关应解析至少 `{error:{type,message,code,param}}`，未知字段原样保存；429/502/503 可重试，401/403/400 不可重试。

**`partial` 是对流式恢复/续写协议的提醒：网关不能只保存已完成 assistant。** 当历史中包含 `partial:true`，说明该 assistant 消息在前一轮可能被截断或尚未完整；如果直接丢弃，续写会丢失上下文。建议 canonical message 增加 `state: complete|partial`，并在 OpenAI 输出时仅将 `partial` 放在内部扩展，避免未授权客户端误用。

#### 5. 智谱 GLM：Chat v4 已是标准兼容形态，但思考与工具多轮必须执行 GLM-5 规则

**GLM 的兼容成本主要在思考开关和推理回传，不在基础消息结构。** 官方 GLM-5 文档给出 `POST https://open.bigmodel.cn/api/paas/v4/chat/completions`、`Content-Type: application/json`、`Authorization: Bearer YOUR_API_KEY`。[32] 这比历史 PaaS 版本更贴近标准 OpenAI Chat，但 `thinking`、`reasoning_content` 仍属于能力扩展。

**`thinking` 是当前明确的顶层扩展，不是可有可无的 metadata。** GLM-5 官方示例直接发送：

```json
{
  "model":"glm-5",
  "messages":[
    {"role":"user","content":"..."}
  ],
  "thinking":{"type":"enabled"},
  "max_tokens":65536,
  "temperature":1.0
}
```

这意味着 canonical 不应把 `thinking.type` 直接删除，而应转换为内部 `reasoning.mode=enabled|disabled|auto`，在 GLM 渠道序列化为 `thinking`。`max_tokens=65536` 只代表该示例，不能当作所有 GLM 模型的硬上限；网关应从模型卡片读取上限，并对请求执行最小值裁剪。

**流式推理内容使用 `reasoning_content`，可复用 DeepSeek 风格归一器。** 官方流式代码明确读取 `chunk.choices[0].delta.reasoning_content` 和 `chunk.choices[0].delta.content`。[32] 因而 DeepSeek、GLM、部分方舟模型可共享一个 `reasoning_accumulator`；但方舟加密思考模型不能用同一“明文回传”策略。工具调用仍应以标准 `delta.tool_calls[]` 的 `id`、`type`、`function.name`、`function.arguments` JSON 增量合并，不能以 `reasoning_content` 承载调用参数。

**GLM 多轮的核心风险是“删除了思考内容后再次请求”。** 官方 GLM-5 页展示了带 assistant 历史消息的调用示例，但没有在本次抓取的同一页中给出所有模型“是否必须回传 reasoning_content”的全量规则。鉴于 DeepSeek 同源协议已经证明此类规则会按 tools 场景变化，建议 GLM 渠道默认保存完整原始 assistant（包括 `reasoning_content`、`tool_calls`），按模型版本执行回传策略，而不是为节省存储只保留 `content`。

**响应统计与限流信息以实测为准。** GLM-5 官方页确认了请求/流式调用样例，但未在本次抓取内容中完整展示 `usage.prompt_tokens_details.cached_tokens`、`completion_tokens_details.reasoning_tokens`、私有错误码表或限流头。因此本报告**不编造 GLM 错误数字码、重试规则或缓存字段**。契约测试应覆盖：非流式 `usage`、流式 `stream_options.include_usage`、tool 后的 `finish_reason`、思考结束后的首个 content、连续相同 `role`。

#### 6. 百度千帆 ERNIE：存在原生 API 与 V2 OpenAI 兼容层，不能混用鉴权

**千帆对网关的最大风险不是字段缺失，而是历史原生 API 与 V2 兼容层并存。** 用户指定兼容路径为 `https://qianfan.baidubce.com/v2`；官方错误码页仍是文心工作坊原生接口风格，返回 `{error_code, error_msg}` 而非完整 OpenAI `error` 对象。[33] 若渠道配置为 `/v2`，应优先验证其响应是否为 OpenAI 结构；若命中原生 endpoint，则需走千帆原生映射。

**原生鉴权与 OpenAI Bearer 不兼容，V2 兼容层须单独验证。** 千帆 SDK 文档说明传统调用可使用 Access Key/Secret Key，并通过应用 AK/SK 获取凭证；原生推理 API 还可能涉及 access_token。[34] 因此不要假设 `Authorization: Bearer sk-xxx` 能直接用于所有 ERNIE endpoint。对于 `/v2`，建议首先用最小 `chat/completions` 探测：认证失败形态、错误 body、model 列表和 `usage`，再决定是否加入 OpenAI canonical。

| 接口族 | 推断/已确认接入 | 兼容策略 | 不可假定项 |
|---|---|---|---|
| ERNIE 原生 | AK/SK、可能 access_token；`error_code/error_msg` | 原生适配层 | OpenAI 字段全覆盖 |
| 千帆 V2 | 用户指定 `qianfan.baidubce.com/v2` | 优先 OpenAI canonical | 实际鉴权头、错误结构、模型表 |
| 模型 SDK | 旧版 `ChatCompletion` | 内部桥接，不建议直接暴露 | 与 V2 的请求 schema 相同 |

**错误码映射应优先保留原始数字 code。** 官方列表明确：1 未知错误、2 服务暂不可用、3 方法不支持、4 集群超限额、6 无权限、13 获取 service token 失败、14 IAM 失败、15 应用不存在、17 日请求超限/欠费或免费额度耗尽。[33] 可重试集建议为 `{1,2,4}`；不可重试为 `{3,6,13,14,15,17}`。由于该页属于原生 API，V2 兼容层若返回 OpenAI 结构，不应强行覆盖为千帆数字码。

**结构化输出、工具、联网与思考均按模型快照处理。** 本次已抓取官方资料未完整展示千帆 V2 的 `json_schema`、`enable_search`、thinking、SSE 心跳和 `cached_tokens`，故不将其写成全局支持。建议渠道注册时保存能力位：`tool_calls`、`parallel_tool_calls`、`response_format.{json_object,json_schema}`、`streaming`、`include_usage`、`thinking`，并在模型更新时回归。

#### 7. 腾讯混元：原生 API 3.0 并非 OpenAI Chat，网关必须显式适配

**混元是“OpenAI 兼容层可包装，但官方原生接口并非 OpenAI Chat”的典型案例。** 腾讯云官方 API 3.0 文档使用 TC3-HMAC-SHA256 签名、Action/Version/Region 公共参数和 `ChatCompletions` Action，而非简单的 `/v1/chat/completions`。[35] 因此若接入点是 `hunyuan.tencentcloudapi.com`，网关不能把它当作 Ollama/DeepSeek 同类渠道。腾讯有独立 OpenAI 兼容产品的可能性需以控制台当前文档为准，不能根据产品名假定路径。

**消息数组约束比 OpenAI 更严，直接复制 messages 可能失败。** 官方要求 `Messages.N` 中 system 可选且必须在最前；user、assistant、`tool`（function call 场景）需按规则排列，tool 可连续出现；`Content` 不能为空，默认单账号并发 5 路。[35] OpenAI 通常允许更自由的消息序列，因此网关在混元 native 渠道应执行预校验：去除空 content、合并相邻同角色、将 tool result 置于合法位置、限制上下文长度。

**鉴权与重试规则是腾讯云标准而非 LLM 业务规则。** 官方页本次抓取未完整展示混元业务错误码，但已知其属于 API 3.0。建议采用通用腾讯云错误结构 `{Response:{Error:{Code,Message,RequestId}}}` 映射；401/403 类认证、参数错误不重试，5xx/限流可有限重试，并保存 `RequestId`。并发 5 路是默认限制而非可忽略提示，网关需按账户购买额配置并发槽。

**混元能力字段存在版本差。** 当前官方抓取内容未完整证明 Chat Completions Action 已全面支持 OpenAI 风格 `tool_choice`、`response_format.json_schema`、`reasoning_content`、SSE `[DONE]` 与 `usage`；故不臆造。推荐把“混元 native”和“混元 OpenAI 兼容”作为两个 provider driver：前者做签名、消息规范化、错误映射；后者复用 OpenAI 归一器但保留模型能力白itelist。

#### 8. MiniMax：标准 Chat 外扩出 service tier 与审计字段，流式细节仍需契约测试

**MiniMax 的 OpenAI 兼容程度较高，但响应携带明显的平台私有字段。** 官方 Chat API 索引发布时间为 2026-06-09，示例端点为 `POST /v1/`，curl 使用 `Authorization: Bearer`、`Content-Type: application/json`。[36] 官方兼容说明与抓取到的文本聊天接口概述一致，请求使用标准 `model`、`messages`，并展示图像和思考能力。

**`thinking` 与 `max_completion_tokens` 已出现，但属于具体模型能力而非可无条件转换。** 官方示例：

```json
{
  "model":"MiniMax-M3",
  "messages":[
    {"role":"user","content":[
      {"type":"text","text":"What does this image show?"},
      {"type":"image_url","image_url":{"url":"https://..."}}
    ]}
  ],
  "thinking":{"type":"adaptive"},
  "max_completion_tokens":500
}
```

这说明至少 MiniMax-M3 接受标准 OpenAI 的 `max_completion_tokens` 及私有 `thinking.type`。网关可将 canonical `max_output_tokens` 优先映射为 `max_completion_tokens`；仅当目标模型不支持时回退到 `max_tokens`，不应双写。

**响应中的 `usage` 与审计字段必须分层。** 官方示例响应包含标准 `usage`，同时有 `prompt_tokens_details.cached_tokens`；此外还有 `input_sensitive`、`output_sensitive`、`base_resp.status_code/status_msg`。[36] 后者属于服务状态包装，不应覆盖 OpenAI 的 HTTP 错误语义：仅在 `base_resp.status_code!=0` 且 HTTP 成功时，才将其转成网关内部 provider warning 或错误。`cached_tokens` 应映射至 canonical cache read，但不要把 MiniMax 的审计敏感标记暴露给普通租户。

**限流与错误细节在本次抓取中不完整。** 索引页未展示完整错误码表或 `x-ratelimit-*`；因此只可确认 API 结构与 service tier，不能声称具体 RPM、TPM、错误数字码。建议默认：`service_tier=priority` 可提高准入优先级，代价约 1.5 倍价格（官方索引说明），因此网关应根据预算策略显式设置，不要从普通租户请求中透传。

#### 9. 讯飞星火：OpenAI 兼容层字段较规范，限流码必须保留原始值

**星火的 OpenAI 兼容层适合直接接入，但模型版本仍决定上下文边界。** 官方 HTTP 文档明确：`base_url=https://spark-api-open.xf-yun.com/v1/`、接口为 `/v1/chat/completions`、`Authorization: Bearer <APIPassword>`、`Content-Type: application/json`。[37] 因此认证与基础路径可与 DeepSeek/百炼同类渠道共享同一 OpenAI driver。

**请求字段明确支持流式、JSON 与工具，足以覆盖 Agent 基础场景。** 官方示例包含 `stream:true`、`response_format:{type:json_object}`、`tools:[{type:function,function:{...}}]`。[37] 不过这些证据不足以断言所有模型均支持 `json_schema`、并行工具或思考模式；把“支持 `json_object`”外推为“支持完整 structured output”是网关常见错误。

**usage 简单，错误码却是私有数字体系。** 官方展示响应 `usage:{prompt_tokens,completion_tokens,total_tokens}`。[37] 错误码包括：10007 用户流量受限，需等待；11200 授权/业务量超过限制；11201 日流控超限；11202 秒级流控超限；11203 并发流控超限。[37] 10007、11201–11203 可重试但应限流退避；11200 需检查授权和套餐。由于数字 code 与 HTTP 状态并非一一对应，映射表必须保存 `provider_error_code`。

| 版本/场景 | 官方上下文信息 | 网关约束 |
|---|---|---|
| Ultra | 输入 32K、输出 32K | 记录模型快照 |
| Max | 输入 8K/32K、输出 8K/8K | 旧套餐下线，需动态模型表 |
| Pro | 输入 8K/32K、输出 8K/4K | 按具体版本读取 |
| Lite | 输入 8K、输出 4K | 免费/轻量限制可能变化 |

**流式契约必须实测的原因：** 官方文档证明 `/v1/chat/completions` 支持 `stream`，但本次未完整展示 SSE 原始帧、首个 role、`[DONE]`、keep-alive、tool delta 与 `include_usage` 的全局保证。讯飞历史接口曾存在私有字段，故应把星火列为契约测试重点，不宣称“完全 OpenAI SSE”。

#### 10. 阶跃星辰 StepFun：接近标准 Chat，私有推理强度是明确扩展

**StepFun 可归入“OpenAI 兼容优先”的轻适配渠道。** 官方 Chat API 文档给出 `POST https://api.stepfun.com/v1/chat/completions`、`Authorization: Bearer`、`Content-Type: application/json`，OpenAI Python/JS SDK 初始化示例直接使用 `/v1`。[38] 这足以复用标准 OpenAI 请求驱动。

**`reasoning_effort` 是当前确定的私有扩展，映射方向比字段名更重要。** 官方示例：

```json
{
  "model":"step-3.7-flash",
  "messages":[{"role":"user","content":"请用三句话解释什么是强化学习。"}],
  "reasoning_effort":"medium",
  "max_tokens":1024
}
```

canonical 可定义 `reasoning.effort=low|medium|high`，StepFun 映射为 `reasoning_effort`；若目标厂商使用 `reasoning_effort` 整数/枚举不同，需另做转换。文档未承诺 `low/medium/high` 覆盖全部模型，故应允许渠道配置默认值及是否透传。

**流式、工具、多模态能力应以具体模型为准。** 官方页面明确包含“流式响应”和“推理强度”主题，并有图片理解、Base64 图片能力；但本次抓取内容没有完整展开 tool call、reasoning content delta、SSE 结束帧、错误码或限流头。因此不将 StepFun 的 reasoning content 字段名、tool 并行行为和缓存字段写成确定全局事实；契约测试应覆盖 `step-*` 模型族并回填能力矩阵。

## 第六章　开源推理与网关生态

> Ollama 原生是 NDJSON 而非 SSE；vLLM 以 OpenAI 兼容为核心并扩展引导式解码；LiteLLM 本身就是"协议转换层"的成熟参考实现。

#### 11. Ollama：原生 REST/NDJSON 与 OpenAI 兼容层并存，切勿把 /api/chat 当 SSE

**Ollama 的原生 API 不是 OpenAI Chat，也不是 SSE；网关应把 NDJSON 作为第一等协议。** 默认服务地址为 `http://localhost:11434`。`POST /api/chat` 接收 `model`、`messages`，可选 `tools`、`format`、`options`、`stream`、`think`、`keep_alive`；`stream` 默认 `true`。当 `stream=false` 返回单个 JSON；为 `true` 时按换行分隔 JSON（NDJSON），最终对象 `done=true`。这与 `data: ...` 和 `[DONE]` 的 SSE 完全不同。[39]

**`/api/chat` 的 message 模型比 OpenAI 更“本地推理友好”。** 官方响应包含 `model`、`created_at`、`message.{role,content,thinking,tool_calls,images}`、`done`、`done_reason`，以及 `total_duration`、`load_duration`、`prompt_eval_count`、`prompt_eval_duration`、`eval_count`、`eval_duration`。[39] 其中 `thinking` 对应推理模型输出；`prompt_eval_count/eval_count` 可用于本地计费和性能诊断；`done_reason` 可表示停止原因，但不应冒充 OpenAI `finish_reason`。

**options 是 Ollama 私有采样命名空间，需要显式映射。** 官方 `/api/chat` 将 `options` 定义为运行时生成选项；仓库 API 文档确认 `think` 可为布尔值或 `low/medium/high/max`，并支持 `tools`。[40] 常见 options 包括 `num_predict`、`temperature`、`top_p`、`top_k`、`num_ctx`、`repeat_penalty`、`seed`、`stop`、`mirostat`、`num_gpu` 等，但实际可用性取决于版本和模型。建议 canonical 只允许标准化采样参数进入 `options` 白名单，未知字段落到 `provider_params.ollama`。

| Canonical | Ollama `/api/chat` | 说明 |
|---|---|---|
| `max_output_tokens` | `options.num_predict` | 0/-1 常表示无限制，需按版本测试 |
| `temperature` | `options.temperature` | 通常为 0–1，模型/版本有差异 |
| `top_p` | `options.top_p` | 透传 |
| `top_k` | `options.top_k` | OpenAI 无标准对应 |
| `repetition_penalty` | `options.repeat_penalty` | 注意参数名不同 |
| `stop` | `options.stop` | 可为字符串或数组 |
| `seed` | `options.seed` | 可复现输出 |
| `reasoning.mode` | `think` | bool 或 effort 字符串 |
| `context_length` | `num_ctx` | 受模型与显存限制 |

**流式 NDJSON 的结束条件是 `done:true`，不是解析 `[DONE]`。**

```json
{"model":"qwen3","created_at":"...Z","message":{"role":"assistant","tool_calls":[{"function":{"name":"get_current_weather","arguments":{"location":"Toronto","format":"celsius"}}}]},"done":false}
{"model":"qwen3","created_at":"...Z","message":{"role":"assistant","content":"今天多云。"},"done":false}
{"model":"qwen3","created_at":"...Z","message":{"role":"assistant","content":""},"done":true,"total_duration":...,"eval_count":...}
```

Ollama 官方 tool streaming 示例显示 content 与 tool_calls 可跨 chunk 累积，`arguments` 通常以对象增量合并。[41] 网关 NDJSON 解析器应按 `\n` 分割、忽略空行、按 `done` 收尾；不要把 `application/x-ndjson` 当作 SSE。

**OpenAI 兼容层是“部分兼容”，不能替代原生 API 的全部控制力。** 官方明确 Ollama 仅兼容 OpenAI API 的“部分”，示例为 `http://localhost:11434/v1/`、`api_key='ollama'`；支持 Chat、Streaming、JSON mode、可复现输出、Vision、Tools、Reasoning/thinking control、Logprobs。[42] 同时官方明确指出：`/v1/completions` 的 `prompt` 当前仅接受字符串；`/v1/responses` 仅支持无状态 flavor，没有 `previous_response_id`/conversation 支持。[42] 因此若客户端需要 `max_tokens`、`stop`、tools 等，先验证具体 Ollama 版本；否则退回原生 `/api/chat` 桥接。

**错误是 HTTP 状态与 `{error}` 结合，而非复杂业务 code。** 官方 API 文档示例返回 200/404 与 JSON 说明；实际服务器错误通常返回非 2xx 及 `{"error":"..."}`。网关应在 native 驱动中把 Ollama 错误标准化为 OpenAI 风格 `{error:{message,type,code,param}}`，并保留原始 message；进程崩溃、模型未拉取、显存不足多表现为 500/超时，应触发实例隔离而非参数重试。

#### 12. vLLM OpenAI-Compatible Server：高自由度后端，兼容性取决于启动参数与模型模板

**vLLM 的目标是 OpenAI 形态兼容，但其真实协议面由部署版本、模型 parser 和额外参数共同决定。** 本次官方实时页抓取受限，已抓取版本化文档确认端点包含 `/v1/chat/completions`、`/v1/completions`、`/v1/chat/completions/batch`、`/v1/responses`、`/v1/embeddings`、`/v1/audio/transcriptions|translations`；Chat 中 `user` 参数被忽略，且支持并行工具调用控制。[43] 对网关而言，vLLM 不应被视为“一个固定模型”：不同 commit 的字段支持、错误响应、tool parser 都可能变化。

**标准参数可透传，extra 参数则必须受模型白名单控制。** 官方说明 vLLM 支持不属于 OpenAI 的参数，例如 `top_k`，可通过 `extra_body={"top_k":50}` 或 JSON 直接合并。[43] Chat extra 常见包括 `best_of`、`use_beam_search`、`top_k`、`min_p`、`repetition_penalty`、`length_penalty`、`ignore_eos`、`min_tokens`、`stop_token_ids`、`echo`、`add_generation_prompt`、`include_stop_str_in_output`、`guided_json/regex/choice/grammar`、`guided_decoding_backend`。**这些字段不应默认转发**：它们可能显著改变模型行为或暴露部署能力。

| 能力 | 启动/请求机制 | 网关策略 |
|---|---|---|
| Tool calling | `--enable-auto-tool-choice`、`--tool-call-parser` | 按模型版本选择 parser，不能全局开启 |
| Tool parser | hermes、mistral、llama3_json、granite、internlm、pythonic、xlam 等 | canonical 不直接暴露 parser 名称 |
| Reasoning | `--reasoning-parser` | deepseek_r1/qwen3/glm45 之类映射取决于版本 |
| Structured output | `guided_json/regex/choice/grammar`、`response_format` | 优先 OpenAI `response_format`，回退 extra |
| Sampling | `top_k`、`repetition_penalty`、`min_tokens` 等 | 仅白名单模型允许 extra_body |
| LoRA | `--lora-modules`、模型前缀/路由 | 转换为部署侧路由，避免用户任意选择 |
| 请求 ID | `--enable-request-id-headers`、`X-Request-Id` | 日志关联；不假定所有版本启用 |
| 优先级 | `X-Vllm-Priority` | 内部调度，不暴露租户原始值 |

**结构化输出是 vLLM 的强项，但字段名存在版本漂移。** 已抓取官方版本文档展示 `guided_json`、`guided_regex`、`guided_choice`、`guided_grammar`、`guided_decoding_backend`，并允许通过 `response_format` 或 extra body 使用。[43] 新版可能把部分能力收敛至 OpenAI `response_format` 的 `json_schema`、新增 `guided_decoding_backend`、改变 `reasoning_content` 输出。生产部署应锁定镜像 tag，从 `/v1/models` 和启动日志读取能力，并运行 contract test，而不是把 2026 年文档硬编码为所有版本事实。

**流式使用统计依赖 flag 与版本。** 与 DeepSeek 类似，网关应在 vLLM 请求中默认设置 `stream_options.include_usage=true`；若部署版本不支持，则捕获、记录并在末段用本地统计构造兼容 `usage`。finish 原因通常包括 `stop`、`length`、`abort` 等；`abort` 不是 OpenAI 常见标准值，网关应映射为 canonical `cancelled`，保留原始值。`[DONE]` 是否发送、usage 是否在最后 chunk、空 content 的 tool call 增量，均需每版本验证。

**错误响应不是强契约。** vLLM 在参数错误、模型不存在、推理异常时通常返回 4xx/5xx 与 JSON，但本次证据不足以承诺所有版本均为 `{error:{message,type,code}}`。建议先解析标准 OpenAI 错误，再解析 FastAPI/pydantic 的 `detail`，最后兜底为 `error.message=raw_body`。

#### 13. LiteLLM：适合作为转换/路由参考，而非未经验证的协议真相源

**LiteLLM 的价值是把 100+ provider 的错误、usage、模型路由收敛到 canonical，而不是消除上游私有字段。** 官方说明所有 provider 返回 OpenAI 兼容的 usage：`prompt_tokens/completion_tokens/total_tokens`；流式 `include_usage=true` 时，usage chunk 在 `data:[DONE]` 之前发出，其余 chunk 的 `usage` 为 `null`。[44] 自研网关可复用该设计：内部统一“统计事件在结束前、usage 可空、最后 `[DONE]`”的规范。

**`always_include_stream_usage` 是应对厂商缺失 usage 的有效模式。** LiteLLM Proxy 配置：

```yaml
general_settings:
  always_include_stream_usage: true
```

启用后即使客户端未发送 `stream_options.include_usage`，代理也会注入并在末段返回 usage。[44] 该能力不保证上游真实返回 token；如果上游仍缺失，代理可用请求/响应 token 近似统计，但必须明确标记为 `estimated=true`，绝不能与真实计费缓存 token 混用。

**模型前缀是路由语法，不是协议字段。** LiteLLM 使用 `provider/model` 风格标识，例如 `openai/`、`anthropic/`、`ollama/`、`deepseek/`、`volcengine/`；官方还说明可基于模型 mode 自动 bridge `/chat/completions` 与 `/responses`。[44] 自研网关可借鉴为内部 `channel_id:model_id`，但对外不应强制暴露 provider 前缀；前缀会改变 SDK model 名称，影响客户端追踪与审计。

**异常映射的目标是客户端错误类型兼容。** LiteLLM 官方称会将跨 provider 异常映射为 OpenAI 异常子类，如 `AuthenticationError`、`RateLimitError`、`APIError`。[45] 因此推荐网关输出统一 `error.type∈authentication/permission/rate_limit/bad_request/not_found/server`，原始 provider code/request id 放入 `error.extensions`。这不意味着上游 HTTP 码总被改写，网关应区分“对外 HTTP 码”和“业务错误类型”。

**成本与 fallback 属于控制面，不应污染模型响应。** LiteLLM 有 `response_cost`、callback、proxy 限流、fallback/router 概念。[44][23] 自研设计中，成本、延迟、渠道健康度应放在响应扩展头或独立 telemetry；不要在 `choices`/`usage` 中塞入非标准计费对象，避免破坏 OpenAI 客户端。

#### 14. 其他开源/自建方案：差异集中在原生协议、额外参数和聚合透传

**Xinference、FastChat、LocalAI、SGLang、llama.cpp 可共享 OpenAI driver，但能力发现必须逐个注册。** 这些项目的共同点是暴露 `/v1/chat/completions`、`/v1/models`、`/v1/embeddings` 的子集；差异在于 tool parser、guided decoding、多模态、reasoning、音频和部署参数。网关不应仅根据“OpenAI-compatible”标签启用全部功能，而应运行 capability probe：发送最小 chat、stream、tool、json_schema、usage、取消请求，再记录支持矩阵。

**SGLang 的 OpenAI 兼容层重点在 tool/structured/reasoning 启动配置。** 与 vLLM 类似，工具调用和思考内容解析依赖模型模板与 parser；某些版本通过 `--tool-call-parser`、`--reasoning-parser` 或 response format 插件控制。推荐将 SGLang 归入“自部署推理后端”驱动，使用 vLLM 式 extra_body 白名单，不把 CLI 参数直接暴露给租户。

**llama.cpp server 存在 OpenAI 兼容 Chat 与自有 completion 的差异。** 其 `/v1/chat/completions` 可接入 OpenAI 客户端，而原生 `/completion` 使用 `prompt` 字符串、slot、maintain/save/restore 等本地推理概念。Ollama 官方兼容文档也侧面确认 `/v1/completions` 的 `prompt` 仅支持字符串。[42] 因而网关若同时暴露 completions 与 chat，必须明确命名空间，不能把 `/v1/completions` 当成 Chat 降级版。

**One-API/New-API 是渠道聚合层，而不是协议标准化保证。** 这类网关通常对外暴露 `/v1/chat/completions`，内部将 `model` 映射到不同 provider，支持 API Key 聚合、余额、限流和渠道 fallback；`extra_body`/provider-specific params 则用于透传非标准字段。自研网关应借鉴的只有三点：渠道配置不可泄露私有 header；透传字段必须按 provider/模型版本隔离；上游错误、request id、usage 必须原样保存。不要假设聚合层会修正 SSE、usage、思考回传或错误码。

## 第七章　跨协议字段映射总表

> 本章是"OpenAI ↔ Claude ↔ Gemini"三方互转的映射规范，也是第七章至第九章的工程基础。核心结论：**保留 provider 原生对象，只向 OpenAI 客户端投影子集**。

### OpenAI 兼容网关转换规范

#### 3.1 核心策略：保留原生对象，投影 OpenAI 表面

网关内部至少应有：

```text
Client OpenAI request
  -> normalized IR (model, system, messages, tools, sampling, stream)
  -> provider native request
  -> provider native response/events
  -> normalized IR Response
  -> OpenAI ChatCompletion / chunks
```

任何“直接把 OpenAI JSON 改名成 Gemini JSON”的方案都会在处理 thinking、tool result、流式生命周期、空 candidates 时失真。

#### 3.2 角色与系统消息映射

| OpenAI | Claude | Gemini |
|---|---|---|
| `system` message | **顶层 `system`** | **顶层 `systemInstruction`** |
| `user` | `user` | `user` |
| `assistant` | `assistant` | `model` |
| `tool` | user message 内的 `tool_result` | user content 内的 `functionResponse` part |

**转换规则：**

1. 将 OpenAI `messages` 拆分为 `system[]` 与对话 `messages/contents`。
2. Claude：所有 system 合并成 string 或 system block；不得保留 `role:system`。
3. Gemini：放入 `systemInstruction.parts[]`；其 `role` 可省略或忽略。
4. OpenAI 连续 tool messages：Claude 应聚合为一条 user message 的多个 `tool_result`；Gemini 聚合为一条 user content 的多个 `functionResponse`。

#### 3.3 工具调用三方映射

| 概念 | OpenAI | Claude | Gemini |
|---|---|---|---|
| 工具定义 | `functions[]/tools[].function` | `tools[]` | `tools[].function_declarations[]` |
| 参数 schema | `parameters` | `input_schema` | `parameters` |
| 调用容器 | `choice.message.tool_calls[]` | `content[]` 中 `tool_use` block | `parts[]` 中 `functionCall` |
| 调用 ID | `id` | `id` | `name`/`id`（版本相关） |
| 函数名 | `function.name` | `name` | `functionCall.name` |
| 参数 | `arguments`（**JSON 字符串**） | `input`（**对象**） | `args`（**对象**） |
| 结果 role | `tool` | `user` + `tool_result` | `user` + `functionResponse` |
| 结果内容 | `content`（字符串） | `content`（string/blocks） | `response`（对象） |
| tool choice | `auto/none/required` + function | `auto/any/tool/none` | `AUTO/ANY/NONE` + allowed names |

**JSON 边界：**

- OpenAI→Claude/Gemini：解析 `arguments` 字符串为对象；失败时用 provider 允许的结构化错误包装，不要发送裸字符串。
- Claude/Gemini→OpenAI：序列化 `input`/`args` 为 JSON 字符串；保证 stable key order 与空对象 `{}`。
- Gemini `functionResponse.response` 序列化后放到 OpenAI `tool` message；反向需解析。
- Claude 流式中，`input_json_delta.partial_json` 拼接后才可解析；不要逐 chunk JSON.parse。

#### 3.4 流式转换：这是损失最大的区域

| 协议 | 事件模型 | 必须维护的状态 |
|---|---|---|
| OpenAI | `choices[].delta`，`[DONE]` | tool_calls index/arguments 缓冲 |
| Claude | `message_start/block_start/delta/stop/delta/stop` | block index、type、text/partial_json/thinking/signature |
| Gemini | `data:` 完整响应对象流 | candidate index、part 合并、thought/function/code 状态机 |

**无损转发建议：**

- OpenAI 客户端模式：把原生流归一化为“text delta / tool_call delta / finish / usage”。OpenAI 没有 Claude `index` 生命周期，需由网关合成 `index`。
- Claude→OpenAI：每个 `content_block_start` 建立 `tool_calls[index]`；`input_json_delta` 写入 arguments；`message_delta.stop_reason` 映射为 `finish_reason`。`message_start.usage` 与 `message_delta.usage` 在流结束前合并。
- Gemini→OpenAI：按 candidate/part 类型合并；text append，functionCall 投影为 `tool_calls`，thought 默认不暴露。最终 chunk 的 `usageMetadata` 映射到 `usage`。
- **不可无损信息**：Claude block 中间状态、Gemini thoughtSignature、多个 candidate、`promptFeedback`、服务器工具结果原生结构。若客户端需要这些能力，应提供 `provider_native_events` 旁路通道。

#### 3.5 `max_tokens` 与采样参数

| 参数 | OpenAI | Claude | Gemini |
|---|---|---|---|
| 最大输出 | `max_completion_tokens`/`max_tokens`（API 版本依赖） | `max_tokens`（**必填**） | `maxOutputTokens` |
| 输入 token | prompt token | input token | promptTokenCount |
| 缓存 token | `cached_tokens`（responses API 口径） | `cache_read/creation_input_tokens` | `cachedContentTokenCount` |
| 思维 token | `reasoning_tokens` | `output_tokens` 内；`usage.thinking_tokens` 视模型 | `thoughtsTokenCount` |
| top_k | 无 | 有 | 有 |

**网关默认值策略：**收到 OpenAI 请求但没有 `max_completion_tokens` 时，根据目标 Claude 模型设置安全默认并写入 `max_tokens`；不能让字段缺失。Gemini `maxOutputTokens` 缺失时由模型默认，但仍建议在网关设置上限以防计费失控。

#### 3.6 `stop_reason` / `finish_reason` 三方映射

| Anthropic `stop_reason` | Gemini `finishReason` | OpenAI `finish_reason` | 建议网关规范值 |
|---|---|---|---|
| `end_turn` | `STOP` | `stop` | `stop` |
| `max_tokens` | `MAX_TOKENS` | `length` | `length` |
| `stop_sequence` | `STOP` | `stop` | `stop`（附原生 reason） |
| `tool_use` | `UNEXPECTED_TOOL_CALL`/普通 function call | `tool_calls` | `tool_calls` |
| `pause_turn` | — | — | `pause`（provider extension） |
| `refusal` | `SAFETY` | `content_filter` | `content_filter` |
| — | `MALFORMED_FUNCTION_CALL` | — | `tool_calls_error` |
| — | `SAFETY/RECITATION/BLOCKLIST/PROHIBITED_CONTENT/SPII` | `content_filter` | `content_filter` |
| — | 空 candidates | — | `error` 或 `content_filter` |

**映射必须可双向追溯。**规范化 IR 应同时保存 `provider_finish_reason` 和标准 `finish_reason`；OpenAI 客户端只承诺标准值，管理面/调试面暴露原生值。

#### 3.7 Usage 三方映射

| 标准 IR | OpenAI（Chat Completions 历史） | Anthropic | Gemini |
|---|---|---|---|
| `prompt_tokens` | `prompt_tokens` | `usage.input_tokens` | `usageMetadata.promptTokenCount` |
| `completion_tokens` | `completion_tokens` | `usage.output_tokens` | `usageMetadata.candidatesTokenCount` |
| `total_tokens` | `total_tokens` | input+output；cache 不简单相加 | `totalTokenCount` |
| `cached_tokens` | `prompt_tokens_details.cached_tokens` | `cache_read_input_tokens` | `cachedContentTokenCount` |
| `cache_creation_tokens` | — | `cache_creation_input_tokens` | 无直接标准字段 |
| `reasoning_tokens` | `reasoning_tokens` | `usage.output_tokens` 包含；新模型细分 | `thoughtsTokenCount` |
| `tool_use_prompt_tokens` | — | 通常计入 input，视结构 | `toolUsePromptTokenCount` |

**注意分母与包含关系。**Anthropic 的 `cache_read_input_tokens` 是 `input_tokens` 的子集；Gemini 的 `totalTokenCount` 官方说明可能包含 prompt+candidates+tool_use prompt+thoughts，不能简单等于 `promptTokenCount+candidatesTokenCount`[17]。OpenAI 兼容响应应明确 `prompt/completion/total` 的相加关系，把 reasoning/cached 放入 `completion_tokens_details`/`prompt_tokens_details`，避免重复计费。

#### 3.8 其他转换难点

- **`stop_sequences` 与 `stop`：**OpenAI 常用单个 `stop`；Claude/Gemini 均为数组。转换时把 OpenAI `stop` 提升为单元素数组；反向若多值无法表达，返回错误而非静默取第一个。
- **`n` / `candidateCount`：**Claude 无 `n`；Gemini 支持 1–20。OpenAI `n>1` 不能无损转 Claude；应拒绝、顺序生成或投影为单候选。
- **`top_k`：**Claude/Gemini 支持，OpenAI 历史 Chat Completions 不暴露；网关可记录但不得伪造。
- **`presence/frequencyPenalty`：**Gemini 有，Claude/OpenAI 口径不同；未实现时丢弃并告警。
- **`safetySettings`：**OpenAI 无直接等价。Gemini 默认可因 category 覆盖不全而放行；网关可将组织安全策略映射为 Gemini `safetySettings`，并将空 candidates 转错误。不要假定 `BLOCK_NONE` 可关闭所有 Google 策略。
- **思考链不可兼容：**Claude thinking/redacted_thinking、Gemini thought/thoughtSignature 均可能具有签名、加密或顺序约束。OpenAI `reasoning_content` 没有跨 provider 签名协议；推荐策略是**不跨协议复用原始 reasoning**，只选择性投影可见摘要，或在管理员模式透传 provider 原生响应。
- **认证头透传：**Claude 必须带 `anthropic-version`；缺失/被覆盖可能导致版本错误。Google AI Studio 优先 API Key，Vertex 必须 OAuth；不要在 URL query、header、proxy 之间复制 secret。

#### 3.9 网关校验与失败处理清单

1. 请求进入即绑定 `provider/model/version/beta`，校验 schema 前先确定能力矩阵。
2. Claude：`max_tokens` 必填；system 不得 role；tool_result 在 user；thinking/redacted_thinking 原样回传。
3. Gemini：model 在 URL；流式用 `alt=sse`；functionResponse.response 为对象；thoughtSignature 不可丢；空 candidates 必须处理。
4. 流式中保存原始事件、转换状态、request-id；关闭时合成最终 IR。
5. 错误标准化：`provider_error.type`、`http_status`、`request_id`、`retry_after/retryDelay`、`is_retryable`。
6. 限流：Claude 按 token bucket + workspace；Gemini 按 RPM/TPM/RPD + 花费层；都不应依赖单 header 强一致。
7. 重试只用于幂等请求：GET、count tokens、纯生成；tool 调用/副作用前必须 client confirmation 或 idempotency key。
8. 缓存：Claude block cache_control、Gemini cachedContent 是两套机制；用户前缀/工具 schema 变化时失效。
9. 可观测性：原始请求/响应样本、模型路由、cache hit、空 candidate、流式中断、参数转换失败。
10. 契约测试：每个 provider 至少覆盖 text、multi-turn、parallel tool、thinking、stream、safety-empty、429、超大 prompt。

## 第八章　错误、限流与重试：跨协议统一操作手册

前面各章已分别给出每家协议的错误细节，本章做**横向归并**，目标是给网关一个可以直接落地的判定表与决策树。核心判断是：

> **错误处理的难点不是"认不认识这个错误码"，而是"这个错误到底能不能重试"。** 同样是 HTTP 429，OpenAI 的 `rate_limit_exceeded` 可以退避、Anthropic 的"月度花费上限"早重试必败、Google 的 `RESOURCE_EXHAUSTED` 要遵守 `retryDelay`、国内厂商的余额耗尽则完全不该重试。

### 8.1 错误信封：四种结构

```jsonc
// ① OpenAI：message / type / param / code 四件套
{"error":{"message":"You exceeded your current quota...","type":"insufficient_quota","code":"insufficient_quota"}}

// ② Anthropic：外层多一个 type:"error"，附带 request_id
{"type":"error","error":{"type":"not_found_error","message":"..."},"request_id":"req_011CSHoEeqs5C35K2UUqR7Fy"}

// ③ Google：google.rpc.Status 风格，details 里带结构化信息
{"error":{"code":429,"message":"...","status":"RESOURCE_EXHAUSTED",
  "details":[{"@type":"type.googleapis.com/google.rpc.RetryInfo","retryDelay":"30s"}]}}

// ④ 国内厂商：多为 OpenAI 风格 + 私有 code 体系
//    DeepSeek   → {"error":{"message":"...","type":"unknown_error","code":"invalid_request_error"}}
//    火山方舟   → {"error":{"code":"InvalidParameter","message":"...","param":null}}
//    阿里云百炼 → 带点号命名空间 code，如 Throttling / Throttling.RateQuota
//    百度千帆   → 数值型 error_code（如 17 类）+ error_msg
//    腾讯混元   → {"Response":{"Error":{"Code":"...","Message":"..."},"RequestId":"..."}}
//    MiniMax    → OpenAI error + base_resp.status_code 双轨
```

**网关要点**：错误信封必须"全字段留存 + 归一化投影"。原始 `code`/`type`/`http_status`/`request_id` 一律保留到扩展字段，对外只暴露归一化类型，避免向客户端泄露内部模型名、文件路径或 prompt 片段。

### 8.2 可重试性判定矩阵

| HTTP | OpenAI `code`/`type` | Claude `error.type` | Gemini `status` | 国内常见形态 | 判定 |
|---:|---|---|---|---|---|
| 400 | `invalid_value`、`string_above_max_length`、`context_length_exceeded`、`model_not_found` | `invalid_request_error` | `INVALID_ARGUMENT`、`FAILED_PRECONDITION` | 方舟 `InvalidParameter`、千帆参数错误 | **否**，修正请求 |
| 401 | `invalid_api_key`、`incorrect_api_key` | `authentication_error` | `PERMISSION_DENIED`（凭证类） | 各家 key 失效 | **否**，换凭证 |
| 402 | — | `billing_error` | — | **DeepSeek 余额不足**、方舟欠费、千帆欠费 | **否**（极易被误判为 429 而重试） |
| 403 | `unsupported_country_region`、`permission_denied` | `permission_error` | `PERMISSION_DENIED` | 未开通服务/区域限制 | **否** |
| 404 | `not_found_error` | `not_found_error` | `NOT_FOUND` | 模型/资源不存在 | **否** |
| 408 | `request_timeout` | — | `DEADLINE_EXHAUSTED`(408) | 读取超时 | 谨慎，需评估幂等 |
| 409 | `conflict_error` | `conflict_error` | `FAILED_PRECONDITION`(409) | 状态冲突 | 视操作而定 |
| 413 | — | `request_too_large`（上限 32MB） | — | 请求体过大 | 否，需缩小/截断 |
| 422 | `unprocessable_entity_error` | — | — | DeepSeek 参数错误 | **否** |
| 429 | `rate_limit_exceeded`、`slow_down`、`tokens_rate_limit_exceeded` → **可重试** | `rate_limit_error`（令牌桶）→ **可重试** | `RESOURCE_EXHAUSTED` → **可重试，守 retryDelay** | RPM/TPM 超限 → 可重试 | **可**，退避 + 熔断 |
| 429 | `insufficient_quota`、`credit_balance_exhausted`、`organization/project_spend_limit_reached`、`organization_usage_limit_reached` → **不可重试** | 月度花费上限 / workspace 上限 → **不可重试** | 花费型限额（10 分钟滚动窗口）→ 等窗口 | 余额耗尽/配额耗尽 → **不可重试** | **否**，告警并切换渠道或拒绝 |
| 500 | `internal_server_error` | `api_error` | `INTERNAL` | 服务端错误 | **是**，限次 |
| 503 | `service_unavailable_error`、`server_is_overloaded` | — | `UNAVAILABLE` | 模型过载 | **是**，守 `Retry-After` |
| 504 | — | `timeout_error` | `DEADLINE_EXCEEDED`(504) | 网关超时 | 仅幂等请求；长输出建议改流式 |
| 529 | — | `overloaded_error` | — | — | **是**，退避 |

**最容易失控的一条规则**：网关**绝不能**用"所有 4xx 不重试、所有 429 退避"这种粗粒度规则。必须下探到 `error.code` 或 `error.type` 层面，把"速率限流"与"额度/配额/预算耗尽"分开——后者重试不仅无效，还会浪费余额、触发风控，甚至在部分厂商侧放大限流。

### 8.3 限流响应头对照

| 维度 | OpenAI | Anthropic | Gemini | 国内厂商 |
|---|---|---|---|---|
| 请求数 | `x-ratelimit-limit-requests`<br>`x-ratelimit-remaining-requests`<br>`x-ratelimit-reset-requests` | `anthropic-ratelimit-requests-limit`<br>`-remaining` / `-reset` | RPM（文档维度，头不保证） | 多未公开或私有 |
| Token 数 | `x-ratelimit-limit-tokens`<br>`-remaining-tokens` / `-reset-tokens` | `anthropic-ratelimit-tokens-*`<br>`anthropic-ratelimit-input-tokens-*`<br>`anthropic-ratelimit-output-tokens-*` | TPM / RPD（文档维度） | 部分返回 cached tokens 相关字段 |
| 重置时间格式 | **持续时间**：`1s`、`6m0s`、`1d` | **RFC 3339 绝对时间**（requests-reset） | `details[].retryDelay`：`30s` | 不统一 |
| 建议等待 | `Retry-After`（秒） | `retry-after`（**秒数，不是 HTTP-date**） | `RetryInfo.retryDelay` 优先，其次 `Retry-After` | 部分无 |
| 排障 ID | `x-request-id` | `request_id`（响应体内） | 请求日志 / `x-request-id` | `X-Request-Id`、方舟 `X-Client-Request-Id`、混元 `RequestId` |

**限流头是"建议值"而非"精确账本"。** 令牌桶持续补充，突发配额可能使头与瞬时用量不一致；OpenAI 的 `limit-tokens` 与 usage tier 强相关，Claude 的组织级限额可由 Workspace 自定义。网关应基于"剩余量 + reset + Retry-After"做**建议性**限流与本地熔断，而不是把它当强一致计数器。

### 8.4 统一重试决策规则

```
function decide_retry(err):
    # 第一层：HTTP 状态码粗筛
    if err.http in {401, 402, 403, 404, 413, 422}: return NO_RETRY      # 凭证/权限/资源/参数
    if err.http == 400: return NO_RETRY                                 # 除非网关自身能修正
    if err.http == 429:
        # 第二层：下探 code 语义，区分「速率」与「额度」
        if err.code in QUOTA_CODES:      return NO_RETRY + ALERT + SWITCH_CHANNEL
        if err.code in RATE_CODES:       return BACKOFF(retry_after or expo)
    if err.http in {500, 503, 529}:      return BACKOFF(retry_after or expo)
    if err.http in {408, 504}:           return RETRY_ONLY_IF_IDEMPOTENT
    if err is NETWORK_ERROR:             return BACKOFF (仅当尚未收到任何字节)
```

配套的硬约束：

| 约束 | 建议值 / 做法 |
|---|---|
| 退避算法 | 指数退避 + 抖动（jitter），基准 0.5~1s，上限 30~60s |
| 最大尝试次数 | 2~3 次（含首次），总耗时上限单独设闸 |
| `Retry-After` 优先级 | 高于自算退避值，作为**最小**等待时长 |
| 重试放大 | 网关层统一设置 SDK `maxRetries`，**禁止**应用层 SDK 与边缘网关各自重试造成倍数放大 |
| 熔断 | 同一渠道连续失败达阈值即熔断，切换备用渠道 |
| 连接错误 | 仅在**尚未收到任何响应字节**时才可安全重试；已收到首字节说明服务端已开始生成 |

### 8.5 幂等性：LLM 没有原生幂等键

这是网关设计中最容易被忽略的风险点。

- **不存在服务端去重**：即便携带同一个 `x-request-id`，也不应被假定为幂等。网络失败可能发生在模型已开始计费、已产生工具副作用、已部分写入存储之后。
- **正确做法**：
  1. 连接层短超时重试；
  2. 业务层对"已持久化 `response_id` / tool result"的请求做应用级幂等；
  3. 对不可重放的工具（付款、写库）必须由工具服务端提供幂等键；
  4. 流式客户端允许接收重复事件，输出层按 `(output_index, content_index, item_id)` 去重。
- **渠道切换的前提是幂等**：已产生计费或部分工具副作用的请求，不能自动切换到另一渠道重放。

### 8.6 错误归一化对象设计

网关对外的错误响应建议固定为三层结构——**对外类型 + 原始错误 + 可重试决策**：

```jsonc
{
  "error": {
    "type": "rate_limit",            // rate_limit | authentication | permission | bad_request
                                     // | not_found | server | content_policy
    "code": "provider-native-code",  // 原始 code，便于排障与白名单
    "message": "human readable",
    "param": null,
    "retryable": false,              // ← 网关重试器唯一依赖的字段
    "request_id": "gateway-request-id",
    "extensions": {
      "provider": "deepseek",
      "http_status": 429,
      "raw_code": "RateLimitExceeded.EndpointRPMExceeded",
      "provider_request_id": "..."
    }
  }
}
```

重试器**只依赖** `(type, retryable, http_status, raw_code)` 四元组，不解析 message 文本——厂商的错误文案随时可能变化，依赖文本匹配的重试逻辑必然在某次文档更新后失效。

## 第九章　自研 OpenAI 兼容网关：落地清单

本章整合前三份研究中分散的工程结论。**与第七章的分工**：第七章解决"字段怎么映射"，本章解决"系统怎么搭、坑怎么躲"。

### 9.1 架构流水线：三层对象模型

网关内部至少应维护三层对象，**任何"直接把 OpenAI JSON 改名成目标协议 JSON"的方案都会在处理 thinking、tool result、流式生命周期、空 candidates 时失真**：

```text
客户端 OpenAI 请求
  → 规范化 IR (model, system, messages, tools, sampling, stream, reasoning)
  → provider 原生请求
  → provider 原生响应 / 原生事件流
  → 规范化 IR Response
  → OpenAI ChatCompletion / chunk 投影
```

关键约束：

1. **保留 provider 原生对象**：Claude 的 `content_block_start/delta/stop`、Gemini 的 `thoughtSignature`、Responses 的 `encrypted_content` 都有顺序或签名约束，一旦被"规范化"破坏就无法回传。若客户端需要这些能力，应提供 `provider_native_events` 旁路通道，而不是塞进 OpenAI 字段。
2. **IR 同时保存双份停止原因**：`provider_finish_reason`（原始值）+ 标准 `finish_reason`（对客户端承诺的标准值）。映射必须可双向追溯，管理面/调试面暴露原生值。
3. **鉴权头白名单透传**：仅透传 `Authorization`、`OpenAI-Organization`、`OpenAI-Project`、`anthropic-version` 等必需头，默认剥离其余请求头（注意所有请求头合计应低于 64KiB）。

### 9.2 OpenAI 双协议：十大高频坑

**坑 1：把 Responses 事件硬翻译为 Chat delta。** 可实现的最小子集是 `output_text.delta → content`、`function_call_arguments.delta → tool_calls.arguments`、`response.completed → final choice`；但 reasoning、annotations、refusal、image/code/file/computer/mcp 事件没有 Chat 官方对应物。更稳妥的做法是**双协议原生暴露**，Chat 兼容层只翻译"文本 + 函数调用 + finish_reason"，其余放入扩展字段或丢弃并告警。

**坑 2：`tool_call.index` 与 `call_id` 映射错误。** Chat 流中 `tool_calls[].index` 是合并主键；Responses 中由 `output_index`/`item_id`/`call_id` 共同定位。向 Chat 转换时应稳定生成 `call_*` id，并保留 `call_id → tool_call_id` 双向表；反向转换不得丢失未完成的 arguments。

**坑 3：arguments 的 JSON 边界。** 增量可为空字符串、可跨转义字符切断、可在 finish 后仍未闭合；只有 `done` 事件才是官方终止点。**保留 raw buffer，绝不自行补全括号、修复控制字符或解析前缀**。

**坑 4：SSE 缓冲与帧解析。** 必须设置 `X-Accel-Buffering: no`、Nginx `proxy_buffering off`、`chunked_transfer_encoding on`，禁用 gzip buffering。按 `\n\n` 切分事件，先匹配 `data:`；`data: [DONE]`、注释行、多行 `data`、无 payload 的 event 都要能处理。**代理超时必须大于模型最大生成时间。**

**坑 5：usage 缺失与字段名混用。** Chat 是 `prompt/completion/total`，Responses 是 `input/output/total`，details 子字段也不同。不要在指标采集里做"自动别名"，应显式区分协议维度。流式中 Chat 需 `stream_options.include_usage`，Responses 的 `response.completed` 通常带 usage 但连接中断时会丢失。

**坑 6：Responses 状态字段丢失。** `status:"incomplete"` 不等于成功；`incomplete_details`、工具调用状态、reasoning summary 都必须透传。不能把 `cancelled`/`queued`/`in_progress` 一律映射为 HTTP 200。

**坑 7：reasoning / encrypted_content 被规范化破坏。** 不得解码、排序 map keys、压缩空格或重新序列化；无状态第二轮必须原样回传。日志中只记录 item id 与长度，**不记录明文加密 blob**。

**坑 8：strict schema 子集误判。** `strict:true` 下不支持的组合会直接报错。网关若动态生成 JSON Schema，应在转发前做白名单校验，而不是把 OpenAI 的错误泛化为 500。

**坑 9：SDK 对 `object`/`type` 的严格校验。** 客户端可能 switch `object:"chat.completion"` 或 `type:"response.completed"`。伪造 Chat 响应时必须完整填充 `id`/`object`/`created`/`model`/`choices`；伪造 Responses 时填充 `object:"response"`、完整事件 `type` 与 `sequence_number`。**不要只返回 `data:` 裸 JSON。**

**坑 10：错误事件与 HTTP 错误的双重处理。** Responses 流中的 `error` 是应用层协议事件，HTTP 200 仍可能发生。网关既要转换 SSE 错误事件，也要在 TCP/HTTP 失败时正确关闭流；向 Chat 客户端适配时，把流中不可恢复错误转为标准 error event 并记录 `response_id`，而不是突然断连。

### 9.3 流式归一化：用内部事件集抹平五种流模型

流式转换器是网关最关键的组件。建议内部只保留一组规范事件：

```text
Event = message_start | reasoning_delta | content_delta
      | tool_call_delta | usage | message_stop | error
```

各协议的驱动负责把 SSE / NDJSON / 具名事件 / 完整对象流都转成这组内部事件，再对外渲染成标准 OpenAI delta。映射关系：

| 内部事件 | OpenAI Chat | OpenAI Responses | Claude | Gemini | Ollama NDJSON |
|---|---|---|---|---|---|
| `message_start` | 首 chunk（带 `role`） | `response.created` / `output_item.added` | `message_start`（**此处取 input tokens**） | 首 chunk | 首个对象（含 `done:false`） |
| `content_delta` | `delta.content` | `output_text.delta` | `content_block_delta.text_delta` | chunk 中 `parts[].text` | `message.content` |
| `reasoning_delta` | `delta.reasoning_content`（部分实现） | `reasoning_summary_text.delta` | `thinking_delta` | `parts[].thought` | `message.thinking` |
| `tool_call_delta` | `delta.tool_calls[]`（`index` 合并） | `function_call_arguments.delta` | `input_json_delta.partial_json` | `functionCall` part | `message.tool_calls` |
| `usage` | 末包（`include_usage`） | `response.completed` | `message_start` + `message_delta` 合并 | 末 chunk `usageMetadata` | 末对象性能字段 |
| `message_stop` | `finish_reason` + `[DONE]` | `response.completed` | `message_stop` | 流关闭 | `done:true` |
| `error` | HTTP 错误 / 流中断 | `error` 事件 | `error` 事件 | HTTP 错误 | `{error}` 字段 |

**流式结束的判定优先级**（严格按序，不可颠倒）：

```text
上游显式终止符（[DONE] / done:true / response.completed / message_stop）
  > 上游正常关闭且已收到 terminal finish_reason
  > 客户端主动取消
  > 超时
  > 网络异常  → 永远不伪造成功
```

**解析器必须容忍的畸形输入**：`: ` 注释行与心跳、首 chunk 无 `role`、tool call 中间 chunk 重复 `role`、末包仍带 `delta`、`delta: null`、空 JSON 对象、未发送 `[DONE]` 就断开。连接关闭但无终止符时，按 cancel / network error 处理。

> 以下 9.4–9.6 节从厂商与开源生态视角补充差异矩阵、陷阱清单与 canonical 架构设计，与 9.1–9.3 节互为补充。

#### 9.4 差异速查矩阵

**矩阵的结论是：没有任何国内厂商可以仅靠 OpenAI SDK 获得完整的“思考、缓存、限流、错误”兼容。** 下表 `✅` 表示官方文档已明确或可直接迁移；`⚠️` 表示部分模型/版本支持或需实测；`❌` 表示本次未确认，不宣称支持。所有 OpenAI 兼容端点均应在生产前跑契约测试。

| Provider | base_url/鉴权 | OpenAI 兼容层 | SSE | `[DONE]` | 非流式 usage | 流式 usage | 思考字段/开关 | tool/JSON schema | 限流头/体系 | 错误码体系 | 私有扩展风险 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| DeepSeek | `/v1`、Bearer | 高 | ✅ | ✅ | ✅ | `include_usage` | `reasoning_content`/`thinking` | ⚠️/⚠️ | 官方页未全展示 | 402/422 明确 | 缓存字段、思考回传 |
| 百炼 Qwen | workspace 域名、`/compatible-mode/v1`、Bearer | 高 | ✅，私有 header | ⚠️实测 | ✅ | ⚠️实测 | 模型快照相关 | ⚠️/`json_object`确认 | 区域/workspace | OpenAI 映射+原生 code | `X-DashScope-*`、搜索 |
| 方舟 Seed | `/api/v3`、Bearer/AK | 高，多协议 | ✅ | ⚠️实测 | ✅ | ⚠️实测 | `reasoning_content`；明文/加密 | 模型相关 | RPM 错误码 | `InvalidParameter` 等 | 思考回传、endpoint |
| Kimi | `/v1`、Bearer | 高 | ✅ | ⚠️实测 | ⚠️ | ⚠️ | `extra_body.thinking` | 历史差异，当前快照测试 | 未确认 | `error.type/message` | `partial`、搜索 |
| GLM-5 | `/api/paas/v4`、Bearer | 高 | ✅ | ⚠️实测 | ⚠️ | `include_usage`推荐 | `thinking`/`reasoning_content` | 快照测试 | 未确认 | 官方页未完整展示 | 思考回传 |
| 千帆 V2 | `/v2` | 中/取决于层 | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️/⚠️ | 原生 SDK 与 V2 并存 | 原生 `error_code` | 鉴权双轨、原生消息约束 |
| 混元 native | TC3 签名 API 3.0 | ❌OpenAI | TC3 SSE | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️/⚠️ | 默认 5 并发 | RequestId/Code | 消息顺序、空 content、签名 |
| MiniMax | `/v1`、Bearer | 高 | ✅ | ⚠️实测 | ✅ | ⚠️ | `thinking.type`/`reasoning` | ⚠️/⚠️ | service tier | `base_resp`+HTTP | sensitive、`cached_tokens` |
| 讯飞星火 | `/v1`、Bearer | 高 | ✅ | ⚠️实测 | ✅ | ⚠️ | ⚠️ | ✅/`json_object` | 数字流控码 | 10007/11200–11203 | 版本上下文差异 |
| StepFun | `/v1`、Bearer | 高 | ✅ | ⚠️实测 | ⚠️ | ⚠️ | `reasoning_effort` | ⚠️/⚠️ | 未确认 | 未确认 | effort 映射 |
| Ollama native | `11434`、`/api/chat` | ❌OpenAI | NDJSON，非 SSE | `done` | ✅ | NDJSON | `message.thinking`/`think` | 模型相关 | 本地部署 | `{error}`/HTTP | options、keep_alive |
| Ollama `/v1` | 11434、`api_key=ollama` | 部分 | ✅ | ✅ | ✅ | ✅ | thinking control | ⚠️/✅ | 本地 | 透传后端 | 部分兼容限制 |
| vLLM | 自定义、`--api-key` | 高但版本敏感 | ✅ | ⚠️版本 | ✅ | flag 相关 | parser/版本相关 | extra/guided | 自定义 | 版本相关 | extra_body、LoRA、guided |
| LiteLLM proxy | `/v1`、Bearer | 高，转换层 | 归一化 | 归一化 | 归一化 | `always_include` | 路由/parser 决定 | 上游决定 | proxy 层 | 统一 OpenAI 风格 | 模型前缀、成本扩展 |
| SGLang/LocalAI/FastChat/Xinference/llama.cpp | `/v1` | 中/高 | 多数 SSE | ⚠️ | ⚠️ | ⚠️ | 各自 parser | 各自能力 | 自建 | 自建/版本相关 | tool/reasoning/audio |

#### 9.5 兼容层陷阱清单与规避方案

**陷阱 1：流式 usage 缺失会让计费和可观测失效。** DeepSeek 明确要求 `include_usage` 才返回完整统计，许多厂商不会主动发送末包 usage。[23] 规避方案：网关默认注入 `stream_options.include_usage=true`；若上游不支持该字段，从请求失败/未知参数、连接中断、超时中识别，不能静默伪造。真实统计优先；估算统计仅在显式开关下启用，并标记 `usage.extensions.estimated=true`。

**陷阱 2：`reasoning_content`、`thinking`、加密思考块不是同一个语义。** DeepSeek、GLM、部分方舟明文模型使用 `reasoning_content`；MiniMax 示例中思考控制更接近 OpenAI Responses 风格；方舟加密模型不能只存摘要；Kimi 用 `partial`。规避：内部定义 `ReasoningPart{format,text,cipher_blob,effort,model_version}`，下游只在明确模型策略下序列化。永远不要把 reasoning 写入普通可显示 content，除非用户明确要求。

**陷阱 3：`[DONE]`、SSE 注释、首个 role 和重复 role 会使标准客户端挂起或报错。** 有的服务发送 keep-alive 注释、首 chunk 无 `role`、tool call 中间 chunk 重复 `role`、末包仍带 `delta`。规避：解析器必须容忍 `: ` 注释、`data: [DONE]`、空 JSON、`delta=null`；首个有效 delta 补 `role=assistant`；同一 `index` 合并 `tool_calls.id/function.arguments`。连接关闭但无 `[DONE]` 时，按 cancel/network error 处理，不视为正常完成。

**陷阱 4：`finish_reason` 私有值会被严格 SDK 校验拒绝。** DeepSeek 历史上出现 `insufficient_system_resource`，vLLM 可能有 `abort`，各家混用 `tool_calls/stop`。规避：保留 `finish_reason` 原始值至 `extensions`，对外 canonical 映射为 `stop`、`tool_calls`、`length`、`content_filter`、`cancelled`、`provider_specific`。不要抛错未知值。

**陷阱 5：错误码不一致会让重试放大器失控。** DeepSeek 402、方舟欠费、千帆 17、讯飞 11200 都可能“看起来像 4xx”，但重试会浪费额度或触发风控。规避规则：402/401/403 不重试；400/422 仅在可修正状态后重试；429/503/504 使用退避与 circuit breaker；5xx 单实例隔离。每个 provider 返回原始 `http_status`、`code`、`type`、`message`、`request_id`。

**陷阱 6：采样参数区间、默认值和含义不一致。** DeepSeek temperature≤2、默认 1；OpenAI 常见 0–2；Ollama 常用 0–1；`top_k`、repeat_penalty 没有 OpenAI 标准对应。规避：请求规范化为 canonical `{temperature,top_p,max_output_tokens,stop,seed,tools}`；provider driver 负责裁剪、范围映射和未知字段丢弃。拒绝把 `temperature=0` 全局改写为 `0.0001`，除非该渠道明确要求。

**陷阱 7：`max_tokens` 与 `max_completion_tokens` 不能无脑复制。** MiniMax 已展示 `max_completion_tokens`；DeepSeek 官方 Chat 文档主要使用 `max_tokens`；OpenAI 新模型倾向 `max_completion_tokens`。规避：canonical 使用 `max_output_tokens` 和 `include_reasoning_tokens`；按模型版本选择字段名。思考模型还需确认上限是否包含 reasoning tokens，否则请求会被截断。

**陷阱 8：多模态内容私有化。** 百炼、方舟、MiniMax、GLM、StepFun 的 image/video/audio 结构、Base64、file_id、detail 字段不同。规避：内部采用 `ContentPart{text,image{url,bytes,detail},audio,video,file_id}`，每个 provider 序列化自有结构；未知 part 返回 400，不丢弃。

**陷阱 9：缓存统计不能直接相减或重复计费。** DeepSeek 明确区分 cache hit/miss；MiniMax 展示 `prompt_tokens_details.cached_tokens`；百炼、方舟、GLM、Kimi、StepFun 的缓存可见性本次未全部确认。规避：canonical 定义 `cache_read_input_tokens`、`cache_creation_input_tokens`，只有上游明确字段才填；总 token 以 `prompt+completion` 为准，避免把 details 重复加总。

**陷阱 10：`id`、时间戳、空/ null content 与 request id 差异。** OpenAI 常见 `chatcmpl-`；国内服务可能为其他格式；created 可能是秒或毫秒；有些消息 content 为 `""`，有些为 `null`。规避：网关重写对外 `id`、`created` 为规范值，原始 `id` 放入扩展；content 默认 `""`，只有上游明确 `null` 且客户端严格时转为 `null`；保留 `X-Request-Id`/上游 request id 映射。

#### 9.6 建议的兼容层架构

**canonical 应是“OpenAI Chat 超集”，而不是最小 OpenAI 子集。** 推荐内部请求模型：

```json
{
  "spec_version":"1.0",
  "model":"{tenant_channel}::{provider_model}",
  "messages":[{"role":"system|user|assistant|tool","content":"...|[]","tool_calls":[],"reasoning":null}],
  "sampling":{"temperature":1.0,"top_p":1.0,"max_output_tokens":8192,"stop":[],"seed":null},
  "tools":[],"tool_choice":null,
  "reasoning":{"enabled":true,"effort":"high"},
  "stream":true,"stream_options":{"include_usage":true},
  "provider_params":{"deepseek":{"thinking":{"type":"enabled"}}},
  "extensions":{"request_id":"...","user_id":"...","budget":"..."}
}
```

这里的关键是把一切非 OpenAI 字段放进 `provider_params.{provider}` 与 `extensions`，而不是污染顶层。这样渠道驱动可按模型快照决定是否发送，也能避免不同厂商相同字段名产生语义冲突。

**字段白名单与显式透传应双轨运行。** 默认只允许 OpenAI 标准字段进入 upstream；`extra_body`/provider-specific params 必须通过渠道配置启用，并在请求前后做 schema diff。上线流程：官方文档版本 → 模型能力快照 → contract test → 允许列表 → 灰度。禁止把任意客户端 query/header 拼接为 upstream JSON。

**流式归一化是网关最关键的转换器。** 建议内部事件为 `Event=message_start|reasoning_delta|content_delta|tool_call_delta|usage|message_stop|error`。驱动把 NDJSON、SSE、注释行、单 JSON、chunked transfer 都转成内部事件；对外再渲染标准 OpenAI delta。这样可以统一处理 Ollama NDJSON、DeepSeek SSE、方舟加密思考、工具参数增量。结束判定优先顺序：上游 `[DONE]`/`done:true` → 上游正常关闭且收到 terminal finish → 客户端取消 → 超时；网络异常永远不伪造成功。

**usage 兜底统计必须有明确责任边界。** 上游返回真实 usage 时直接归一；上游只返回 prompt/completion 时计算 total；上游缺失时可选本地 tokenizer 估算并标记 estimated；上游返回 cache hit/miss 时分别保存。思考 token 必须放在 `completion_tokens_details.reasoning_tokens`，不重复加到 `completion_tokens`。本地统计不得用于强制扣费，只能用于告警、调试和降级。

**错误归一化采用三层结构：对外类型、原始错误、可重试决策。** 对外响应固定为：

```json
{
  "error":{
    "type":"rate_limit|authentication|permission|bad_request|not_found|server|content_policy",
    "code":"provider-native-code",
    "message":"human readable",
    "param":null,
    "retryable":false,
    "request_id":"gateway-request-id",
    "extensions":{"provider":"deepseek","http_status":429,"raw_code":"RateLimitExceeded.EndpointRPMExceeded","provider_request_id":"..."}
  }
}
```

网关重试器只依赖 `(type, retryable, http_status, raw_code)`。402、余额/欠费、配额、参数、权限均 `retryable=false`；429、503、超时、连接错误为 `retryable=true`，但需 channel circuit breaker。切换渠道的前提是请求幂等：已产生计费或部分工具副作用的请求不能自动重放。

**多轮 reducer 必须保存完整原始 assistant。** 对思考模型，历史至少保留 `content`、`reasoning_content`/cipher blob、`tool_calls`、`finish_reason`、模型版本和回传策略。回传规则由 provider driver 决定，不采用“全局删 reasoning”或“全局拼 reasoning”。对 Kimi `partial`、方舟加密思考等扩展，在 canonical 中显式保存，避免被通用 message 清洗逻辑破坏。

**契约测试应成为协议文档的自动证据。** 每个渠道/模型版本至少覆盖：非流式与流式 chat、空/长 system、tool call 与并行 call、tool result 回传、思考开/关、JSON mode、`max_output_tokens` 截断、缓存复用两次请求、`stream_options.include_usage`、取消连接、401/400/429/5xx、首个/末个 SSE chunk、重复 role、注释行、缺失 usage、异常 content/null。测试报告应自动生成“差异速查矩阵”，而非只保存手写文档。

## 附录 A　契约测试清单

**契约测试应成为协议文档的自动证据，而不是一次性集成验证。** 每个渠道 / 模型版本上线前至少覆盖以下用例，测试报告自动生成"差异速查矩阵"并回写文档，避免手写文档与线上行为脱节。

### A.1 基础生成

| # | 用例 | 通过标准 |
|---:|---|---|
| 1 | 非流式纯文本 | 返回标准信封，`id`/`object`/`created`/`model`/`choices`/`usage` 齐全 |
| 2 | 流式纯文本 | 首个事件、中间 delta、终止符齐全，拼接后与非流式结果语义一致 |
| 3 | `max_output_tokens` 截断 | `finish_reason == length`，内容被正确截断而非报错 |
| 4 | `stop` / `stop_sequences` | 命中停止串即停，且能区分单值 `stop` 与数组 `stop_sequences` |
| 5 | `temperature` / `top_p` 边界 | 各家取值区间差异被网关正确裁剪，不被上游 400 |
| 6 | `n > 1` / `candidateCount` | 明确支持、降级为单候选、还是拒绝（Claude 无 `n`，不可伪造） |
| 7 | 空 / 超长 system | 不报错；超长时返回可识别的错误而非 500 |
| 8 | `seed` 可复现性 | 记录是否真正生效（多数厂商不保证） |

### A.2 多轮与上下文

| # | 用例 | 通过标准 |
|---:|---|---|
| 9 | 三轮对话 | 上下文被正确携带，角色映射无误（含 `assistant ↔ model`） |
| 10 | assistant prefill | 末条 assistant 消息能引导续写（Claude 支持，OpenAI 需特定写法） |
| 11 | 超长上下文 | 触发的错误类型可识别（400 `context_length_exceeded` vs 413 vs 私有码） |
| 12 | 缓存复用 | 连续两次相同前缀请求，第二次 `cached_tokens` / cache hit 字段有体现 |
| 13 | 思考块回传 | 按 provider 策略原样回传（Claude thinking、Gemini thoughtSignature、方舟加密思考），不被清洗逻辑破坏 |

### A.3 工具调用

| # | 用例 | 通过标准 |
|---:|---|---|
| 14 | 单工具调用 | `tool_calls` 结构完整，参数可 `JSON.parse` |
| 15 | 并行工具调用 | 多个 `tool_calls` 的 `index` 正确，arguments 各自独立拼接 |
| 16 | **流式参数增量拼接** | arguments 跨 chunk、跨转义字符、出现空串、finish 后才闭合，均能正确聚合 |
| 17 | 工具结果回填 | 按目标协议正确构造（`role:"tool"` / `tool_result` block / `functionResponse` part） |
| 18 | 工具报错回传 | `is_error` / 错误内容传入后模型能自愈或正确报错 |
| 19 | 模型调用不存在的函数 | 网关有兜底，不把幻觉参数透传给业务执行 |
| 20 | `tool_choice` 各取值 | `auto`/`none`/`required`/指定函数 行为符合预期 |
| 21 | 多轮工具循环 | 至少两轮调用-回填，上下文顺序不被打乱 |

### A.4 结构化输出与多模态

| # | 用例 | 通过标准 |
|---:|---|---|
| 22 | `json_object` 模式 | 返回合法 JSON |
| 23 | `json_schema` + strict | schema 子集不支持时返回可识别错误，而非被网关泛化为 500 |
| 24 | `refusal` / 内容策略拒绝 | 有明确字段或错误，不表现为空内容 |
| 25 | 图片输入（base64 / URL） | 各家私有结构与 `detail` 参数正确转换 |
| 26 | 音频 / 文件 / PDF 输入 | 未支持时明确拒绝而非静默丢弃 |
| 27 | 结构化输出 + 流式 | 流式下 schema 仍成立，末包内容可解析 |

### A.5 流式边界与健壮性

| # | 用例 | 通过标准 |
|---:|---|---|
| 28 | 首 chunk 无 `role` | 解析器不报错，能自行补 `role:"assistant"` |
| 29 | 重复 `role` / 末包仍带 `delta` | 能容忍，不重复输出 |
| 30 | 心跳与注释行 | 不解析失败，不作为内容输出 |
| 31 | 缺失 `[DONE]` / `done:true` | 按异常终处理，不视为成功 |
| 32 | 流中途 `error` 事件 | 能捕获并转为标准错误，不静默截断 |
| 33 | 客户端中途断开 | 上游连接被正确关闭，不泄漏连接 |
| 34 | 代理缓冲 | Nginx 等中间层不缓冲 SSE（`proxy_buffering off`） |
| 35 | 缺失 usage | 触发兜底策略并标记 `estimated`，不静默伪造 |

### A.6 错误、限流与计费

| # | 用例 | 通过标准 |
|---:|---|---|
| 36 | 401 无效 key | 不重试，返回归一化 `authentication` |
| 37 | **402 / 余额不足** | **不重试**，与 429 严格区分，触发告警与渠道切换评估 |
| 38 | 429 速率限流 | 遵守 `Retry-After` / `retryDelay` 退避，重试次数有上限 |
| 39 | 429 额度/配额耗尽 | 不重试，归一化 `type` 与速率类 429 可区分 |
| 40 | 500 / 503 / 529 | 退避重试，限次；连续失败触发熔断 |
| 41 | 超时（408 / 504） | 仅幂等请求重试；长请求建议自动改流式 |
| 42 | 各家私有错误码 | 原始 `code` 留存于 `extensions`，归一化 `type` 正确 |
| 43 | usage 一致性 | `total` 与各分项关系自洽；cached / reasoning tokens 不重复计入 |
| 44 | 并发与 RPM 限制 | 本地限流器生效，不触发上游风控 |
| 45 | request id 贯通 | 网关 `request_id` 与上游 id 双向可查，日志可关联 |

## 附录 B　引用来源

> 以下为全报告引用的一手来源，统一编号。标注"官方"的为厂商/项目官方文档。

[1] https://platform.openai.com/docs/api-reference/introduction
> “Use this reference to look up OpenAI API endpoints, request and response schemas, streaming events, client library methods, and shared behavior such as authentication, errors, rate limits, and request IDs.”

[2] https://platform.openai.com/docs/guides/migrating-to-responses-api
> “In Responses, you receive an array of Items labeled output. … Reasoning models have a richer experience in the Responses API with improved tool usage.”

[3] https://platform.openai.com/docs/api-reference/chat/completions/create
> “This value is now deprecated in favor of max_completion_tokens, and is not compatible with o-series models.”

[4] https://platform.openai.com/docs/api-reference/responses/create
> “reasoning.encrypted_content: Includes an encrypted version of reasoning tokens in reasoning item outputs.”

[5] https://platform.openai.com/docs/guides/error-codes
> “Retry with full input context and previous_response_id set to null.”

[6] https://platform.openai.com/docs/guides/function-calling
> “all fields in properties must be marked as required.”

[7] https://platform.openai.com/docs/guides/structured-outputs
> “additionalProperties: false must always be set in objects”

[8] https://platform.openai.com/docs/guides/rate-limits
> “x-ratelimit-reset-tokens 6m0s The time until the token rate limit resets to its initial state.”

[9] https://platform.openai.com/docs/guides/streaming-api-responses
> “The Responses API uses semantic events for streaming. Each event is typed with a predefined schema.”

[10] **Anthropic：Streaming Messages API**  
https://docs.anthropic.com/en/api/messages-streaming

[11] **Anthropic：Token Counting / Count Tokens**  
https://docs.anthropic.com/en/api/messages-count-tokens

[12] **Anthropic API Reference：Getting Started**  
https://docs.anthropic.com/en/api/getting-started

[13] **Google AI for Developers：Gemini API Text Generation**  
https://ai.google.dev/gemini-api/docs/text-generation

[14] **Google Cloud：Generate Content with the Gemini API**  
https://cloud.google.com/vertex-ai/gemini/docs/generate-content

[15] **Google AI for Developers：Gemini API 参考**  
https://generativeai-dot-devsite-v2-prod-3p.appspot.com/api/generate-content

[16] **Google AI for Developers：Gemini API 缓存文档**  
https://ai.google.dev/gemini-api/docs/caching

[17] **Google API Types：UsageMetadata**  
https://googleapis.github.io/js-genai/release_docs/interfaces/types.UsageMetadata.html

[18] **Abacus.AI：Anthropic Messages API Reference**  
https://abacus.ai/help/developer-platform/route-llm/anthropic-messages

[19] **Anthropic：Prompt Caching**  
https://docs.anthropic.com/en/api/prompt-caching

[20] **Google AI for Developers：Gemini API Troubleshooting / Retries**  
https://ai.google.dev/gemini-api/docs/troubleshooting

[21] **Google AI for Developers：Gemini API Rate Limits**  
https://ai.google.dev/gemini-api/docs/rate-limits

[22] **Google AI for Developers：Gemini Context Caching**  
https://ai.google.dev/gemini-api/docs/context-caching

[23] https://api-docs.deepseek.com/zh-cn/api/create-chat-completion
> “include_usage boolean 如果设置为 true,流式返回的所有块都会包含 usage 字段。”

[24] https://api-docs.deepseek.com/guides/thinking_mode
> “Thinking mode does not support the temperature, top_p, presence_penalty, or frequency_penalty parameters.”

[25] https://api-docs.deepseek.com/zh-cn/quick_start/error_codes
> “402 - 余额不足…429 - 请求速率达到上限。”

[26] https://help.aliyun.com/en/model-studio/compatibility-of-openai-with-dashscope
> “The request parameters are aligned with the OpenAI interface.”

[27] https://volcengine.com/docs/82379/2123275
> “Authorization: Bearer $ARK_API_KEY”

[28] https://volcengine.com/docs/6645/1330626
> “extra_headers={'X-Client-Request-Id': '202406251728190000B7EA7A9648AC08D9'}”

[29] https://console.volcengine.com/ark/region:cn-beijing/docs/82379/2636748?lang=zhcv
> “choices[].message.reasoning_content”

[30] https://www.volcengine.com/docs/56651/1282849
> “429 | TooManyRequests | RateLimitExceeded.EndpointRPMExceeded”

[31] https://platform.moonshot.cn/docs/api/overview
> “thinking 参数需要通过 SDK 的 extra_body 传递；partial 是写在 messages 中 assistant 消息上的字段。”

[32] https://docs.bigmodel.cn/cn/guide/models/text/glm-5
> “thinking: { 'type': 'enabled' }”
> “chunk.choices[0].delta.reasoning_content”

[33] https://ai.baidu.com/ai-doc/WENXINWORKSHOP/tlmyncueh
> “错误信息包含: error_code 错误码 error_msg 错误描述信息。”

[34] http://qianfan.readthedocs.io/
> “在使用千帆 SDK 之前，用户需要百度智能云控制台…获取 Access Key 与 Secret Key。”

[35] https://cloud.tencent.cn/document/api/1729/101837
> “本接口支持流式或非流式调用,当使用流式调用时为 SSE 协议。”
> “默认该接口下单账号限制并发数为 5 路。”

[36] http://platform.minimax.io/docs/api-reference/text-chat
> “Usage…prompt_tokens_details: { cached_tokens: 114 }”
> “thinking: { type: 'adaptive' }”

[37] https://www.xfyun.cn/doc/spark/HTTP%E8%B0%83%E7%94%A8%E6%96%87%E6%A1%A3.html
> “base_url 需要配置为:https://spark-api-open.xf-yun.com/v1/”
> “11203 授权错误:并发流控超限。”

[38] https://platform.stepfun.com/docs/zh/api-reference/chat/streaming
> “reasoning_effort: 'medium'”
> “base_url='https://api.stepfun.com/v1'”

[39] https://docs.ollama.com/api/chat
> “The final response object will include statistics and additional data from the request.”

[40] https://github.com/ollama/ollama/blob/main/docs/api.md
> “think: (for thinking models) should the model think before responding?”

[41] https://ollama.com/blog/streaming-tool
> “Streaming responses with tool calling”

[42] https://docs.ollama.com/api/openai
> “Ollama provides compatibility with parts of the OpenAI API.”

[43] https://vllm.website.cncfstack.com/serving/online_serving/openai_compatible_server/
> “The X-Request-Id HTTP request header can be enabled with --enable-request-id-headers.”

[44] https://docs.litellm.ai/docs/completion/usage
> “LiteLLM returns the OpenAI compatible usage object across all providers.”

[45] https://docs.litellm.ai/#litellm-proxy-server-llm-gateway
> “LiteLLM maps exceptions across all supported providers to the OpenAI exceptions.”
