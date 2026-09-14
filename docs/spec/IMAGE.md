# spec/IMAGE.md — 图片输入规范

> 状态：半稳定　|　最后更新：2026-09-14
> 读者：实现 M5 图片输入的 codec / converter 作者。
> 实现前必读：`IR.md`、本文件，以及对应 `chat.md` / `messages.md` / `responses.md`。
> 本文只定义**图片输入的表示转换**。资源下载、上传与托管属于 host，不属于核心。

---

## 1. 范围

**M5 支持**

- OpenAI Chat Completions、Anthropic Messages、OpenAI Responses 的图片输入。
- 远程 URL 与 inline Base64 两种来源。
- OpenAI `data:` URI 与 Anthropic 分离 `media_type` / `data` 字段之间的规范化。
- 图片转换失败时的显式 `Report`。

**M5 不支持**

- 图片输出、图片生成、音频、视频、PDF 或其他文件附件。
- Files API、`file_id`、上传、下载、转码、缩放、OCR。
- 远程 URL 的可达性、鉴权、签名有效期、重定向、SSRF 防护或缓存。
- 对 URL 指向资源的主动读取。

**核心不变量**

- **IMG-INV-1**：核心绝不发 HTTP 请求，绝不读取远程 URL。
- **IMG-INV-2**：核心绝不上传、托管或生成可公开访问的图片 URL。
- **IMG-INV-3**：无法表达的字段或表示必须进 `Report`，不得静默丢弃。
- **IMG-INV-4**：远程 URL 字符串 byte-equal 透传，不重新拼装查询参数。
- **IMG-INV-5**：Base64 与 `data:` URI 的转换必须保持 `media_type` 和 payload byte-equal。

## 2. Canonical IR

```rust
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
```

- `ImageSource::RemoteUrl`：目标服务端需要自行获取的非 `data:` URL。M5 只规范化 `http://` 与 `https://`。
- `ImageSource::Base64`：`data` 只保存 Base64 payload，**不含** `data:<media_type>;base64,` 前缀；`media_type` 独立保存。
- `ImageRef.detail`：保留 OpenAI `detail` 原始字符串。Anthropic 无等价字段，跨协议时记 `Report`。

`data:` URI 在协议边界解析为 `ImageSource::Base64`；反向编码为 OpenAI 时合成 `data:<media_type>;base64,<data>`。

## 3. 规范化规则

### 3.1 Remote URL

- 仅接受 `http://` 或 `https://`。
- URL 字符串按原字节复制，不做百分号解码、query 重排、末尾斜杠增删或 host 小写化。
- 其他 scheme 暂不进入 `RemoteUrl`；按 `Unsupported` 处理并记 `Report`。
- 是否让目标上游访问该 URL，由 host 和上游网络策略决定。

### 3.2 data URI

OpenAI 风格的图片 data URI 必须满足：

```text
data:<media_type>;base64,<payload>
```

- `<media_type>` 非空，例如 `image/png`、`image/jpeg`、`image/webp`。
- 必须有 `;base64` 标记。
- `<payload>` 进入 `ImageSource::Base64.data`，前缀与 media type 不进入 payload。
- 非 Base64 data URI、缺 media type、缺 `;base64` 或编码无法识别时返回明确错误 / `Report`。

### 3.3 Anthropic 分离字段

```json
{
  "source": {
    "type": "base64",
    "media_type": "image/png",
    "data": "BASE64_PAYLOAD"
  }
}
```

- `media_type` 必须存在。
- `data` 必须是纯 Base64 payload，不能包含 data URI 前缀。
- `source.type:"url"` 映射为 `ImageSource::RemoteUrl`。

## 4. Wire 形态

| 协议 | 表示 | Wire 字段 | 规范化 |
|---|---|---|---|
| Chat Completions | 远程 URL | `content[].image_url.url` | `RemoteUrl` |
| Chat Completions | inline Base64 | `content[].image_url.url = "data:...;base64,..."` | `Base64` |
| Responses | 远程 URL | `input_image.image_url` | `RemoteUrl` |
| Responses | inline Base64 | `input_image.image_url = "data:...;base64,..."` | `Base64` |
| Messages | 远程 URL | `source:{type:"url", url}` | `RemoteUrl` |
| Messages | inline Base64 | `source:{type:"base64", media_type, data}` | `Base64` |

OpenAI 的 `image_url` / `image_url.url` 是**多态图片来源字段**，不是“仅远程 URL”。实现不得只看字段名。

## 5. 转换矩阵

在三个协议都支持 URL 与 Base64 的能力下，正常转换保持来源表示：

| 来源 IR | Chat 目标 | Responses 目标 | Messages 目标 |
|---|---|---|---|
| `RemoteUrl` | `image_url.url = url` | `input_image.image_url = url` | `source.type:"url"` |
| `Base64` | `image_url.url = data URI` | `input_image.image_url = data URI` | `source.type:"base64"` |

只读、无状态场景不需要资源转换：

- 源为 URL、目标支持 URL：透传 URL。
- 源为 Base64、目标支持 inline Base64：透传 payload 与 media type。
- OpenAI 与 Anthropic 之间需要做 data URI 的拆分或拼装，但不访问图片资源。

以下转换不在核心范围：

- `RemoteUrl -> Base64`：需要下载，交给 host resolver。
- `Base64 -> RemoteUrl`：需要上传或托管，交给 host uploader。
- `file_id -> ImageSource`：需要 Files API 或 provider 存储，M5 明确不支持。

## 6. 与 host 的边界

核心只能接收已经解析好的来源：

```text
host / resolver
  -> 得到 RemoteUrl 或 Base64
  -> Converter
  -> 目标协议 wire 字段
```

如果 host 需要把 URL 下载为 Base64，或把 Base64 上传后生成 URL，应在调用 `Converter` 之前完成。核心不提供网络实现，也不依赖网络客户端。

## 7. Report 与错误

以下情况必须显式处理：

| 情况 | 处理 |
|---|---|
| 非 http/https URL scheme | `Unsupported` + `Report` |
| `data:` URI 格式非法 | `InvalidInput` |
| `file_id` / Files API 引用 | `Unsupported` + `Report` |
| `detail` 目标协议无法表达 | 省略并记 `Report` |
| 图片 part 目标协议完全无法表达 | 省略并记 `Report`；`Strict` 下 `Fatal` |

## 8. 契约测试清单

- [ ] Remote URL 在 Chat / Messages / Responses 三协议内 byte-equal 往返。
- [ ] Base64 payload 与 media type 在三协议内往返一致。
- [ ] OpenAI data URI ↔ IR Base64 拆分 / 拼装一致。
- [ ] Messages `source.type:"url"` ↔ OpenAI `image_url.url`。
- [ ] Messages `source.type:"base64"` ↔ OpenAI data URI。
- [ ] 非法 data URI、缺 media type、缺 `;base64` 明确失败。
- [ ] `file_id` 明确 `Unsupported`。
- [ ] `detail` 跨协议时记 `Report`，同协议内保留。
- [ ] 9 个协议方向的图片输入矩阵不访问网络。
- [ ] live vision 测试默认 ignored，缺模型能力时 skip 且不计为通过。

## 9. 相关文档

- IR 类型与不变量：`IR.md §3、§9`
- 协议字段映射：`chat.md` / `messages.md` / `responses.md`
- 测试策略：`../TESTING.md`
- 决策缘由：`../decisions/0006-image-input-no-transport.md`
