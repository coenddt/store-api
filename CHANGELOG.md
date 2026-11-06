# Changelog — store-api

本文件记录 store-api（node / py / rust / go 四实现共用 spec）的契约变更。
格式遵循 Keep a Changelog；版本号由发布流程统一 bump。

## [Unreleased]

### Breaking

- **`ERR_NOT_FOUND:`（写探针未命中）HTTP 状态 500 → 404**：与「资源不存在统一 404」跨协议口径对齐，
  `code` 保持 core machine code `not_found`、`message` 原样透传（与 gRPC `details` / MCP `message` 同口径）。
  依赖旧 500 判断的调用方需改为按 404 判定。见 `spec/03-errors.md` 判定顺序第 4 层。

### Fixed

- **py 上传体改为边读边限流**：`_read_upload_body` 改用 `request.stream()` 分块累计，超限立即中断并释放已缓冲字节，
  不再先 `await request.body()` 把超大请求整读入内存；`uploadLimit`（默认 32MB）从「事后比较」变为真正的内存防线。
- **node 非法 / 空 JSON 请求体 → 400 `INVALID_BODY`**：映射 Fastify
  `FST_ERR_CTP_INVALID_JSON_BODY` / `FST_ERR_CTP_EMPTY_JSON_BODY`，不再落 500。
- **上传 `?field=` 增加 schema 可写白名单**：仅允许 schema 字段树中的标量 `string` / `file`（`object` 下钻为 `a.b`），
  排除 `_id`、`$` 前缀与原型污染段；未命中 → 400 `INVALID_PARAM` 并给出明确 message。下载（读）不受限。
