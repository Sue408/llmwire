# llmwire

`llmwire` 是一个**双向、默认无状态、可嵌入任意 host 的 LLM wire protocol 转换核**。

Host 把原始请求或响应字节交给请求级 `Converter`，它负责：

- SSE 分帧与跨 chunk 拼接
- 源协议解析
- 协议无关 IR 转换
- 目标协议序列化
- 降级与不支持的显式 `Report`

`llmwire` 不发送 HTTP、不管理连接、不保存跨请求状态，也不绑定 async runtime。

> 状态：`M0` 至 `M5` 已完成，`P0` 至 `P2` 验证增强已完成。当前版本为 `0.1.0`。
> 已验证范围与尚未覆盖的真实 API 场景见 `docs/TESTING.md`。

## 支持范围

| 能力 | 状态 |
|---|---|
| OpenAI Chat Completions | 请求、响应、流式 |
| Anthropic Messages | 请求、响应、流式 |
| OpenAI Responses | 无状态请求、响应、流式 |
| 三协议双向文本转换 | 支持 |
| response `id` / `model` | 透传 target 上报值；`model` 缺失时统一空字符串 |
| tool call / tool result | 支持，ID 字节保真 |
| thinking / signature / encrypted reasoning | 支持或显式降级，不伪造 |
| usage / cache usage | 保留未知与零的差异 |
| 图片输入 URL / Base64 | 支持，三协议间映射 |
| 图片 `file_id` | 明确不支持 |
| 有状态 Responses | 明确不支持 |

核心依赖约束：除 `serde`、`serde_json`、`thiserror`、`bitflags` 外，不引入运行时依赖。

## 安装

发布前使用本地路径或 Git 引用：

```toml
[dependencies]
llmwire = { path = "../llmwire" }
```

要求 Rust `edition 2021`。

## 非流式用法

`converter(src, dst, caps)` 中的方向约定：

- `src`：客户端正在使用的协议
- `dst`：上游后端正在使用的协议

```rust
use llmwire::{converter, resolve, ProtocolId};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut converter = converter(
        ProtocolId::Chat,
        ProtocolId::Messages,
        resolve(ProtocolId::Chat, ProtocolId::Messages, "claude-model"),
    )?;

    let client_request = br#"{
        "model": "claude-model",
        "messages": [{"role": "user", "content": "hello"}],
        "max_completion_tokens": 32
    }"#;

    let mut upstream_request = Vec::new();
    converter.request(client_request, &mut upstream_request)?;
    // Host 将 upstream_request 发给 Messages 后端。

    let backend_response = br#"{
        "id": "msg_1",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": "world"}],
        "model": "claude-model",
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 3, "output_tokens": 2}
    }"#;

    let mut client_response = Vec::new();
    converter.response(backend_response, &mut client_response)?;

    let report = converter.take_report();
    println!(
        "report: {} unmapped, {} warnings",
        report.unmapped.len(),
        report.warnings.len()
    );

    println!("{}", String::from_utf8_lossy(&client_response));
    Ok(())
}
```

完整示例：

```powershell
cargo run --example nonstream
```

## 流式用法

流式模式下，host 必须：

1. 先调用 `request`，由 SDK 记录 `stream=true` 与请求上下文。
2. 每收到一个上游 chunk 就调用 `feed`，并立即转发追加出的字节。
3. 上游结束后调用 `finish`，转发尾帧并检查 `Termination`。
4. 在合适时机调用 `take_report`。

```rust
use llmwire::{converter, resolve, ProtocolId, Termination};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut converter = converter(
        ProtocolId::Chat,
        ProtocolId::Messages,
        resolve(ProtocolId::Chat, ProtocolId::Messages, "claude-model"),
    )?;

    let client_request = br#"{
        "model": "claude-model",
        "stream": true,
        "messages": [{"role": "user", "content": "hello"}],
        "max_completion_tokens": 32
    }"#;
    let mut upstream_request = Vec::new();
    converter.request(client_request, &mut upstream_request)?;

    let upstream_chunk = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-model\"}}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );

    let mut client_stream = Vec::new();
    converter.feed(upstream_chunk.as_bytes(), &mut client_stream)?;

    let mut tail = Vec::new();
    let termination = converter.finish(&mut tail)?;
    client_stream.extend_from_slice(&tail);
    assert_eq!(termination, Termination::Explicit);

    println!("{}", String::from_utf8_lossy(&client_stream));
    Ok(())
}
```

完整示例：

```powershell
cargo run --example streaming
```

## 能力与策略

简单的协议对配置可以使用：

```rust
let caps = llmwire::resolve(
    llmwire::ProtocolId::Chat,
    llmwire::ProtocolId::Messages,
    "claude-model",
);
```

需要按模型覆盖策略时使用 `StaticHost`：

```rust
use llmwire::{
    Host, ModelProfile, ProtocolId, StaticHost, ThinkingPolicy, UnknownModelPolicy,
};

let host = StaticHost::with_unknown_policy(UnknownModelPolicy::Reject)
    .with_profile(
        "claude-model",
        ModelProfile {
            thinking: Some(ThinkingPolicy::Passthrough),
            max_output_tokens: Some(8192),
            ..ModelProfile::default()
        },
    );

let caps = host.capabilities(
    ProtocolId::Chat,
    ProtocolId::Messages,
    "claude-model",
)?;
# Ok::<(), llmwire::Error>(())
```

能力策略描述的是“这条路由允许采用什么转换方式”，不是协议本身的能力表。

## 错误与 Report

`llmwire` 使用双信道表达问题：

- `Result::Err`：适用于尚未向客户端发出成功响应前的失败。
- `Converter::feed` 中的流内失败：写入目标协议 error 事件，并记录到 `Report`；不会混用为 `Ok(Termination::Explicit)`。
- `Report`：记录字段降级、不可表达内容与策略阻断。

典型处理方式：

```rust
let report = converter.take_report();
for item in report.unmapped {
    eprintln!(
        "unmapped field={} reason={:?} severity={:?}",
        item.field, item.reason, item.severity
    );
}
```

`take_report` 会清空当前已累积的报告。

## Host 的职责

`llmwire` 只处理 wire 字节，host 仍需负责：

- HTTP 请求、认证、超时与重试
- 路由选择与上游连接
- 从请求体中提取模型名并选择后端
- 把 `out` 字节写入客户端或上游
- SSE 出口关闭代理缓冲、gzip 与网关聚合
- 客户端断连、网络异常与上游取消

## 已知限制

- 不伪造 `signature`、`redacted_thinking` 或 thinking 内容。
- 不重算或改写客户端传入的 `cache_control` 断点。
- 不支持有状态 Responses 请求，例如 `previous_response_id` 与 `store=true`。
- Chat 流式 `n > 1` 当前显式返回 `Unsupported`。
- 图片仅支持远程 `http(s)` URL 与 Base64 payload；`file_id` 不支持。
- 核心不下载图片、不上传图片、不生成托管 URL。
- Live 矩阵当前以本地网关为主，官方端点矩阵尚未执行。
- Messages thinking/signature live 测试仍为 skipped，不能视为该路径已获真实 API 验证。

## 文档

- `docs/DESIGN.md`：架构、公开 API 与不可协商约束
- `docs/spec/IR.md`：协议无关 IR
- `docs/spec/STREAMING.md`：流式事件与 FSM
- `docs/spec/chat.md`、`docs/spec/messages.md`、`docs/spec/responses.md`：协议映射
- `docs/spec/IMAGE.md`：图片输入规则
- `docs/TESTING.md`：测试策略、live API 与当前验证矩阵
- `docs/PLAN.md`：里程碑与完成定义
- `docs/README.md`：完整文档地图

## 许可证

MIT。详见 `LICENSE`。

## 开发

```powershell
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked -p llmwire
pwsh -NoProfile -File scripts/check-core-deps.ps1
pwsh -NoProfile -File scripts/check-live-gate.ps1
```

Live API 测试默认 ignored，只有显式运行 `--ignored` 时才访问网络。配置方式见 `docs/TESTING.md §7`。
