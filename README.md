# store-api

RESTful API auto-generation for the common-store data layer family (nodejs-store / py-store).

> 中文文档（主文档）：[README.zh-CN.md](./README.zh-CN.md)

- Node: Fastify adapter (`store-api-node`, npm)
- Python: FastAPI adapter (`store-api-py`, PyPI)
- Shared: `spec/` (single source of truth for routing/params/errors/context mapping) + `conformance/` (cross-runtime parity cases)

Design rule: **REST is just the HTTP skin over GQL** — the adapters invent zero semantics of their own; everything maps onto store's existing schema / GQL / RBAC semantics.
