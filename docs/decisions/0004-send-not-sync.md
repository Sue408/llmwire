# ADR-0004：`Converter: Send` 而非 `Sync`

> 状态：接受　|　日期：2026-09-14

## 背景

host 通常运行在异步多线程运行时（Tauri v2 / tokio）。请求级 `Converter` 会活过 `await` 点。Rust 中这涉及 `Send` / `Sync` 自动 trait 的选择。

## 决策

`Converter` 约束为 **`Send`，不约束 `Sync`**；所有方法取 `&mut self`。

## 理由

1. 对象要跨 `await` 点存活并被 `tokio::spawn` 时，**必须 `Send`**，否则不编译。
2. 方法全为 `&mut self`，`&Converter` 无法调用任何方法，"共享引用并发"本就不成立；要求 `Sync` 没有实际收益。
3. 强制 `Sync` 会**限制内部实现**：例如用 `RefCell`/`Cell` 累积 `Report` 是 `Send` 但 `!Sync`；要求 `Sync` 会堵死这类轻量选项。

## 后果

- host 若真要跨线程共享，须 `Arc<Mutex<Converter>>`（请求级对象通常不需要）。
- 内部可使用 `Send` 但 `!Sync` 的容器，保持实现自由。

## 备选与否决

- **同时要求 `Sync`**：牺牲实现自由度换一个用不上的能力，否决。
- **不加 `Send`**：异步 host 无法 spawn，选择面被无谓收窄，否决。
