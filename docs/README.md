# llmwire 文档总览

> 状态：活跃　|　最后更新：2026-09-14
> 本文件是文档体系的**地图与规则**。不知道读哪份时，先读这里。

## 1. 为什么这样分层

文档按**变更频率 + 读者**分层，而不是按内容主题堆叠。原因：agent 每次只加载少量上下文，一旦多份文档内容重叠，就会在冲突处"猜"，而猜错比不会更贵。

- **稳定层**（少改）：`DESIGN.md` —— 定位、架构、模块边界、公开 API。
- **半稳定层**（随协议演进）：`spec/*.md` —— IR 类型、流式模型、各协议字段映射。
- **活跃层**（高频更新）：`PLAN.md` —— 里程碑、进度、验收。
- **决策层**（只增不改）：`decisions/*.md` —— ADR，回答"为什么"。
- **证据层**（冻结）：`reference/*` —— 协议一手资料，只作证据，不回改成规范。
- **废弃层**：`archive/*` —— 历史提案，不权威。

**单一事实来源**：协议字段细节只存在于 `spec/`；`DESIGN.md` 只能引用、不得复制。`reference/` 不得被改写成规范，规范以 `spec/` 为准。

## 2. 文档清单与状态

| 文件 | 状态 | 读者 | 说明 |
|---|---|---|---|
| `../AGENTS.md` | 活跃 | 所有 agent | 第一入口：命令 + 文档地图 + 优先级 |
| `DESIGN.md` | 冻结-稳定 | 架构/实现 | 正式项目设计：定位、架构、API、不变量、陷阱 |
| `spec/IR.md` | 半稳定 | codec 实现者 | IR 规范：类型、canonical form、IR 级不变量 |
| `spec/STREAMING.md` | 半稳定 | 流式实现者 | 内部事件集、FSM、`feed`/`finish`、终止语义 |
| `spec/chat.md` | 半稳定 | chat codec | OpenAI Chat ↔ IR 双向映射 + 陷阱 + 用例 |
| `spec/messages.md` | 半稳定 | messages codec | Anthropic Messages ↔ IR 双向映射 + 陷阱 + 用例 |
| `spec/responses.md` | 半稳定 | responses codec | OpenAI Responses ↔ IR 双向映射 + 陷阱 + 用例 |
| `PLAN.md` | 活跃 | 接任务者 | M0–M4 里程碑与任务块、验收标准 |
| `TESTING.md` | 半稳定 | 写测试者 | 测试策略 + 契约清单索引 |
| `decisions/000*.md` | 冻结 | 想推翻决策者 | ADR：为什么这么定 |
| `reference/*` | 冻结 | 查证者 | 协议深度研究报告、同类项目分析报告 |
| `archive/*` | 已废弃 | 追溯者 | v0.1 设计提案，勿据此实现 |

## 3. 阅读路径

- **总体把握**：`../AGENTS.md` → `DESIGN.md` → `PLAN.md`
- **实现任一 codec**：`spec/IR.md` → `spec/STREAMING.md` → `spec/<protocol>.md`
- **改流式**：`spec/STREAMING.md` → `DESIGN.md §6、§8`
- **接任务**：`PLAN.md` → 该任务"文档锚点"列出的章节
- **想推翻某设计**：`decisions/<对应 ADR>.md`

## 4. 权威优先级（冲突时）

1. 结构 / 架构 / API → `DESIGN.md`
2. 字段映射 / 语义 → `spec/*.md`
3. 决策缘由 → `decisions/*.md`
4. 协议一手证据 → `reference/*`
5. 历史提案 → `archive/*`（最低）

## 5. 写作规范

- 正文用简体中文；类型名、字段名、命令用英文原文。
- 每份文档顶部写 **状态 + 最后更新**。
- 不复制别处内容；用相对路径交叉引用。
- 编号约定：不变量 `INV-n`、陷阱 `TRAP-n`、验收标准 `AC-n`、里程碑 `M<阶段>-<序号>`。
- 规范句要**可执行**：能被直接转成测试的断言，优于形容词。

## 6. 文档地图

```text
llmwire/
├── AGENTS.md                     # agent 入口
└── docs/
    ├── README.md                 # 本文件
    ├── DESIGN.md                 # 正式项目设计
    ├── PLAN.md                   # 里程碑与任务
    ├── TESTING.md                # 测试策略
    ├── spec/
    │   ├── IR.md
    │   ├── STREAMING.md
    │   ├── chat.md
    │   ├── messages.md
    │   └── responses.md
    ├── decisions/
    │   ├── 0001-bytes-seam.md
    │   ├── 0002-request-level-object.md
    │   ├── 0003-report-first-class.md
    │   ├── 0004-send-not-sync.md
    │   └── 0005-independent-ir.md
    ├── reference/
    │   ├── LLM接口协议格式深度研究报告.md
    │   └── LLM协议转换分析报告.md
    └── archive/
        └── llmwire_Rust协议转换SDK设计提案.md
```

## 7. 已知冲突

> 发现文档冲突时，按 §4 裁决，并在此登记一条。**不要私自在实现里选边。**

| 日期 | 冲突点 | 裁决依据 | 处理 |
|---|---|---|---|
| 2026-09-14 | `ToolUseKind` 含 `Box` 却派生 `Copy` | `spec/IR.md §5` 类型定义 | 移除 `Copy`；已处理 |
| 2026-09-14 | `PartKind` 同时归 `ids.rs` 与 `ir/event.rs` | `DESIGN.md` 模块边界优先 | 归属改为 `ir/event.rs`；已处理 |
| 2026-09-14 | 流式 `Opaque` / `signature_delta` 缺少内部事件表达，`responses` 引用不存在的 `Event::Opaque` | `spec/STREAMING.md` 事件集为准 | 增加 `PartKind::Opaque(OpaqueKind)` 与 `Delta::Opaque`；协议映射改用 `PartDelta`；已处理 |
| 2026-09-14 | `response.incomplete` 引用不存在的 `StopReason::MultipleCandidates` | `spec/IR.md §7`、`spec/responses.md §5` | 按 `incomplete_details.reason` 映射 `MaxTokens` / `ContentFilter` / `Other` 并上报；已处理 |
| 2026-09-14 | Chat 空 `delta.content` 与“首个非空”冲突 | `spec/chat.md CHAT-TRAP-5` | 明确空字符串也算 `delta.content` 首次出现；已处理 |
| 2026-09-14 | `ImageRef` 被引用但未定义 | `spec/IR.md §3` 核心类型完整性 | 补充 `Url` / `Base64` 两态；已处理 |
| 2026-09-14 | `Opaque` 派生 `Debug` 会暴露 bytes，违背 INV-4 | `DESIGN.md` INV-4 优先 | 改为手写 `Debug`，仅输出 `kind + len`；已处理 |
| 2026-09-14 | IR-INV-USAGE-2 正向公式漏减 cache_creation，与反向公式和包含关系注释冲突 | IR.md §8 的可逆 round-trip 要求 | 正向改为同时减去 cached / cache_creation；已处理 |
| 2026-09-14 | 流式 `PartStart` 缺少 tool id/name/kind，Messages/Chat 工具调用跨协议时无法无损转换 | `spec/STREAMING.md` 事件集需满足 INV-3、IR-INV-TOOL-1 | 在 `PartStart` 增加可选 `ToolStart` 元数据；已处理 |
| 2026-09-14 | Chat 流式 `n>1` 需要 choice 维度，但 `Event` 集只有 block index | `spec/chat.md CHAT-TRAP-4`、`IR-INV-N-1` | 当前显式 `Unsupported`，不静默退化；choice 元数据扩展留待后续任务 |
