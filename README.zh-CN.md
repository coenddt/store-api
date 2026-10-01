# store-api

为 [common-store](../) 数据层家族（nodejs-store / py-store，后续 rust 宿主）提供 **schema 驱动的 RESTful API 自动生成**。

- Node 端：Fastify 适配器（`store-api-node`，npm）
- Python 端：FastAPI 适配器（`store-api-py`，PyPI）
- Go 端：net/http 适配器（`store-api-go`，`go/` 子目录，适配 go-store 宿主；conformance runner 见 `go/conformance_test.go`）
- 共享：`spec/` 一份 REST 映射规范 + `conformance/` 一套双端一致性用例

## 设计哲学：REST 只是 GQL 的 HTTP 皮

适配层**零语义发明**：路由、参数、错误、权限全部映射到 store 已有的 schema / GQL / RBAC 语义上。凡是 store 没有的语义（如 PUT 全量替换、自动复数化路由），本层一律不提供。这是双端 parity 成本最低的唯一路径。

## 快速上手

### Node（Fastify）

```js
const { init, store } = require('nodejs-store');
const fastify = require('fastify');
const { storeApiPlugin } = require('store-api-node');

await init({ default: 'mongodb://...' });
store.register(userSchema);

const app = fastify();
await app.register(storeApiPlugin, { store, prefix: '/api' });
await app.listen({ port: 3000 });
// GET /api/user?q=user($condition: @c0) { name }  &  p.c0={"age":{"$gte":18}}
```

### Python（FastAPI）

```python
from py_store import init, store
from store_api_py import create_app

await init({'default': 'mongodb://...'})
store.register(user_schema)

app = create_app(store, prefix='/api')
# uvicorn main:app
```

## 仓库结构

```
store-api/
├── spec/          # 共享 REST 映射规范（三端实现的唯一依据）
│   ├── 00-overview.md    # 定位与分层
│   ├── 01-routing.md     # schema → 路由映射
│   ├── 02-params.md      # 查询参数 → GQL 映射
│   ├── 03-errors.md      # 错误 → HTTP 状态码映射
│   └── 04-context.md     # 请求上下文 / RBAC 注入
├── conformance/   # 三端一致性用例（同一份 JSON，各端各自执行、断言一致）
├── node/          # store-api-node（Fastify 适配器）
├── py/            # store-api-py（FastAPI 适配器）
└── rust/          # store-api-rs（axum 适配器，消费 Store trait；rust-store 宿主直接实现该 trait 接入）
```

## 一致性纪律

任何路由/参数/错误语义的变更，必须先改 `spec/`，再双端同步实现，并在 `conformance/cases/` 补用例。两端 smoke / conformance 测试读取同一份场景 JSON，断言 HTTP 行为逐项一致。

## 路线

- v0：node / py / rust 三包，CRUD + GQL 查询透传 + 错误映射 + 上下文注入（本仓库现状）
- v1：`@fastify/swagger` 与 Pydantic、utoipa 三端 OpenAPI 文档对齐；conformance 三端互验 CI
- rust/ 开箱桥接：cargo feature `rust-store`（`RustStoreAdapter` 新类型实现 `Store` trait，git 依赖 rust-store 宿主，发布 crates.io 后改版本依赖）；桥接 e2e 用真实宿主 + 真实 SQLite 走完整 HTTP 栈（含 guest 403 / 归档事务化）
