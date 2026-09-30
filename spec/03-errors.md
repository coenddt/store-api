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
| 请求体缺失 / 非 JSON 对象 | 400 | `INVALID_BODY` |
| `p.*` 值非法（JSON 解析失败） | 400 | `INVALID_PARAM` |
| GQL 解析失败（store 抛出） | 400 | store 原始 code/name |
| store `PermissionError`（RBAC 拒绝） | 403 | store 原始 code/name |
| 其余 store 抛出的错误（数据库、连接、方言等） | 500 | store 原始 code/name |

## 判定顺序（双端实现必须一致）

1. 适配层自身守卫（body / param 合法性）→ 400
2. `store.PermissionError`（按错误类型判定，**不**按 message 字符串匹配）→ 403
3. `queryOne` 空结果 → 404
4. 其余一律 500 透传

## 禁止事项（对照 no-error-masking）

- ❌ `catch` 后返回 `{"data": null}` 或空数组把错误洗成成功
- ❌ 500 响应里用 `|| '未知错误'` 之类兜底伪造 message——取不到 `message` 就显式置 null 并保留 code
- ❌ 把 store 的中性返回（如 `update` 影响 0 行）当错误；`update`/`remove` 影响 0 行返回 `200` 与 store 原始结果，是否视为业务异常由调用方判断
