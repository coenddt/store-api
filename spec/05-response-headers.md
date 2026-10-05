# 05 — 响应头注记

## `x-cache`

- **语义**：标识本次 HTTP 响应是否经过缓存。**注记位**，不是缓存实现。值域恒为 `HIT` / `MISS` / `BYPASS`。
  - `BYPASS`：响应未经过缓存——**本版本唯一实际取值**（不实现缓存）。
  - `HIT` / `MISS`：预留给未来缓存实现，由宿主 provider 上报。
- **取值来源**：宿主 `store.cacheStatus()`（node）/ `store.cache_status()`（py）；store-api **不发明缓存语义**，只透传注记。
  - 宿主未提供该方法 → 恒 `BYPASS`。
  - 宿主返回非法值 / 抛错 → 恒 `BYPASS`；宿主侧对非法/异常走反馈通道留痕（禁静默）。
- **覆盖范围**：**所有** HTTP 响应（含 2xx 与 4xx/5xx）均携带该头。
- **禁止**：注记必须为真——不得在无缓存实现时输出 `HIT` / `MISS`。

## 与 00-overview「不做缓存」的关系

`00-overview.md` 的「不做缓存」指**不实现缓存**；本头仅为宿主缓存状态的**透传注记**，不改变
store-api「HTTP ↔ store 调用翻译」的定位（零语义发明）。

## 下载响应头（`GET /{resource}/{id}/file`）

- **`content-type`**：下载响应的媒体类型。取值来源优先级：`fileResolver` 返回的 `contentType` → 未注入 resolver 时兜底 `text/plain; charset=utf-8`。缺省（resolver 未给）回落 `application/octet-stream`。
- **`content-disposition`**：恒为 `attachment; filename="<fileName>"`；`fileName` 来源：`fileResolver` 返回的 `fileName` → 未注入 resolver 时兜底 `<resource>-<id>`；缺省回落 `file`。
- **语义边界**：皮不推断 MIME（避免发明语义），`contentType` / `fileName` 由接入方 `fileResolver` 决定（生产应注入 resolver 走 `store.resource.open`）。下载响应体为**字节流**，非 JSON 壳。
