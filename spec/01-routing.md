# 01 — schema → REST 路由映射

前提：schema 已通过 `store.register(defn)` 注册。适配器启动时（register 钩子 / create_app 时）读取 `store.list()`，为每个已注册资源生成下列路由。运行期新注册的 schema 是否生效**不做承诺**（两端框架的路由注册时机不同），接入方应在注册完所有 schema 后再挂载适配器。

## 路由表

| 方法 | 路径 | store 调用 | 成功响应 |
|---|---|---|---|
| GET | `/{resource}` | `query(gql, params)` | `200 {"data": [...]}` |
| GET | `/{resource}/{id}` | `queryOne(gql, params)` | `200 {"data": {...}}` |
| POST | `/{resource}` | `insert(schemaName, body)` | `201 {"data": <插入结果>}` |
| PATCH | `/{resource}/{id}` | `update(schemaName, cond, body)` | `200 {"data": <更新结果>}` |
| DELETE | `/{resource}/{id}` | `remove(schemaName, cond)` | `200 {"data": <删除结果>}` |
| GET | `/{resource}/{id}/file` | `queryOne(gql, params)` | `200` 字节流 + `content-type` / `content-disposition`（**非 JSON 壳**） |

## 明确不提供（零语义发明）

- **PUT**：store 的 `update` 是部分更新语义；PUT 的全量替换语义在 store 中不存在，不发明。
- **子资源路由** `/{resource}/{id}/{relation}`：v1 不提供。关系数据通过列表查询的 GQL 关系展开获取（见 02-params）。子资源路由的权限/分页语义复杂，留待有真实需求再进 spec。**唯一例外**：`/{resource}/{id}/file` 文件下载（见路由表）——字节来源由接入方 `fileResolver` 注入，皮只搬运，见 00-overview「例外登记」。
- **路由名复数化**：路由段 = schema 资源名**原样**，不做单复数变换（双端各自实现复数规则必然漂移）。
- **批量端点** `POST /{resource}/batch` 等：store 有 `insertMany`/`updateMany`，但其条件/批量语义与 REST 惯例的映射未定，v1 不进 spec，待需求落地时补。

## 主键约定

单条路由 `{id}` 绑定到主键字段等值条件。主键字段名由适配器选项 `idField` 指定，**双端默认均为 `'_id'`**，双端必须一致。条件形如 `{ [idField]: id }`（Node）/ `{ idField: id }`（Python），作为 `@c0` 参数绑进适配器生成的 GQL 串：

```
GET /user/abc  ⇒  store.queryOne('user($condition: @c0) { <全字段投影> }', { c0: { _id: 'abc' } })
```

**投影必须显式生成**（取 schema 注册定义的 `fields` 键列表，`GET /{resource}/{id}` 与 `POST` 返回文档同理）：GQL 省略投影段的契约语义是「只返回 `_id`」（实证：nodejs-store 参考实现 + rust-store 宿主一致行为，core `compute_keep` 对空 fields 仅保留 `_id`）——省略投影会让单条路由返回只剩 `_id` 的数据。schema 未声明 `fields` 时退化 为无投影（此时 data 仅含 `_id`，属上游 schema 定义不完整的显式可见后果，不静默兜底）。

## 归档表过滤（双端一致）

`store.list()` 返回的名称**包含自动派生的归档表**（注册 schema 时自动派生 `<Name>Deleted` 镜像，见 nodejs-store/src/schema.js 注释）。路由生成时按以下规则过滤，规则双端逐字一致：

> 名称以 `Deleted` 结尾、且去掉该后缀后的名称也在 `list()` 结果中 ⇒ 视为归档表，不生成路由。

适配器同时提供 `resources` 选项（显式资源名数组）覆盖默认行为，供接入方精确控制暴露面。

## 请求体

- POST / PATCH 请求体必须是 JSON 对象，原样传给 store；适配器不校验业务字段（校验是 store schema 的职责）。
- 请求体缺失或非 JSON 对象 ⇒ `400`（见 03-errors）。
