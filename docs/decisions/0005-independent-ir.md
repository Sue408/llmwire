# ADR-0005：独立 IR，不以 OpenAI 形状为长期语义

> 状态：接受　|　日期：2026-09-14

## 背景

三种架构范式可选：① 以某协议（如 OpenAI Chat）为 canonical；② 选一个表达力强的协议当枢纽（如 cc-router 用 Anthropic）；③ 设计独立 IR。

## 决策

采用**独立 IR**（`spec/IR.md`），形状贴近 OpenAI 作为最低公共分母，但通过 `Opaque` 逃生舱搭载任意非 OpenAI 语义。IR 是唯一跨 codec 的交换格式，`codec/*` 之间禁止互相依赖。

## 理由

1. 直接复用 OpenAI 形状作长期记忆格式，会丢失 Anthropic thinking、server tool、Responses reasoning 等语义（LiteLLM 教训）。
2. 选某协议当枢纽会被其表达能力绑架（cc-router 的短板）；一旦上游表达枢纽表达不了的东西，转换即卡死。
3. 独立 IR 让"新增协议 = 新增一套 codec"，而非 N² 映射或重复写 canonical 桥。

## 后果

- IR 必须"优雅地容纳不同点"：细粒度 block、`Opaque` 逃生舱、role 塌缩为 User/Assistant、usage 包含关系固化。
- IR 维护成本：新增协议若引入新语义，可能需要扩展 IR；这是可接受的、受控的演进。

## 备选与否决

- **OpenAI canonical**：丢失非 OpenAI 语义，`Opaque` 也救不了结构性缺失。
- **枢纽协议**：把上限锁死在单个协议的表达能力上。
- **显式 N×N codec 矩阵（WaLiAPI）**：协议增多即组合爆炸，维护成本高。
