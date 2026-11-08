# Changelog — store-api

本文件记录 store-api（node / py / rust / go 四实现共用 spec）的契约变更。
格式遵循 Keep a Changelog；版本号由发布流程统一 bump。

## [Unreleased]

> 目标版本 **1.0.0**（本轮为破坏性变更集中释放）。本文件不改动 `package.json` / `pyproject.toml` 版本，
> 由发布流程统一 bump。

### Breaking

#### 1. 非法 / 空 JSON 请求体：`500` → `400 INVALID_BODY`

- **改了什么**：node 端把 Fastify 的 `FST_ERR_CTP_INVALID_JSON_BODY` / `FST_ERR_CTP_EMPTY_JSON_BODY`
  映射为 `400` + `code: "INVALID_BODY"`，不再作为未捕获异常落 `500`。
- **下游要动哪里**：按 `5xx` 做重试或告警的调用方需改为把 `400 INVALID_BODY` 视为**客户端请求错误**
  （不重试）；请求体拼装方需在发送前校验 JSON 合法性与非空。

#### 2. 写探针未命中：`500` → `404`，`code` 取 core machine code `not_found`

- **改了什么**：`ERR_NOT_FOUND:`（写探针未命中）HTTP 状态由 `500` 改为 `404`，与「资源不存在统一 404」
  的跨协议口径对齐；`code` 保持 core machine code `not_found`，`message` 原样透传（与 gRPC `details` /
  MCP `message` 同口径）。见 `spec/03-errors.md` 判定顺序第 4 层。
- **下游要动哪里**：依赖旧 `500` 判断「后端异常」的调用方需改为按 `404` / `not_found` 判定
  「目标不存在」；原本把 404 当「接口错误」上报的监控规则需同步调整。
  go 端此前缺失 `/file` 子路由的行为一并收敛，见下 `Added`。

#### 3. 上传 `?field=` 增加可写白名单（node / py）

- **改了什么**：`POST /{prefix}/{resource}/{id}/file?field=<path>` 仅允许 schema 字段树中的标量
  `string` / `file`（`object` 下钻写作 `a.b`）；`_id`、`$` 前缀、`__proto__` / `constructor` /
  `prototype` 等原型污染段一律拒绝，未命中返回 `400 INVALID_PARAM` 并给出明确 message。下载（读）不受限。
- **下游要动哪里**：往非字符串字段或非 schema 声明字段挂文件的调用方会由「成功」变为 `400`；
  需要扩展可写点时必须先在 schema 中声明字段。

#### 4. 上传 `?field=` 同规则落到 rust 实现

- **改了什么**：rust 端此前 `?field=` 无白名单，可写 `_id`（覆盖主键）与 `$` 前缀段。现与 node / py
  同规则：先硬禁止 `_id` / `__proto__` / `constructor` / `prototype` / `$` 前缀段，再由
  `Store::schema_definition` 驱动完整白名单（未声明字段、非 string 字段、string 下钻不存在的子段
  一律 `400 INVALID_PARAM`；修前分别是 `500 STORE_ERROR` 泄漏 SQL 与 `200` 放行）。
- **下游要动哪里**：与第 3 条相同；另需注意修前「静默写成功」的错误请求现在会显式失败（这是期望行为）。

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
- **下载 `Content-Disposition` 清洗 `fileName` 的 CR/LF**：py 实测可把 `\r\n` 原样写入响应头（可拆出额外头），
  node 实测会 500 `ERR_INVALID_CHAR`；两实现统一把 CR/LF 替换为空格，文件名其余内容保持不变。

### Tests

- **go `TestRESTContextAndPermission` 的 `guest 写应 403, got 201` 定性为「用例漏配」，非实现缺陷**：
  该用例的 schema 未配置 `write` 白名单，core `permission::evaluate` 对未配置 `role_list` 的默认姿态是
  `UnconfiguredPolicy::Open`（放行）；spec/04 亦明确适配层不加第二层权限判断。故接口层 201 符合契约。
  修正方式与 rust 端同族用例一致：显式 `SetDenyWriteRoles([]string{"guest"})`（`rust_store_e2e.rs` 对应
  `set_deny_write_roles(vec!["guest"])`），并补 admin 写 201 对照，证明 403 来自拒写清单而非整体拒写。
