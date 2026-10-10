# store-api

RESTful API auto-generation for the common-store data layer family (nodejs-store / py-store / rust-store / go-store hosts).

> 中文文档（主文档）：[README.zh-CN.md](./README.zh-CN.md)

- Node: Fastify adapter (`store-api-node`, npm)
- Python: FastAPI adapter (`store-api-py`, PyPI)
- Go: net/http adapter (`store-api-go`, under `go/`, serving the go-store host; conformance runner in `go/conformance_test.go`; not published)
- Rust: axum adapter (`store-api-rs`, under `rust/`, consuming the `Store` trait; the rust-store host implements the trait directly to plug in; not published)
- Shared: `spec/` (single source of truth for routing/params/errors/context/response-headers mapping) + `conformance/` (cross-runtime parity cases)

Design rule: **REST is just the HTTP skin over GQL** — the adapters invent zero semantics of their own; everything maps onto store's existing schema / GQL / RBAC semantics.

## Recent capabilities

- Resource upload / download endpoints: `GET /{resource}/{id}/file` (`fileResolver`) and `POST /{resource}/{id}/file` (`uploadResolver`); 501 when the corresponding resolver is not injected.
- Error mapping: no readable replica (core stable prefix `ERR_RESOURCE_NOT_FOUND:`) → HTTP 404 `NOT_FOUND`; missing context (NoContext, `ERR_NO_CONTEXT:` / machine code `no_context`) → HTTP 403.
- `x-cache` response header: pass-through annotation of the host cache status (`store.cacheStatus()` / `cache_status()`), value `HIT` / `MISS` / `BYPASS`; always `BYPASS` when no cache is implemented, attached to **all** responses (including 4xx/5xx) — see `spec/05-response-headers.md`.
