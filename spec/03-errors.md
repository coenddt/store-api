# 03 — 错误 → HTTP 状态码映射

遵循项目 `no-error-masking` 准绳：**允许被拦截，禁止静默失守**。store 抛出的每一个错误都必须变成带语义的 HTTP 错误响应；错误信息取自错误对象上契约语义为「错误」的字段（`message` / `code`），禁止用其他字段顶包，禁止吞掉后返回空成功。

## 错误响应统一形态

```json
HTTP 4xx/5xx
{ "error": { "code": "<store 错误名或适配层错误名>", "message": "<原始错误信息，原样透传>" } }
```

- `code`：优先取 store 错误对象的 `code`/`name` 字段；无则用适配层错误码（下表）。
- `message`：错误对象的 `message` 原样透传，不改写、不翻译、不圆场。
- 成功响应（2xx）**不得**包含 `error` 字段，也不得包含任何错误语义文案。

## 状态码映射表（双端一致）

| 场景 | 状态码 | code |
|---|---|---|
| 资源未注册（路由表生成后仍命中未注册名——理论上不发生，防御性保留） | 404 | `RESOURCE_NOT_FOUND` |
| 单条查询 `queryOne` 返回空 | 404 | `NOT_FOUND` |
| 下载路由 `GET /{resource}/{id}/file`：记录不存在 / 文件字段为空 | 404 | `NOT_FOUND` |
| 下载路由 `GET /{resource}/{id}/file`：`fileResolver` 读取时资源无任何可读副本（core 稳定前缀 `ERR_RESOURCE_NOT_FOUND:`） | 404 | `NOT_FOUND` |
| 请求体缺失 / 非 JSON 对象 | 400 | `INVALID_BODY` |
| `p.*` 值非法（JSON 解析失败） | 400 | `INVALID_PARAM` |
| GQL 解析失败（store 抛出，core 稳定前缀 `ERR_GQL_PARSE:`） | 400 | `GQL_PARSE` |

> 判定依据（v1 修订）：core 的 GQL 解析错误（tokenizer/parser/参数表达拒绝）统一携带
> `ERR_GQL_PARSE:` 稳定前缀——与 `ERR_PERM_PREFIX` 同构的类型级契约，四端按**前缀**判定
>（构造后剥离，不对中文文案做脆弱匹配）。响应 `code` 统一取 `GQL_PARSE`，`message` 为
> core 原文（剥离前缀后的完整文案）。
| store `PermissionError`（RBAC 拒绝） | 403 | store 原始 code/name |
| store `NoContextError`（`requireContext` 开启且未注入 ctx；core 稳定前缀 `ERR_NO_CONTEXT:`，machine code `no_context`） | 403 | store 原始 code/name |
| 其余 store 抛出的错误（数据库、连接、方言等） | 500 | store 原始 code/name |
| 上传路由 `POST /{resource}/{id}/file`：请求体为空 | 400 | `EMPTY_BODY` |
| 上传路由 `POST /{resource}/{id}/file`：字节体超过 `uploadLimit` | 413 | `UPLOAD_TOO_LARGE` |
| 上传路由 `POST /{resource}/{id}/file`：未注入 `uploadResolver` | 501 | `UPLOAD_NOT_CONFIGURED` |
| 下载路由 `GET /{resource}/{id}/file`：未注入 `fileResolver` | 501 | `FILE_NOT_CONFIGURED` |

## 判定顺序（双端实现必须一致）

1. 未注入 resolver（上传缺 `uploadResolver` / 下载缺 `fileResolver`）→ 501 `UPLOAD_NOT_CONFIGURED` / `FILE_NOT_CONFIGURED`
2. 适配层自身守卫（body / param 合法性、空体、超限）→ 400 `INVALID_BODY` / `INVALID_PARAM` / `EMPTY_BODY`，413 `UPLOAD_TOO_LARGE`
3. 权限类错误（按错误类型 / core machine code 判定，**不**按 message 字符串匹配）→ 403：
   - `store.PermissionError`（RBAC 拒绝）
   - `store.NoContextError`（`requireContext` 开启且 ctx 缺失；core machine code `no_context`，与 `PermissionError` 同属权限类，非 500）
4. core 稳定前缀类错误（构造后剥离，不对中文文案做脆弱匹配）：
   - GQL 解析失败（`ERR_GQL_PARSE:`）→ 400 `GQL_PARSE`
   - 资源无任何可读副本（`ERR_RESOURCE_NOT_FOUND:`）→ 404 `NOT_FOUND`
   两条的 `message` 均为**剥离前缀后的 core 原文**。
5. `queryOne` 空结果（含 file 子路由「记录不存在 / 字段为空」）→ 404
6. 其余一律 500 透传

> file 子路由的判定：未注入 resolver ⇒ 501（第 1 层，先于一切）；
> `queryOne` 返回空 ⇒ 记录不存在 ⇒ 404 `NOT_FOUND`；
> 记录存在但字段为空（`null` / 缺失 / `field` 点路径任一段缺失）⇒ 文件不存在 ⇒ 404 `NOT_FOUND`。二者文案区分、状态码同。
> 上传另有两项：请求体为空 ⇒ 400 `EMPTY_BODY`；字节体超过 `uploadLimit` ⇒ 413 `UPLOAD_TOO_LARGE`。

> 资源读取错误的两分（core `resourceOpen` 语义）：按 `resourceId` 查 `ResourceLocation`——
> **零副本行** ⇒ 抛 `ERR_RESOURCE_NOT_FOUND:` ⇒ 适配器 404 `NOT_FOUND`（资源确实不存在）；
> **有副本行但 provider `get` 失败** ⇒ core 逐个降级、全部失败后**原样重抛**最后一个 provider 错误
> ⇒ 适配器 500 透传（属 IO/降级故障，不是「不存在」）。二者语义不同，禁混同。

## 禁止事项（对照 no-error-masking）

- ❌ `catch` 后返回 `{"data": null}` 或空数组把错误洗成成功
- ❌ 500 响应里用 `|| '未知错误'` 之类兜底伪造 message——取不到 `message` 就显式置 null 并保留 code
- ❌ 把 store 的中性返回（如 `update` 影响 0 行）当错误；`update`/`remove` 影响 0 行返回 `200` 与 store 原始结果，是否视为业务异常由调用方判断
