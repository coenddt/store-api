# Changelog — store-api

本文件记录 store-api（node / py / rust / go 四实现共用 spec）的契约变更。
格式遵循 Keep a Changelog；版本号由发布流程统一 bump。

## [Unreleased]

### Breaking

- **`ERR_NOT_FOUND:`（写探针未命中）HTTP 状态 500 → 404**：与「资源不存在统一 404」跨协议口径对齐，
  `code` 保持 core machine code `not_found`、`message` 原样透传（与 gRPC `details` / MCP `message` 同口径）。
  依赖旧 500 判断的调用方需改为按 404 判定。见 `spec/03-errors.md` 判定顺序第 4 层。

### Added

- **go 端 `/file` 子路由落地**（此前缺失，conformance 4 个 file 用例报 `404 page not found`）：
  - `GET /{prefix}/{resource}/{id}/file`：queryOne → 字段空 404 → resolver → 200 字节流 +
    `content-type`（缺省 `application/octet-stream`）/ `content-disposition`（缺省 `attachment; filename="file"`，CR/LF 清洗）；
  - `POST /{prefix}/{resource}/{id}/file`：未注入 resolver 501 → 空体 400 `EMPTY_BODY` / 超限 413 `UPLOAD_TOO_LARGE`
    → `?field=` 可写白名单 400 `INVALID_PARAM` → queryOne 空 404 → uploadResolver → `update` 回写 200；
  - `Options` 新增 `FileField` / `FileResolver` / `UploadResolver` / `UploadLimit`；
  - resolver 的 `ERR_RESOURCE_NOT_FOUND:` 统一映射 404 `NOT_FOUND`（message 剥前缀）。
- go conformance runner 支持用例级 `opts`（mock resolver）与 `raw:<bytes>` 字节体约定，5 份用例（含 4 份 file）全绿。

### Fixed

- **py 上传体改为边读边限流**：`_read_upload_body` 改用 `request.stream()` 分块累计，超限立即中断并释放已缓冲字节，
  不再先 `await request.body()` 把超大请求整读入内存；`uploadLimit`（默认 32MB）从「事后比较」变为真正的内存防线。
- **node 非法 / 空 JSON 请求体 → 400 `INVALID_BODY`**：映射 Fastify
  `FST_ERR_CTP_INVALID_JSON_BODY` / `FST_ERR_CTP_EMPTY_JSON_BODY`，不再落 500。
- **下载 `Content-Disposition` 清洗 `fileName` 的 CR/LF**：py 实测可把 `\r\n` 原样写入响应头（可拆出额外头），
  node 实测会 500 `ERR_INVALID_CHAR`；两实现统一把 CR/LF 替换为空格，文件名其余内容保持不变。
- **上传 `?field=` 增加 schema 可写白名单**：仅允许 schema 字段树中的标量 `string` / `file`（`object` 下钻为 `a.b`），
  排除 `_id`、`$` 前缀与原型污染段；未命中 → 400 `INVALID_PARAM` 并给出明确 message。下载（读）不受限。

### Tests

- **go `TestRESTContextAndPermission` 的 `guest 写应 403, got 201` 定性为「用例漏配」，非实现缺陷**：
  该用例的 schema 未配置 `write` 白名单，core `permission::evaluate` 对未配置 `role_list` 的默认姿态是
  `UnconfiguredPolicy::Open`（放行）；spec/04 亦明确适配层不加第二层权限判断。故接口层 201 符合契约。
  修正方式与 rust 端同族用例一致：显式 `SetDenyWriteRoles([]string{"guest"})`（`rust_store_e2e.rs` 对应
  `set_deny_write_roles(vec!["guest"])`），并补 admin 写 201 对照，证明 403 来自拒写清单而非整体拒写。

