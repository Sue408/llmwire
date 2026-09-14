# ADR-0003：Report 升为一等 API

> 状态：接受　|　日期：2026-09-14

## 背景

跨协议转换必然有损：目标协议无法表达源协议的某些字段/能力。常见做法是静默丢弃（LiteLLM `drop_params`）或直接报错。

## 决策

`Report` 是**一等公开 API**，通过 `Converter::take_report()` 暴露。每条记录带 `severity`（`Silent`/`Degraded`/`Fatal`）、`reason`（`UnsupportedByTarget`/`NotRepresentable`/`PolicyBlocked`）与**来源路径**（字段名 / 事件序号）。策略上：`Strict` 下 `Fatal` 即 `Err`；`Converted` 下降级但**可见**。

## 理由

1. "静默丢弃是双向协议的第一大反模式"：上层"选最优/重试"逻辑会静默退化，bug 极难归因。
2. 项目硬约束要求 SDK **诚实**——不仅要内部不丢，还要让 host/GUI 看得见折损了什么。
3. `n>1` 静默变单候选、`top_k` 凭空消失这类问题，只有报告可见才能防。

## 后果

- 每个 codec 的降级分支都必须落到 `Report`，成为验收项（`PLAN.md` M4-2）。
- **诚实 ≠ 泄露**：`Opaque`（加密/签名块）在报告中**只记 `kind + len`**（INV-4），不记内容。
- `DESIGN.md` 的 `Mode`/`Strategy` 需区分：是否 passthrough 由路由决定，`Report` 只描述转换质量。

## 备选与否决

- **返回包装类型携带 warnings**：使每次调用都变啰嗦，否决；对象状态白送了这个便利。
- **仅日志**：GUI 不可见，违背"诚实可见"，否决。
