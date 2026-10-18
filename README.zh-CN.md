# store-api

为 [common-store](../) 数据层家族（nodejs-store / py-store / rust-store / go-store 宿主）提供
**schema 驱动的 RESTful API 自动生成**。

> REST 只是 GQL 之上的 HTTP 皮 —— 适配层**零语义发明**；每条路由、参数、错误与权限判定都映射到
> store 已有的 schema / GQL / RBAC 语义上。凡 store 没有的语义，本层一律不提供。

> English docs: [README.md](./README.md)

- Node 端：Fastify 适配器（`store-api-node`，npm）
- Python 端：FastAPI 适配器（`store-api-py`，PyPI）
- Go 端：`net/http` 适配器（`store-api-go`，`go/` 子目录，适配 go-store 宿主；conformance runner 见
  `go/conformance_test.go`；**未发布**）
- Rust 端：axum 适配器（`store-api-rs`，`rust/` 子目录，消费 `Store` trait；rust-store 宿主直接实现该
  trait 接入；**未发布**）
- 共享：`spec/`（路由 / 参数 / 错误 / 上下文 / 响应头映射的唯一事实源）+ `conformance/`（多端一致性用例）

## 运行时矩阵

| 运行时 | 框架 | 包 | 分发 | 资源上传/下载路由 | `x-cache` 响应头 | 状态 |
|---|---|---|---|---|---|---|
| Node | Fastify | `store-api-node` | npm | ✅ | ✅ | 已发布 |
| Python | FastAPI | `store-api-py` | PyPI | ✅ | ✅ | 已发布 |
| Go | `net/http`（标准库，零框架依赖） | `store-api-go`（module `github.com/coenddt/store-api-go`） | 仓内 `go/` | — | — | 未发布 |
| Rust | axum | `store-api-rs` | 仓内 `rust/` | — | — | 未发布 |

> CRUD 路由表（列表 / 单条 / 新建 / 部分更新 / 删除）在**四端**均已实现。资源字节子路由
> （`GET|POST /{resource}/{id}/file`）与 `x-cache` 响应头目前仅在 **node** 与 **py** 落地；go 与 rust
> 适配器目前是纯 CRUD。详见[边界](#边界--明确不提供)。

### 近期能力

- 资源上传 / 下载端点：`GET /{resource}/{id}/file`（`fileResolver`）与
  `POST /{resource}/{id}/file`（`uploadResolver`）；未注入对应 resolver → 501。
- 错误映射：资源无任何可读副本（core 稳定前缀 `ERR_RESOURCE_NOT_FOUND:`）→ HTTP 404 `NOT_FOUND`；
  上下文缺失（NoContext，`ERR_NO_CONTEXT:` / machine code `no_context`）→ HTTP 403。
- `x-cache` 响应头：宿主缓存状态（`store.cacheStatus()` / `cache_status()`）的透传注记，取值
  `HIT` / `MISS` / `BYPASS`；未实现缓存时恒 `BYPASS`，**所有**响应（含 4xx/5xx）均携带 —— 见
  `spec/05-response-headers.md`。

## 目录

- [这是什么](#这是什么)
- [何时使用](#何时使用)
- [何时不用](#何时不用)
- [安装](#安装)
- [快速上手](#快速上手)
  - [Node（Fastify）](#nodefastify)
  - [Python（FastAPI）](#pythonfastapi)
  - [Go（net/http）](#gonethttp)
  - [Rust（axum）](#rustaxum)
- [路由与 URL 形态](#路由与-url-形态)
- [查询参数参考](#查询参数参考)
- [上下文与错误映射](#上下文与错误映射)
- [响应头](#响应头)
- [场景事例](#场景事例)
- [定制 / 注入点](#定制--注入点)
- [边界 / 明确不提供](#边界--明确不提供)
- [FAQ](#faq)
- [开发](#开发)
- [相关项目](#相关项目)
- [License](#license)

## 这是什么

`store-api` 是 common-store 数据层之上的 **HTTP 适配层**。它不是新的数据层，也不做认证、
限流、缓存**实现**。它唯一的职责是翻译 HTTP ⇄ store 调用。

```
HTTP 客户端
   │  RESTful (JSON)
store-api   （本仓库：fastify / fastapi / net/http / axum 适配器）
   │  store 原生 API：list / query / queryOne / insert / update / remove + setContext
数据层      （nodejs-store / py-store / go-store / rust-store 宿主）
   │  GQL → 命令规划
rust-store core  （解析 / 权限 / SQL 方言）
   │
数据库      （MongoDB / MySQL / PostgreSQL / SQLite）
```

### 三条铁律

1. **零语义发明。** 路由、参数、错误、权限语义必须能一一对应到 store 已有语义；store 没有的语义
   本层不提供。
2. **双端 parity 先行。** 任何变更先改 `spec/`，再改各端实现，再补 `conformance/` 用例。规范、代码、
   用例三者不一致即为缺陷。
3. **禁掩盖错误**（遵循项目 `no-error-masking` 准绳）。store 抛出的每个错误都必须映射为带语义的
   HTTP 错误响应：禁止静默吞掉、禁止回退默认值、禁止把中性结果当错误。成功响应中不得出现错误文案。

### 如何复用 store 的语义

- **schema → 路由。** 适配器读取已注册的 schema 列表（`store.list()` / `store.ListSchemas()` /
  `Store::list`），为每个资源生成一族路由。路由段 = schema 资源名**原样**，不做单复数变换。
- **查询 → GQL。** 列表查询（`GET /{resource}`）就是 GQL 的**透传与参数绑定**，不另造第二套筛选 /
  排序 / 分页语法。
- **权限 → RBAC。** 适配层不做认证，只经 `setContext` / `set_context` 为每请求注入上下文，由 store
  自身的 RBAC 管道判定。权限拒绝即 store 错误 → HTTP 403。

## 何时使用

- 你已在用某个 `*-store` 宿主，想**免手写**每资源的 controller / 筛选 / 分页 / 权限校验就得到 REST 面。
- 你希望 REST 面在各运行时**完全一致** —— 同一套 GQL、同一套错误码、同一套状态码。
- 你希望筛选 / 排序 / 聚合用**完整 GQL 语言**，而不是自造的 query-string 迷你语法。
- 你希望 RBAC 由 store 负责，HTTP 层只把请求翻译成上下文。
- 你希望**响亮而诚实的失败**：未实现的能力（如缺 file resolver）返回 `501`，而非静默兜底。

## 何时不用

- 你需要本层**实现**认证、会话、限流或缓存 —— store-api 有意一概不做，只输出 `x-cache` **注记**，
  没有缓存实现。
- 你需要 `PUT` 全量替换、自动复数化路由、关系子资源（`/{resource}/{id}/{relation}`）或批量端点 ——
  这些有意不提供（见[边界](#边界--明确不提供)）。
- 你不在 `*-store` 宿主上 —— 没有可适配的对象。
- 你需要 GraphQL / gRPC / MCP 面 —— 请用同族的其它协议皮（`store-graphql` / `store-grpc` / `store-mcp`）。

## 安装

### Node（npm）

```bash
npm install store-api-node
# peer 依赖（安装期可选；除非显式传 errors.PermissionError，否则运行期必需）：
npm install nodejs-store
```

### Python（PyPI）

```bash
pip install store-api-py
# 可选开发依赖（pytest / httpx / uvicorn）：
pip install "store-api-py[dev]"
```

### Go

Go 适配器**未发布**，从仓库内消费（module 为 `github.com/coenddt/store-api-go`，其 `go.mod` 把
`github.com/coenddt/go-store` `replace` 到本机 `../../go-store`）：

```bash
go get github.com/coenddt/store-api-go
go get github.com/coenddt/go-store
```

### Rust

Rust 适配器**未发布**，以 path 依赖从仓库内消费。默认 feature `rust-store` 开启到 rust-store 宿主的
开箱桥接：

```toml
[dependencies]
store-api-rs = { path = "../store-api/rust" }
# 纯库形态（仅 mock smoke）：
# store-api-rs = { path = "../store-api/rust", default-features = false }
```

## 快速上手

### Node（Fastify）

```js
const { init, store } = require('nodejs-store');
const fastify = require('fastify');
const { storeApiPlugin } = require('store-api-node');

await init({ default: 'mongodb://localhost:27017/app' });
store.register({
  name: 'user',
  fields: { name: { type: 'string' }, age: { type: 'number' } },
});

const app = fastify();
// 先注册完所有 schema，再挂载适配器。
await app.register(storeApiPlugin, { store, prefix: '/api' });
await app.listen({ port: 3000 });

// GET /api/user?q=($condition: @c0) { name, age }&p.c0={"age":{"$gte":18}}
```

### Python（FastAPI）

```python
from py_store import init, store
from store_api_py import create_app

await init({'default': 'mongodb://localhost:27017/app'})
store.register({
    'name': 'user',
    'fields': {'name': {'type': 'string'}, 'age': {'type': 'number'}},
})

app = create_app(store, prefix='/api')
# uvicorn main:app --port 3000
```

### Go（net/http）

```go
package main

import (
	"net/http"

	gostore "github.com/coenddt/go-store"
	storeapi "github.com/coenddt/store-api-go"
)

func main() {
	st, _ := gostore.Open("sqlite://app.db")
	// 先 st.Register(...) + st.EnsureTables(...) —— 注册完所有 schema 再挂路由。

	mux := http.NewServeMux()
	_ = storeapi.New(mux, st, storeapi.Options{
		Prefix:  "/api",
		IDField: "_id",
		ContextProvider: func(r *http.Request) (*gostore.Context, error) {
			token := r.Header.Get("X-User")
			if token == "" {
				return nil, nil // 显式空上下文
			}
			return &gostore.Context{UserID: token, Roles: []string{"admin"}}, nil
		},
	})
	http.ListenAndServe(":3000", mux)
}
```

可直接运行的全链路示例（SQLite 文件库 + 真实 HTTP + Go 计算列）见
[`go/demo/main.go`](./go/demo/main.go)：

```bash
cd store-api/go && go run ./demo
# → REST 服务已就绪: http://127.0.0.1:8091/api/order
```

### Rust（axum）

```rust
use std::sync::Arc;
use store_api_rs::{create_router, Options, Store};

// 为你的宿主实现 `Store`（rust-store 桥接 `RustStoreAdapter` 已代劳）。
let store: Arc<dyn Store> = Arc::new(my_store);

let router = create_router(store, Options {
    resources: None,           // 缺省：store.list() 减去归档表
    id_field: "_id".into(),
    context_provider: None,    // Option<ContextProvider>，见「上下文与错误映射」
}).await;

let app = axum::Router::new().nest("/api", router);
// axum::serve(listener, app).await?;
```

适配器暴露通配路由族（`/{resource}`），前缀由接入方经 `.nest("/api", router)` 自行控制 —— 与
Fastify 的 `prefix` 机制对齐。

## 路由与 URL 形态

前提：schema 已通过 store 注册（`store.register(defn)` / `store.Register(...)`）。适配器在挂载时读取
资源清单并生成下列路由。挂载**之后**新注册的 schema 是否生效**不做承诺**（各端框架的路由注册时机
不同）—— 请先注册完所有 schema 再挂载。

| 方法 | 路径 | store 调用 | 成功响应 |
|---|---|---|---|
| GET | `/{prefix}/{resource}` | `query(gql, params)` | `200 {"data": [...]}` |
| GET | `/{prefix}/{resource}/{id}` | `queryOne(gql, params)` | `200 {"data": {...}}` |
| POST | `/{prefix}/{resource}` | `insert(schemaName, body)` | `201 {"data": <插入结果>}` |
| PATCH | `/{prefix}/{resource}/{id}` | `update(schemaName, cond, body)` | `200 {"data": <更新结果>}` |
| DELETE | `/{prefix}/{resource}/{id}` | `remove(schemaName, cond)` | `200 {"data": <删除结果>}` |
| GET | `/{prefix}/{resource}/{id}/file` | `queryOne(gql, params)` | `200` 字节流 + `content-type` / `content-disposition`（**非 JSON 壳**） |
| POST | `/{prefix}/{resource}/{id}/file` | `queryOne` → `uploadResolver` → `update` | `200 {"data": <回写后的记录>}` |

URL 各段落如何落到 GQL：

| URL 段落 | 含义 | GQL 映射 |
|---|---|---|
| `{resource}` | schema 资源名，**原样** | GQL 资源头（`user(...) { ... }`） |
| `{id}` | 主键等值条件 | 条件 `{ [idField]: id }` 绑为 `@c0`；GQL `user($condition: @c0) { ... }` |
| `{id}/file` | 资源字节子路由 | 见[场景事例](#场景事例) —— 由 `fileResolver` / `uploadResolver` 承载 |
| `?q=...` | 条件 / 排序 / 分页 / 关系展开 / 聚合 | 原样拼在资源头之后 |
| `?p.<name>=...` | GQL 命名参数 | 绑到 `params` 中的 `@<name>` |
| `?field=...` | 字节子路由的字段点路径 | 定位记录上承载资源引用的字段 |

### 主键约定

单条路由把 `{id}` 绑到主键等值条件。主键字段名由适配器选项 `idField` 指定；**各端默认均为 `'_id'`**
且必须一致。

```
GET /api/user/abc
  ⇒ store.queryOne('user($condition: @c0) { name, age }', { c0: { _id: 'abc' } })
```

### 投影必须显式生成

GQL 的契约为：**省略投影段 = 只返回 `_id`**（各端共享的 core 契约；core `compute_keep` 对空 fields 仅
保留 `_id`），**不是**「返回全字段」。因此：

- 单条路由与 `POST` 响应由适配器按 schema 的 `fields` 键列表生成**显式投影**。
- 无 `q` 的列表路由同样生成显式全字段投影。
- 带 `q` 的列表路由，投影**完全由 `q` 决定**，适配器不追加任何东西。
- schema 未声明 `fields` 时投影退化为空 → `data` 仅含 `_id`。这是上游 schema 定义不完整的显式可见
  后果，不静默兜底。

### 归档表过滤（各端逐字一致）

`store.list()` 返回的名称包含自动派生的归档表（注册 `Xxx` 时自动派生 `XxxDeleted` 镜像）。规则在
各端逐字一致：

> 名称以 `Deleted` 结尾、**且**去掉该后缀后的名称也在列表中 ⇒ 视为归档表，**不生成路由**。

各端适配器同时提供 `resources` 选项（显式资源名数组）覆盖默认行为，供接入方精确控制暴露面。

## 查询参数参考

列表查询的全部能力来自 GQL 本身。适配层只做**透传与参数绑定**，不发明第二套筛选 / 排序 / 分页语法。

| 查询参数 | 语义 | 映射 |
|---|---|---|
| `q` | GQL 查询体（**资源头之后**的部分 —— 条件及之后） | 拼接进 GQL 串 |
| `p.<name>` | 绑定 GQL 命名参数 `@<name>` | 收集进 `params` 对象 |
| `field` | 字节子路由（`GET|POST /{resource}/{id}/file`）的字段点路径 | 定位记录上承载资源引用的字段；缺省回落适配器选项 `fileField` |

### `q` 的形态

`q` 是去掉资源头的 GQL 片段，适配器负责拼上资源头：

```
GET /api/user?q=($condition: @c0, $limit: @l) { name, age }&p.l=10
```

拼接为 `user($condition: @c0, $limit: @l) { name, age }`，等价于调用
`store.query('user($condition: @c0, $limit: @l) { name, age }', { c0: {...}, l: 10 })`。

> **GQL 参数值一律是 `@name` 命名引用**（core GQL 语法；py-store / nodejs-store 同构），
> **不接受字面量** —— 数值、布尔、对象等都经 `p.<name>` 传入后在 params 侧转型。写成 `$limit: 10`
> 字面量形态会被 core 以 `ERR_GQL_PARSE:` 拒绝。

排序、分页、关系展开、聚合等全部走 GQL 自有语法（`$orderby` / `$limit` / `$offset` / 关系子查询 /
`$group` 等）。本规范不重复定义，以 store 的 GQL 文档为准。

### `p.<name>` 的值类型转换

所有 `p.*` 值以**字符串**到达，按下列规则转换后绑定（首个命中即止，无命中则保持字符串原样），各端
逐字一致：

1. `true` / `false` → 布尔
2. `null` → null
3. 完全符合整数正则 `^-?\d+$` → 整数
4. 完全符合浮点正则 `^-?\d+\.\d+$` → 浮点数
5. 其余 → 字符串原样

需要复杂结构（对象 / 数组）时，值为合法 JSON 则解析为对象；解析失败 ⇒ `400` 并原样报出解析错误
（禁静默当字符串兜底）。

同名参数重复出现时（`p.tag=a&p.tag=b`），绑定值为**数组**（每元素各自转换）；单次出现时绑定标量。

### `field` 的形态与边界

- `field` 为**点路径**（如 `images.full` / `file`），逐段取对象属性；缺省（不传）时用适配器选项
  `fileField`。
- 仅支持点路径寻址（`a.b`）；**不支持**数组下标 / 通配 / 表达式。
- 路径任一段缺失 ⇒ 视同「文件字段为空」⇒ `404 NOT_FOUND`（与 `fileField` 缺失同判定）。
- 该参数**仅**对 file 子路由生效；列表 / 单条路由忽略之（不报错）。

### 无 `q` 时

适配器**生成全字段投影**（取 schema `fields` 键列表）—— 否则（无投影）契约语义是只返回 `_id` 列
表。是否被权限裁剪由 store RBAC 决定，适配层不追加任何默认条件。

### 错误

- `q` 拼接后经 store 解析失败（GQL 语法错误）⇒ `400`，响应体携带 store 的原始错误信息，原样透传。
- `p.*` JSON 解析失败 ⇒ `400`，携带解析错误原文。

## 上下文与错误映射

### 上下文注入（RBAC）

store 的 RBAC 以请求上下文为基准（`store.setContext(ctx)` / `store.set_context(ctx)`）。store-api
自身**不做认证**，只提供把 HTTP 请求翻译成上下文的钩子。

| 选项 | 类型 | 语义 |
|---|---|---|
| `contextProvider` / `context_provider` | `(req) => ctx \| Promise<ctx>` | 每请求调用；返回值作为本请求的 store 上下文。 |

行为：

1. 每个请求进入 handler 前，若配置了 `contextProvider`，调用它并经 `store.setContext(ctx)` 注入。
2. provider 抛出的错误分类：`PermissionError` ⇒ `403`（与业务 handler 内权限拒绝同一语义）；其余
   错误视为认证 / 上下文构造失败 ⇒ `401`，code `CONTEXT_ERROR`，message 原样透传。这是**唯一**以
   `401` 呈现的场景；store RBAC 拒绝（上下文已设置但权限不足）是 `403`。
3. 适配器不解析任何认证头（`Authorization` / `Cookie` / `API-Key` 一概不碰）—— 身份来源完全由
   `contextProvider` 决定，接入方自行接 JWT / Session 等。
4. 有状态持有的端（node / py / go）在 provider 返回 `null`/`undefined` 时必须**显式注入空上下文**
   （`setContext(null)` 为清除语义），防上一请求上下文残留导致身份跨请求泄漏。Rust 适配器为
   请求级注入，ctx 随 `request.extensions` 传递，无全局状态。
5. store 的 `requireContext` 档位（`store.setRequireContext(true)`）由接入方自行设置；开启后未注入
   上下文的请求会被 store 以 `NoContextError`（core machine code `no_context`，与 `PermissionError`
   同属权限类）拒绝 → `403`，适配层不加第二层判断。
6. **fail-secure 装配守卫。** 宿主已开启上下文强制（`requireContext` 为真）却未配 `contextProvider`
   时，装配期即失败并抛 `ERR_SECURE_CONFIG`，不把配置错误推迟成运行期事故。

### 错误响应统一形态

```json
HTTP 4xx/5xx
{ "error": { "code": "<store 错误名或适配层错误名>", "message": "<原始错误信息，原样透传>" } }
```

- `code`：优先取 store 错误对象的 `code`/`name`；无则用适配层错误码。
- `message`：错误对象的 `message` 原样透传 —— 不改写、不翻译、不圆场。
- 成功响应（2xx）**不得**包含 `error` 字段，也不得包含任何错误语义文案。

### 状态码映射表（双端一致）

| 场景 | 状态码 | code |
|---|---|---|
| 资源未注册（防御性；路由生成后理论上不发生） | 404 | `RESOURCE_NOT_FOUND` |
| 单条 `queryOne` 返回空 | 404 | `NOT_FOUND` |
| 下载 `GET /{resource}/{id}/file`：记录不存在 / 文件字段为空 | 404 | `NOT_FOUND` |
| 下载 `GET /{resource}/{id}/file`：`fileResolver` 读取时资源无任何可读副本（core 稳定前缀 `ERR_RESOURCE_NOT_FOUND:`） | 404 | `NOT_FOUND` |
| 请求体缺失 / 非 JSON 对象 | 400 | `INVALID_BODY` |
| `p.*` 值非法（JSON 解析失败） | 400 | `INVALID_PARAM` |
| GQL 解析失败（store 抛出；core 稳定前缀 `ERR_GQL_PARSE:`） | 400 | `GQL_PARSE` |
| store `PermissionError`（RBAC 拒绝） | 403 | store 原始 code/name |
| store `NoContextError`（`requireContext` 开启且未注入 ctx；core 前缀 `ERR_NO_CONTEXT:`，machine code `no_context`） | 403 | store 原始 code/name |
| 其余 store 错误（数据库、连接、方言等） | 500 | store 原始 code/name |
| 上传 `POST /{resource}/{id}/file`：请求体为空 | 400 | `EMPTY_BODY` |
| 上传 `POST /{resource}/{id}/file`：字节体超过 `uploadLimit` | 413 | `UPLOAD_TOO_LARGE` |
| 上传 `POST /{resource}/{id}/file`：未注入 `uploadResolver` | 501 | `UPLOAD_NOT_CONFIGURED` |
| 下载 `GET /{resource}/{id}/file`：未注入 `fileResolver` | 501 | `FILE_NOT_CONFIGURED` |
| `contextProvider` 抛出非权限错误 | 401 | `CONTEXT_ERROR` |

### 判定顺序（各端实现必须一致）

1. **未注入 resolver**（上传缺 `uploadResolver` / 下载缺 `fileResolver`）→ 501
   `UPLOAD_NOT_CONFIGURED` / `FILE_NOT_CONFIGURED`。
2. **适配层自身守卫**（body / param 合法性、空体、超限）→ 400
   `INVALID_BODY` / `INVALID_PARAM` / `EMPTY_BODY`、413 `UPLOAD_TOO_LARGE`。
3. **权限类错误**（按错误类型 / core machine code 判定，**不**按 message 字符串匹配）→ 403：
   `store.PermissionError` 与 `store.NoContextError`。
4. **core 稳定前缀类错误**（构造后剥离前缀，不对中文文案做脆弱匹配）：`ERR_GQL_PARSE:` → 400
   `GQL_PARSE`；`ERR_RESOURCE_NOT_FOUND:` → 404 `NOT_FOUND`。两条的 `message` 均为**剥离前缀后的
   core 原文**。
5. **`queryOne` 空结果**（含 file 子路由「记录不存在 / 字段为空」）→ 404。
6. **其余一律 500 透传。**

> file 子路由的判定：未注入 resolver ⇒ 501（第 1 层，先于一切）；`queryOne` 返回空 ⇒ 记录不存在 ⇒
> 404 `NOT_FOUND`；记录存在但字段为空（`null` / 缺失 / `field` 点路径任一段缺失）⇒ 文件不存在 ⇒
> 404 `NOT_FOUND`（文案区分、状态码同）。上传另有两项：请求体为空 ⇒ 400 `EMPTY_BODY`；字节体超过
> `uploadLimit` ⇒ 413 `UPLOAD_TOO_LARGE`。
>
> 资源读取错误的两分（core `resourceOpen` 语义）：**零副本行** ⇒ 抛 `ERR_RESOURCE_NOT_FOUND:` ⇒
> 404 `NOT_FOUND`（资源确实不存在）；**有副本行但 provider `get` 全部失败** ⇒ core 原样重抛最后一个
> provider 错误 ⇒ 500 透传（属 IO / 降级故障，不是「不存在」）。二者语义不同，禁混同。

### 禁止事项（对照 `no-error-masking`）

- ❌ `catch` 后返回 `{"data": null}` 或空数组把错误洗成成功。
- ❌ 500 响应里用 `|| '未知错误'` 之类兜底伪造 message —— 取不到就显式置 `null` 并保留 code。
- ❌ 把 store 的中性返回（如 `update` / `remove` 影响 0 行）当错误；此时返回 `200` 与 store 原始结
  果，是否视为业务异常由调用方判断。

## 响应头

### `x-cache`

- **语义**：标识本次 HTTP 响应是否经过缓存。是**注记位**，不是缓存实现。值域恒为 `HIT` / `MISS` /
  `BYPASS`。
  - `BYPASS`：响应未经过缓存 —— **本版本唯一实际取值**（不实现缓存）。
  - `HIT` / `MISS`：预留给未来缓存实现，由宿主 provider 上报。
- **取值来源**：宿主 `store.cacheStatus()`（node）/ `store.cache_status()`（py）。store-api **不发明
  缓存语义**，只透传注记。
  - 宿主未提供该方法 → 恒 `BYPASS`。
  - 宿主返回非法值 / 抛错 → 恒 `BYPASS`（宿主侧对非法 / 异常走反馈通道留痕，禁静默）。
- **覆盖范围**：**所有** HTTP 响应（含 2xx 与 4xx/5xx）均携带该头。
- **禁止**：注记必须为真 —— 不得在无缓存实现时输出 `HIT` / `MISS`。

> 该头在 node 与 py 落地；go 与 rust 适配器不输出 `x-cache`。

### 下载响应头（`GET /{resource}/{id}/file`）

- **`content-type`**：下载响应的媒体类型，由 `fileResolver` 返回的 `contentType` 决定；缺省回落
  `application/octet-stream`。（未注入 `fileResolver` ⇒ 501 `FILE_NOT_CONFIGURED`，不产生响应头。）
- **`content-disposition`**：恒为 `attachment; filename="<fileName>"`；`fileName` 来源 `fileResolver`
  返回的 `fileName`，缺省回落 `file`。
- **语义边界**：皮不推断 MIME（避免发明语义），`contentType` / `fileName` 由接入方 `fileResolver`
  决定。下载响应体为**字节流**，非 JSON 壳。

## 场景事例

每个场景格式固定：**业务背景 → 请求 → 响应 → 优势所在**。示例中的资源名、字段名、方法名均取自本仓库
源码与 `conformance/cases/*.json`。

### 1. 内部管理后台 CRUD

**业务背景。** 内部工具需要对 `user` 资源做增删改查。每资源手写 controller 是易漂移的样板活。

**请求。**

```bash
# 新建
curl -s -X POST http://localhost:3000/api/user \
  -H 'content-type: application/json' \
  -d '{"name":"alice","age":30}'

# 读单条
curl -s http://localhost:3000/api/user/abc

# 部分更新
curl -s -X PATCH http://localhost:3000/api/user/abc \
  -H 'content-type: application/json' \
  -d '{"age":31}'

# 删除
curl -s -X DELETE http://localhost:3000/api/user/abc
```

**响应**（新建返回 201，取自 `conformance/cases/users-crud.json`）：

```json
{ "data": { "_id": "abc", "name": "alice", "age": 30 } }
```

**优势所在。** 每资源自动生成五条路由，无需 controller 代码。`PATCH` 是部分更新，直接映射 store 的
`update`；适配器不校验业务字段（那是 schema 的职责）。特别地**没有 `PUT`** —— store 无全量替换语义，
API 也不提供。

### 2. 用完整 GQL 按字段 / 关系筛选的列表

**业务背景。** 看板要列出 `age >= 18` 的用户，且你不愿自造 query-string 迷你语法。

**请求。**

```bash
curl -s -G 'http://localhost:3000/api/user' \
  --data-urlencode 'q=($condition: @c0) { name, age }' \
  --data-urlencode 'p.c0={"age":{"$gte":18}}'
```

**响应。**

```json
{ "data": [ { "_id": "abc", "name": "alice", "age": 30 } ] }
```

**优势所在。** 适配器拼上资源头并绑定 `@c0`；筛选就是 **GQL**，GQL 支持的每个操作符都零改动可用。
`q` 完全决定投影，适配器不追加。手写 REST 需要自造解析器；此处语法就是 store 的。

### 3. 偏移分页（`$limit` / `$offset`）

**业务背景。** 列表页每次展示 10 行。

**请求。**

```bash
curl -s -G 'http://localhost:3000/api/user' \
  --data-urlencode 'q=($limit: @l, $offset: @o) { name, age }' \
  --data-urlencode 'p.l=10' \
  --data-urlencode 'p.o=20'
```

**响应。**

```json
{ "data": [ /* 最多 10 行 */ ] }
```

**优势所在。** 分页走 GQL，经 `p.*` 绑定。注意**参数值是 `@name` 引用** —— `p.l=10` 把字符串 `"10"`
转型为整数 `10` 并绑定 `@l`；写成 `$limit: 10` 字面量会被 core 以 `ERR_GQL_PARSE:` 拒绝（→ 400
`GQL_PARSE`）。store-api **从不追加 `total` 字段** —— 那属语义发明；计数是另一次 GQL 聚合查询
（具体聚合语法：待核实 / 超出本规范范围）。

### 4. 软删除与审计（归档镜像）

**业务背景。** 删除不能凭空消失；store 自动保留 `XxxDeleted` 归档镜像。

**请求。**

```bash
curl -s -X DELETE http://localhost:3000/api/user/abc
```

**响应。**

```json
{ "data": { /* store 的原始 remove 结果 */ } }
```

**优势所在。** `DELETE` 映射 store 的 `remove`，归档编排在 store 内（事务化）完成 —— HTTP 层不做
特殊事。自动派生的 `userDeleted` 归档表**不生成路由**（归档表过滤），审计副本不会被误暴露为可写
REST 资源。若删除不存在的 id，`remove` 影响 0 行仍返回 `200` 与原始结果 —— API 不在此发明 404
（是否业务异常由调用方判断）。

### 5. 资源下载与上传（`fileResolver` / `uploadResolver`）

**业务背景。** `user` 记录上挂着一个文件引用。你想要 HTTP 下载与上传，又不让 store-api 做存储 IO、
拼签名 URL 或判权限。

**下载请求。**

```bash
curl -s -D - http://localhost:3000/api/user/abc/file -o out.bin
```

**下载响应头。**

```
content-type: application/octet-stream
content-disposition: attachment; filename="file"
x-cache: BYPASS
```

（响应体为字节流，非 JSON）

**上传请求。**

```bash
curl -s -X POST http://localhost:3000/api/user/abc/file \
  -H 'content-type: application/octet-stream' \
  --data-binary @payload.bin
```

**上传响应。**

```json
{ "data": { "_id": "abc", "name": "alice", "file": "sha1abc" } }
```

**优势所在。** 下载 = `queryOne`（定位记录与字段）→ `fileResolver`（产出字节）；上传 = `queryOne` →
`uploadResolver`（落字节，返回 `ref`）→ `update`（回写 `ref`）。因为回写走 **`update`**，RBAC 的**写**
权限自动生效 —— 皮自身从不判权限。字节由 `application/octet-stream` 缓冲 parser 承接，上限由
`uploadLimit` 控制（默认 32MB）。

用 `field` 点路径命中嵌套字段：

```bash
curl -s -X POST 'http://localhost:3000/api/user/abc/file?field=images.full' \
  -H 'content-type: application/octet-stream' \
  --data-binary @cover.jpg
```

### 6. 未注入 resolver 的 501 语义

**业务背景。** 你把字节子路由先部署上去，但字节存储尚未接好。

**请求。**

```bash
curl -s -o - -w '\n%{http_code}\n' http://localhost:3000/api/user/abc/file
curl -s -X POST http://localhost:3000/api/user/abc/file \
  -H 'content-type: application/octet-stream' --data-binary @x.bin
```

**响应。**

```json
{ "error": { "code": "FILE_NOT_CONFIGURED", "message": "..." } }
```

```json
{ "error": { "code": "UPLOAD_NOT_CONFIGURED", "message": "..." } }
```

（均为 HTTP 501）

**优势所在。** 这是 `no-error-masking` 的现场落地：未接好的能力被**显式拒绝**为 `501`，且在
`queryOne` 之前判定 —— 绝无静默 text/plain 兜底、绝无伪造空文件。你可以程序化地区分「未配置」(501)
与「记录不存在」(404)。

### 7. RBAC 上下文注入：403 与 401

**业务背景。** 身份来自请求头里的 JWT，权限须由 store 强制。

**请求（被拒）。**

```bash
curl -s -o - -w '\n%{http_code}\n' http://localhost:3000/api/user \
  -H 'x-user: denied'
```

**响应。**

```json
{ "error": { "code": "PERMISSION_DENIED", "message": "权限不足" } }
```

（HTTP 403）

**请求（token 损坏）。**

```bash
curl -s -o - -w '\n%{http_code}\n' http://localhost:3000/api/user \
  -H 'x-user: broken'
```

**响应。**

```json
{ "error": { "code": "CONTEXT_ERROR", "message": "token 解析失败" } }
```

（HTTP 401）

**优势所在。** 适配器不解析任何头，把请求交给 `contextProvider`，由 **store** 判决。权限拒绝是 403
（RBAC）；上下文构造失败是 401 —— 且 401 是唯一以未认证呈现的场景。开启 `requireContext` 却未配
provider 时，装配期即以 `ERR_SECURE_CONFIG` 快速失败，而非每个请求运行期失败。

### 8. 多租户路由

**业务背景。** 每个租户有独立挂载点，共用一个适配器与一个 store。

**请求。**

```bash
curl -s http://localhost:3000/t/acme/api/user/abc
```

**优势所在。** 前缀由框架自有机制应用（Fastify 的 `prefix` register 选项、FastAPI 的
`create_app(prefix=...)` / `include_router`、axum 的 `.nest(...)`），不是 store-api 的重造。租户段纯属
HTTP 关切；适配器仍把 `{id}` 绑到 `_id` 等值条件，自身不添加任何租户语义。租户隔离是否成立完全由
store 的 RBAC（经 `contextProvider`）负责。

### 9. `x-cache` 透传注记

**业务背景。** 你想在每条响应上看到宿主观测到的缓存状态，而 store-api 不实现缓存。

**请求。**

```bash
curl -s -D - http://localhost:3000/api/user/abc -o /dev/null | grep -i x-cache
```

**响应。**

```
x-cache: BYPASS
```

**优势所在。** 未实现缓存时该头恒为 `BYPASS`，且携带于**每一条**响应（含 404、500），客户端可据
此稳定依赖其存在。日后宿主经 `store.cacheStatus()` 上报 `HIT` / `MISS`，同一响应头零 API 变更即可
承载。注记不会撒谎：无缓存绝不输出 `HIT`。

### 10. 不掩盖的错误语义

**业务背景。** 一次查询中途数据库连接超时；另一处，客户端请求一条不存在的记录。

**请求。**

```bash
curl -s -o - -w '\n%{http_code}\n' http://localhost:3000/api/user/nope
```

**响应。**

```json
{ "error": { "code": "NOT_FOUND", "message": "记录不存在: _id=nope" } }
```

（HTTP 404）

**请求（连接故障以 500 呈现）。**

```json
{ "error": { "code": "CONN_TIMEOUT", "message": "连接超时" } }
```

（HTTP 500）

**优势所在。** `queryOne` 返回空是真 404；store / 基建失败是带 store 原始 `code` 与 `message` 的
500。二者绝不混同 —— 失败的 resolver / provider 不会伪装成 404，缺失记录也不会伪装成 500。成功响应
永不携带 `error` 字段。

## 定制 / 注入点

所有选项均在适配器层；没有一个新增 REST 语义。

| 选项（node / py / go / rust） | 语义 | 默认 |
|---|---|---|
| `idField` / `id_field` | 单条路由主键字段名 | `_id`（各端必须一致） |
| `contextProvider` / `context_provider` / `ContextProvider` | 每请求上下文钩子（见「上下文与错误映射」） | 无 |
| `fileField` / `file_field` | 无 `?field=` 时字节子路由的适配器级默认字段 | `file` |
| `fileResolver` / `file_resolver` | 下载接缝 `(req, rec, id) => { body, contentType, fileName }` | 无 → 下载 501 |
| `uploadResolver` / `upload_resolver` | 上传接缝 `(req, rec, id) => { ref }` | 无 → 上传 501 |
| `uploadLimit` / `upload_limit` | 上传字节体上限（超限 → 413） | 32MB（`33554432`） |
| `resources` | 显式资源名数组；覆盖默认的 `list()` 减归档表 | 自动派生 |
| `errors.PermissionError`（node）/ `permission_error`（py） | 显式提供 store 权限错误类，免硬依赖宿主包 | `store.PermissionError` |
| `policyProvider` / `policy_provider`（node / py） | 每请求策略覆盖钩子 `(ctx) => overrides \| null`；需宿主提供 `baseView()` / `base_view()`；需 `withScope` / `with_scope` 作 optional peer | 无 |

说明：

- 注入 `uploadResolver` 后，上传会把返回的 `ref` 经 `store.update` 按字段路径回写（写操作走 RBAC，写
  权限自动生效）。
- 宿主开启 `requireContext` 却未配 `contextProvider` 时，装配期抛 `ERR_SECURE_CONFIG`。
- 注入 `policyProvider` 而宿主缺 `baseView()` / `base_view()` 时，装配期抛
  `ERR_POLICY_PROVIDER_CONFIG`（fail-fast；皮侧禁自持 Registry）。

## 边界 / 明确不提供

- **`PUT`**：store 的 `update` 是部分更新；`PUT` 的全量替换语义在 store 中不存在，本层不发明。
- **子资源路由** `/{resource}/{id}/{relation}`：不提供。关系数据通过列表查询的 GQL 关系展开获取。
  **唯一例外**是资源字节路由 `/{resource}/{id}/file`（下载 `GET` / 上传 `POST`），字节来源由接入方
  `fileResolver` / `uploadResolver` 注入 —— 皮只搬运。
- **路由名复数化**：路由段 = schema 资源名原样，不做单复数变换（各端复数规则必然漂移）。
- **批量端点**（`POST /{resource}/batch` 等）：store 有 `insertMany`/`updateMany`，但其条件 / 批量语义
  与 REST 惯例的映射未定，暂不进规范。
- **自造查询语言**：仅 GQL 透传与绑定，不提供第二套筛选 / 排序 / 分页语法。
- **无缓存实现**：`x-cache` 仅为注记。
- **无认证**：适配器不解析任何认证头。
- **资源 `_id` 是内容寻址**（sha1），只能由 store 的 `resourcePut` 按内容计算 —— 客户端**禁止**在
  `POST /Resource` 自带 `_id`（否则内容寻址可被伪造）。
- **`ResourceBinding`（`businessTable` / `businessId`）仅为元数据** —— 只记录归属关系，**不构成鉴权
  门禁**；写鉴权一律走 `update` / store 权限管道。
- 上传 parser 的注册是**实例级副作用**；同实例多插件共存须注意注册顺序。

## FAQ

**Q1. store-api 会认证请求吗？**
不会。它不解析任何认证头。身份经 `contextProvider` 提供，其余交给 store 的 RBAC。

**Q2. 为什么我的 `GET /{resource}` 列表只返回 `_id`？**
因为 GQL 契约是省略投影只返回 `_id`。若你传了带条件却不带投影的 `q`，投影完全由 `q` 决定，适配器
不再追加。要么在 `q` 里写投影，要么省略 `q`（适配器会按 schema `fields` 生成全字段投影）。

**Q3. 为什么 `?p.c0={"age":{"$gte":18}}` 绑成对象而 `?p.x=42` 绑成数字？**
所有 `p.*` 以字符串到达并转型：`true`/`false`/`null` 成字面量，整数与浮点正则命中成数值，`{`/`[`
开头做 JSON 解析；其余保持字符串原样。

**Q4. 我能在 `q` 里写字面量，例如 `$limit: 10` 吗？**
不能。GQL 参数值必须是 `@name` 引用；你把值经 `p.l=10` 传入并写 `$limit: @l`。字面量会被 core 以
`ERR_GQL_PARSE:` 拒绝 → HTTP 400 `GQL_PARSE`。

**Q5. 为什么收到 401 `CONTEXT_ERROR`？**
你的 `contextProvider` 抛了非权限错误（例如 JWT 解析失败）。这是唯一的 401 场景。权限拒绝 —— 或
`requireContext` 下上下文缺失 —— 是 403，不是 401。

**Q6. 为什么 `/{resource}/{id}/file` 返回 501？**
因为对应的 resolver 未注入（下载缺 `fileResolver` → `FILE_NOT_CONFIGURED`，上传缺 `uploadResolver`
→ `UPLOAD_NOT_CONFIGURED`）。该判定先于任何 `queryOne`，所以「未配置」总可与「记录不存在」(404)
区分。

**Q7. 下载路由上的两种 404 有何不同？**
同为 404 `NOT_FOUND`，但文案不同：(a) `queryOne` 返回空或字段缺失 ⇒「记录 / 文件不存在」；(b)
resolver 读取时资源无任何可读副本（`ERR_RESOURCE_NOT_FOUND:`）⇒ 资源对象确已不存在。**非**该前缀的
resolver 失败（如 `io down`）是 **500**，绝不是 404。

**Q8. 对不存在的 id 发 `PATCH` 会创建记录吗？**
不会。这里的 `update` 不是 upsert。它影响 0 行并返回 `200` 与 store 原始结果（某些端为 `null`）；
store-api 不把它变成 404。

**Q9. `DELETE` 是硬删吗？**
它映射 store 的 `remove`，归档编排在 store 内完成（`XxxDeleted` 归档镜像）。归档表不生成路由。

**Q10. 我能限定暴露哪些资源吗？**
能 —— 传 `resources` 选项给出显式名称数组。默认暴露 `list()` 减去归档表（以 `Deleted` 结尾且其基名
也已注册的名称）。

**Q11. 为什么 `/api/users` 是 404 而 `/api/user` 正常？**
路由段是 schema 名原样，不做复数化。想要 `/api/users` 就把 schema 注册为 `users`。

**Q12. 有缓存吗？**
没有。`x-cache` 是宿主缓存状态的透传注记；无宿主缓存时恒 `BYPASS`。它携带于所有响应，含错误响应。

## 开发

在各自目录下运行各端测试。

### Node（`node/`）

```bash
cd store-api/node
npm install
npm test          # 运行 `node --test`，覆盖 test/*.test.js
```

smoke 与 error-matrix 套件使用 mock store（不连真实库），与 conformance 用例对齐。

### Python（`py/`）

```bash
cd store-api/py
pip install -e ".[dev]"
pytest            # testpaths = ["tests"]
```

`tests/test_smoke.py` 与 `tests/test_error_matrix.py` 与 Node 套件同构。

### Go（`go/`）

```bash
cd store-api/go
go test ./...     # adapter_test.go + conformance_test.go（消费 ../conformance/cases/*.json）
```

Go 端 conformance runner 直接消费共享的 `../conformance/cases/*.json`。

### Rust（`rust/`）

```bash
cd store-api/rust
cargo test                       # 默认 feature 含 rust-store 桥接
cargo test --no-default-features # 纯库（mock smoke）形态
```

`tests/smoke.rs`、`tests/conformance.rs` 与 `tests/rust_store_e2e.rs`（真实宿主 + 真实 SQLite 走完整
HTTP 栈）默认全跑。

### 一致性纪律

任何路由 / 参数 / 错误语义的变更，必须先改 `spec/`，再改各端实现，并在 `conformance/cases/` 补用例。
规范、代码、用例三者不一致即为缺陷。见 [`conformance/README.md`](./conformance/README.md)。

## 相关项目

- [`../nodejs-store`](../nodejs-store) —— Node 宿主（npm）
- [`../py-store`](../py-store) —— Python 宿主（PyPI，`import py_store`）
- `../rust-store` —— Rust core + 宿主
- `../go-store` —— Go 宿主（仅 SQLite）
- `store-graphql` / `store-grpc` / `store-mcp` —— 同一 GQL 语义之上的同族协议皮
- `store-gateway` —— 组合器

## License

MIT
