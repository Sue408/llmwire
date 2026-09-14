# ADR-0001：接缝切在字节侧

> 状态：接受　|　日期：2026-09-14

## 背景

SDK 需要与 host 约定一个交互边界。可选：让 host 传入已解析的协议对象（typed），或让 host 传入原始字节（`&[u8]`）。

## 决策

SDK 的公开接缝为**字节级**：入站 `&[u8]`，出站写入 `&mut Vec<u8>`。SSE/NDJSON 分帧由 SDK 负责。

## 理由

1. **object-safe 白送**：一旦关联类型不穿过门面，每个 codec 天然 object-safe，`Converter` 只持 `Box<dyn ProtocolCodec>` 两个实例——N 套 codec，不是 N²。
2. **host 不必重写最硬的坑**：SSE 跨 chunk 截断、注释行、`[DONE]`、`delta:null` 等分帧问题，host 若自己实现就是重复劳动且易错。
3. **"轻量固件"定位**：host 只需读字节、喂字节、写字节，集成面最小。

## 后果

- 出站 JSON 由 SDK 拼装 → **必须**保证 key 顺序稳定，否则破坏 prefix cache（`DESIGN.md` TRAP-2）。
- 内部分层仍需 typed + 纯函数，门面只是薄壳，否则 property test 写不了。
- `out` 采用 **append 语义**（SDK 不清空），调用方复用 buffer 前自行 `clear()`。

## 备选与否决

- **typed 接缝**：类型更安全，但 host 需自行分帧、且运行时路由要写 N² 胶水。
- **两者都暴露**：超出"轻量"目标，MVP 不做。
