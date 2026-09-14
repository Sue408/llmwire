# ADR 0006：图片输入只做表示转换，不碰传输

> 状态：接受　|　日期：2026-09-14
> 影响范围：M5、IR、Chat / Messages / Responses codec、Host 边界

## 背景

三家协议都支持图片输入，但来源字段形态不同：

- Chat Completions：`image_url.url` 可放 http(s) URL 或 `data:` URI。
- Responses：`input_image.image_url` 可放 http(s) URL 或 `data:` URI。
- Messages：`source.type` 明确区分 `url` 与 `base64`。

URL 与 Base64 看起来可以互转，但语义不同：

- URL 指向外部资源，需要服务端自行获取。
- Base64 把资源字节直接放进请求体。

将远程 URL 转成 Base64 需要 HTTP 下载；将 Base64 转成真正远程 URL 需要上传或托管。两者都会把传输层职责带入核心。

## 决策

`llmwire` 核心只做图片输入的**表示转换**：

- 保留 `RemoteUrl` 与 `Base64` 两种来源。
- 在 OpenAI data URI 与 Anthropic 分离字段之间做无网络规范化。
- URL 字节保真透传。
- 不下载、不上传、不转码、不校验远程可达性。
- 核心无法完成的跨模式转换显式 `Unsupported` / `Report`。

图片下载、上传、托管、鉴权和 URL 生命周期由 host 或后续 companion crate 负责。

## 结果

正面：

- 保持 INV-5：核心不引入 HTTP / async runtime。
- 保持 INV-6：核心不引入跨请求资源缓存或存储。
- 三家协议的常规双支持路径可以无损转换。
- strict / converted 模式下的失败语义清晰。

代价：

- 从远程 URL 转为 inline Base64 不能开箱即用。
- 从 inline Base64 转为远程 URL 不能开箱即用。
- host 若需要这两种能力，必须显式接入 resolver / uploader。

## 备选方案

### 在核心内置 URL 下载

拒绝。核心会引入 HTTP、超时、重试、SSRF、鉴权和网络错误语义，违反 INV-5。

### 在核心内置对象存储上传

拒绝。核心会持有凭据、跨请求资源生命周期和上传状态，违反 INV-6，也超出转换核定位。

### 只支持 URL，不支持 Base64

拒绝。真实客户端常直接发送本地文件 Base64，且 Messages 明确支持该路径。

### 只支持 Base64，不支持 URL

拒绝。URL 是三家协议共同支持的轻量路径，强制下载会引入不必要网络和失败面。
