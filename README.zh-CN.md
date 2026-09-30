# store-api

为 [common-store](../) 数据层家族（nodejs-store / py-store，后续 rust 宿主）提供 **schema 驱动的 RESTful API 自动生成**。

- Node 端：Fastify 适配器（`store-api-node`，npm）
- Python 端：FastAPI 适配器（`store-api-py`，PyPI）
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
├── spec/          # 共享 REST 映射规范（双端实现的唯一依据）
│   ├── 00-overview.md    # 定位与分层
│   ├── 01-routing.md     # schema → 路由映射
│   ├── 02-params.md      # 查询参数 → GQL 映射
│   ├── 03-errors.md      # 错误 → HTTP 状态码映射
│   └── 04-context.md     # 请求上下文 / RBAC 注入
├── conformance/   # 双端一致性用例（同一份 JSON，双端各自执行、断言一致）
├── node/          # store-api-node（Fastify 适配器）
└── py/            # store-api-py（FastAPI 适配器）
```

## 一致性纪律

任何路由/参数/错误语义的变更，必须先改 `spec/`，再双端同步实现，并在 `conformance/cases/` 补用例。两端 smoke / conformance 测试读取同一份场景 JSON，断言 HTTP 行为逐项一致。

## 路线

- v0：node / py 双包，CRUD + GQL 查询透传 + 错误映射 + 上下文注入（本仓库现状）
- v1：`@fastify/swagger` 与 Pydantic 双端 OpenAPI 文档对齐；conformance 双端互验 CI
- 之后：rust-store 宿主 crate（store-rs）落地后，增加 `rust/` axum 适配器为第三个包
