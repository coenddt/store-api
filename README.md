# store-api

Schema-driven **RESTful API auto-generation** for the [common-store](../) data-layer family
(nodejs-store / py-store / rust-store / go-store hosts).

> REST is just the HTTP skin over **GQL** — the adapters invent **zero semantics of their own**;
> every route, parameter, error and permission decision maps onto store's existing
> schema / GQL / RBAC semantics. Anything store does not have, this layer does not provide.

> 中文文档：[README.zh-CN.md](./README.zh-CN.md)

- Node: Fastify adapter (`store-api-node`, npm)
- Python: FastAPI adapter (`store-api-py`, PyPI)
- Go: `net/http` adapter (`store-api-go`, under `go/`, serving the go-store host; conformance runner in `go/conformance_test.go`; **not published**)
- Rust: axum adapter (`store-api-rs`, under `rust/`, consuming the `Store` trait; the rust-store host implements the trait directly to plug in; **not published**)
- Shared: `spec/` (single source of truth for routing / params / errors / context / response-headers mapping) + `conformance/` (cross-runtime parity cases)

## Runtime matrix

| Runtime | Framework | Package | Distribution | File upload/download routes | `x-cache` header | Status |
|---|---|---|---|---|---|---|
| Node | Fastify | `store-api-node` | npm | ✅ | ✅ | published |
| Python | FastAPI | `store-api-py` | PyPI | ✅ | ✅ | published |
| Go | `net/http` (stdlib, zero framework) | `store-api-go` (module `github.com/coenddt/store-api-go`) | in-repo `go/` | — | — | not published |
| Rust | axum | `store-api-rs` | in-repo `rust/` | — | — | not published |

> The CRUD route table (list / get-one / create / patch / delete) is implemented on **all four**
> runtimes. The resource byte sub-routes (`GET|POST /{resource}/{id}/file`) and the `x-cache`
> response header are currently implemented on **node** and **py** only; the go and rust adapters
> are CRUD-only. See [Boundaries](#boundaries--explicitly-not-supported).

### Recent capabilities

- Resource upload / download endpoints: `GET /{resource}/{id}/file` (`fileResolver`) and
  `POST /{resource}/{id}/file` (`uploadResolver`); 501 when the corresponding resolver is not
  injected.
- Error mapping: no readable replica (core stable prefix `ERR_RESOURCE_NOT_FOUND:`) → HTTP 404
  `NOT_FOUND`; missing context (NoContext, `ERR_NO_CONTEXT:` / machine code `no_context`) → HTTP 403.
- `x-cache` response header: pass-through annotation of the host cache status
  (`store.cacheStatus()` / `cache_status()`), value `HIT` / `MISS` / `BYPASS`; always `BYPASS` when no
  cache is implemented, attached to **all** responses (including 4xx/5xx) — see
  `spec/05-response-headers.md`.

## Table of contents

- [What it is](#what-it-is)
- [When to use it](#when-to-use-it)
- [When not to use it](#when-not-to-use-it)
- [Installation](#installation)
- [Quick start](#quick-start)
  - [Node (Fastify)](#node-fastify)
  - [Python (FastAPI)](#python-fastapi)
  - [Go (net/http)](#go-nethttp)
  - [Rust (axum)](#rust-axum)
- [Routing & URL shape](#routing--url-shape)
- [Params reference](#params-reference)
- [Context & error mapping](#context--error-mapping)
- [Response headers](#response-headers)
- [Scenarios](#scenarios)
- [Customization / injection points](#customization--injection-points)
- [Boundaries / explicitly not supported](#boundaries--explicitly-not-supported)
- [FAQ](#faq)
- [Development](#development)
- [Related projects](#related-projects)
- [License](#license)

## What it is

`store-api` is an **HTTP adaptation layer** on top of the common-store data layer. It is *not* a new
data layer, and it does *not* implement authentication, rate limiting or caching. Its only job is to
translate HTTP ⇄ store calls.

```
HTTP client
   │  RESTful (JSON)
store-api   (this repo: fastify / fastapi / net/http / axum adapters)
   │  store native API: list / query / queryOne / insert / update / remove + setContext
data layer  (nodejs-store / py-store / go-store / rust-store host)
   │  GQL → command planning
rust-store core  (parser / permission / SQL dialects)
   │
database    (MongoDB / MySQL / PostgreSQL / SQLite)
```

### Design rule

1. **Zero semantic invention.** Routes, params, errors and permission semantics must map 1:1 onto
   existing store semantics. If store has no such concept, this layer does not offer it.
2. **Parity first.** Any change lands in `spec/` first, then in the runtimes, then a `conformance/`
   case. Spec, code and cases that disagree are a defect.
3. **Never mask errors** (project-wide `no-error-masking` rule). Every error store raises is mapped to
   a semantically meaningful HTTP error response. No silent swallowing, no default fallback values,
   no treating a neutral result as an error. Successful responses must not carry error text.

### How store semantics are reused

- **Schema → routes.** The adapter reads the registered schema list (`store.list()` /
  `store.ListSchemas()` / `Store::list`) and generates one route family per resource. Route segments
  are the schema resource name **verbatim** — no pluralization.
- **Query → GQL.** List querying (`GET /{resource}`) is GQL, transmitted and bound, not re-invented.
  The adapter never defines a second filter/sort/page grammar.
- **Permissions → RBAC.** The adapter never authenticates. It injects a per-request context via
  `setContext` / `set_context`; the store's own RBAC pipeline then decides. A permission denial is a
  store error mapped to HTTP 403.

## When to use it

- You already use a `*-store` host and want a REST surface **without hand-writing** per-resource
  controllers, filters, pagination or permission checks.
- You want the REST surface to stay **identical across runtimes** — the same GQL, the same error
  codes, the same status codes.
- You want your filtering / sorting / aggregation to be the **full GQL** language rather than a
  bespoke query-string mini-language.
- You want RBAC to be enforced by the store, with the HTTP layer only responsible for translating the
  request into a context.
- You want **loud, honest failures**: an unimplemented capability (e.g. a missing file resolver)
  returns `501`, not a silent fallback.

## When not to use it

- You need authentication, sessions, rate limiting or caching **implemented** here — `store-api`
  deliberately does none of that. Only the `x-cache` **annotation** is emitted; there is no cache.
- You need `PUT` full-replacement, auto-pluralized routes, relation sub-resources
  (`/{resource}/{id}/{relation}`) or batch endpoints — these are intentionally absent (see
  [Boundaries](#boundaries--explicitly-not-supported)).
- You are not on a `*-store` host — there is nothing to adapt.
- You need a GraphQL / gRPC / MCP surface instead — use the sibling protocol skins
  (`store-graphql` / `store-grpc` / `store-mcp`).

## Installation

### Node (npm)

```bash
npm install store-api-node
# peer dependency (optional at install time, required at runtime unless you pass errors.PermissionError):
npm install nodejs-store
```

### Python (PyPI)

```bash
pip install store-api-py
# optional dev extras (pytest, httpx, uvicorn):
pip install "store-api-py[dev]"
```

### Go

The Go adapter is **not published**; consume it from the repository (the module is
`github.com/coenddt/store-api-go` and `go.mod` `replace`s `github.com/coenddt/go-store` to the local
`../../go-store` checkout):

```bash
go get github.com/coenddt/store-api-go
go get github.com/coenddt/go-store
```

### Rust

The Rust adapter is **not published**; consume it from the repository as a path dependency. The
default feature `rust-store` enables the out-of-the-box bridge to the rust-store host:

```toml
[dependencies]
store-api-rs = { path = "../store-api/rust" }
# pure-library form (mock smoke only):
# store-api-rs = { path = "../store-api/rust", default-features = false }
```

## Quick start

### Node (Fastify)

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
// Register all schemas BEFORE mounting the adapter.
await app.register(storeApiPlugin, { store, prefix: '/api' });
await app.listen({ port: 3000 });

// GET /api/user?q=($condition: @c0) { name, age }&p.c0={"age":{"$gte":18}}
```

### Python (FastAPI)

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

### Go (net/http)

```go
package main

import (
	"net/http"

	gostore "github.com/coenddt/go-store"
	storeapi "github.com/coenddt/store-api-go"
)

func main() {
	st, _ := gostore.Open("sqlite://app.db")
	// st.Register(...) + st.EnsureTables(...) first — register all schemas before mounting routes.

	mux := http.NewServeMux()
	_ = storeapi.New(mux, st, storeapi.Options{
		Prefix:  "/api",
		IDField: "_id",
		ContextProvider: func(r *http.Request) (*gostore.Context, error) {
			token := r.Header.Get("X-User")
			if token == "" {
				return nil, nil // explicit empty context
			}
			return &gostore.Context{UserID: token, Roles: []string{"admin"}}, nil
		},
	})
	http.ListenAndServe(":3000", mux)
}
```

A full runnable demo (SQLite file store + real HTTP + computed columns) lives in
[`go/demo/main.go`](./go/demo/main.go):

```bash
cd store-api/go && go run ./demo
# → REST service ready at http://127.0.0.1:8091/api/order
```

### Rust (axum)

```rust
use std::sync::Arc;
use store_api_rs::{create_router, Options, Store};

// Implement `Store` for your host (the rust-store bridge `RustStoreAdapter` does this for you).
let store: Arc<dyn Store> = Arc::new(my_store);

let router = create_router(store, Options {
    resources: None,           // default: store.list() minus archive tables
    id_field: "_id".into(),
    context_provider: None,    // Option<ContextProvider>, see §Context
}).await;

let app = axum::Router::new().nest("/api", router);
// axum::serve(listener, app).await?;
```

The adapter exposes a wildcard route family (`/{resource}`), so the caller controls the prefix via
`.nest("/api", router)` — aligned with Fastify's `prefix` mechanism.

## Routing & URL shape

Prerequisite: schemas are registered with the store (`store.register(defn)` /
`store.Register(...)`). The adapter reads the resource list when it mounts and generates the routes
below. Schemas registered *after* mounting are **not** guaranteed to take effect (framework
route-registration timing differs between runtimes) — register every schema first, then mount.

| Method | Path | store call | Success response |
|---|---|---|---|
| GET | `/{prefix}/{resource}` | `query(gql, params)` | `200 {"data": [...]}` |
| GET | `/{prefix}/{resource}/{id}` | `queryOne(gql, params)` | `200 {"data": {...}}` |
| POST | `/{prefix}/{resource}` | `insert(schemaName, body)` | `201 {"data": <insert result>}` |
| PATCH | `/{prefix}/{resource}/{id}` | `update(schemaName, cond, body)` | `200 {"data": <update result>}` |
| DELETE | `/{prefix}/{resource}/{id}` | `remove(schemaName, cond)` | `200 {"data": <delete result>}` |
| GET | `/{prefix}/{resource}/{id}/file` | `queryOne(gql, params)` | `200` byte stream + `content-type` / `content-disposition` (**not a JSON envelope**) |
| POST | `/{prefix}/{resource}/{id}/file` | `queryOne` → `uploadResolver` → `update` | `200 {"data": <record after write-back>}` |

How each URL part maps to GQL:

| URL part | Meaning | GQL mapping |
|---|---|---|
| `{resource}` | schema resource name, **verbatim** | the GQL resource head (`user(...) { ... }`) |
| `{id}` | primary key equality condition | condition `{ [idField]: id }` bound as `@c0`; GQL `user($condition: @c0) { ... }` |
| `{id}/file` | resource byte sub-route | see [Scenarios](#scenarios) — backed by `fileResolver` / `uploadResolver` |
| `?q=...` | filter / sort / page / relation / aggregate | appended verbatim after the resource head |
| `?p.<name>=...` | GQL named parameter | bound to `@<name>` in `params` |
| `?field=...` | field dot-path for the byte sub-routes | locates the field carrying the resource reference |

### Primary key convention

The single-record routes bind `{id}` to a primary-key equality condition. The key field name is the
adapter option `idField`; **the default is `'_id'` on every runtime** and must stay consistent.

```
GET /api/user/abc
  ⇒ store.queryOne('user($condition: @c0) { name, age }', { c0: { _id: 'abc' } })
```

### Projection must be generated explicitly

GQL's contract is: **omitting the projection section returns only `_id`** (a core contract shared by
all hosts; core `compute_keep` keeps only `_id` for empty fields) — *not* "all fields". Therefore:

- Single-record routes and `POST` responses get an **explicit projection** built from the schema's
  `fields` keys.
- A list route with no `q` also gets an explicit full-field projection.
- A list route *with* `q` lets `q` decide the projection entirely; the adapter adds nothing.
- If a schema declares no `fields`, the projection degrades to empty → `data` contains only `_id`.
  That is the explicit, visible consequence of an incomplete upstream schema — no silent fallback.

### Archive-table filtering (identical across runtimes)

`store.list()` includes auto-derived archive tables (registering `Xxx` auto-derives an `XxxDeleted`
mirror). The rule, applied verbatim on every runtime:

> A name is treated as an archive table — and gets **no route** — if it ends with `Deleted` **and**
> the name without that suffix is also present in the list.

Every adapter also exposes a `resources` option (an explicit array of resource names) to override the
default and pin the exposed surface.

## Params reference

All list-query capability comes from GQL itself. The adapter only **transmits and binds**; it never
invents a second filter/sort/page grammar.

| Query param | Meaning | Mapping |
|---|---|---|
| `q` | GQL query body (the part **after the resource head** — the condition and everything following) | appended to the GQL string |
| `p.<name>` | binds the named GQL parameter `@<name>` | collected into the `params` object |
| `field` | field dot-path for the byte sub-routes (`GET|POST /{resource}/{id}/file`) | locates the field carrying the resource reference; defaults to the adapter option `fileField` |

### Shape of `q`

`q` is the resource-head-less GQL fragment. The adapter prepends the resource head:

```
GET /api/user?q=($condition: @c0, $limit: @l) { name, age }&p.l=10
```

becomes `user($condition: @c0, $limit: @l) { name, age }` — equivalent to calling
`store.query('user($condition: @c0, $limit: @l) { name, age }', { c0: {...}, l: 10 })`.

> **GQL parameter values are always `@name` named references** (core GQL syntax; py-store and
> nodejs-store are isomorphic). **Literals are not accepted** — numbers, booleans and objects are all
> passed through `p.<name>` and coerced on the params side. A literal such as `$limit: 10` is
> rejected by core with `ERR_GQL_PARSE:`.

Sorting, paging, relation expansion and aggregation all use GQL's own syntax
(`$orderby` / `$limit` / `$offset` / relation sub-queries / `$group`, …). This spec does not
redefine them — the store's GQL documentation is authoritative.

### Value coercion for `p.<name>`

Every `p.*` value arrives as a **string**. Coercion rules (first match wins; no match keeps the raw
string), applied verbatim on every runtime:

1. `true` / `false` → boolean
2. `null` → null
3. fully matching the integer regex `^-?\d+$` → integer
4. fully matching the float regex `^-?\d+\.\d+$` → float
5. otherwise → the string as-is

For complex structures (objects/arrays): if the value is valid JSON it is parsed into an object; if
parsing fails → `400` with the raw parse error (never silently treated as a string).

When the **same** param appears repeatedly (`p.tag=a&p.tag=b`), the bound value is an **array**
(each element coerced individually); a single occurrence binds a scalar.

### `field` shape and limits

- `field` is a **dot-path** (e.g. `images.full`, `file`), resolved segment by segment. When omitted,
  the adapter option `fileField` is used.
- Only dot-path addressing is supported (`a.b`); **no** array indices / wildcards / expressions.
- If any segment is missing ⇒ treated as "the file field is empty" ⇒ `404 NOT_FOUND` (the same
  judgement as a missing `fileField`).
- This param affects **only** the `file` sub-route; list and single-record routes ignore it (no error).

### When there is no `q`

The adapter generates a full-field projection from the schema `fields` keys — otherwise (no
projection) the contract means returning only `_id`. Whether rows are trimmed by permissions is
decided by store RBAC; the adapter appends no default condition.

### Errors

- A `q` that fails store parsing (GQL syntax error) ⇒ `400`, body carries store's raw error
  message verbatim.
- A `p.*` JSON parse failure ⇒ `400`, carrying the raw parse error.

## Context & error mapping

### Context injection (RBAC)

The store's RBAC is keyed on the request context
(`store.setContext(ctx)` / `store.set_context(ctx)`). `store-api` performs **no authentication** — it
only provides a hook that translates the HTTP request into a context.

| Option | Type | Meaning |
|---|---|---|
| `contextProvider` / `context_provider` | `(req) => ctx \| Promise<ctx>` | Called per request; the return value becomes this request's store context. |

Behavior:

1. Before each handler, if `contextProvider` is configured, it is called and its result is injected
   via `store.setContext(ctx)`.
2. Errors thrown by the provider are classified: a `PermissionError` ⇒ `403` (same semantics as a
   permission denial inside a business handler); any other error ⇒ `401` with code `CONTEXT_ERROR`,
   `message` passed through verbatim. This is the **only** case rendered as `401`. A store RBAC
   denial (context set but permissions insufficient) is `403`.
3. The adapter parses **no** auth headers (`Authorization` / `Cookie` / `API-Key` are untouched).
   Identity comes entirely from `contextProvider` — wire your own JWT / session there.
4. State-holding runtimes (node / py / go) must inject an explicit empty context when the provider
   returns `null`/`undefined` (`setContext(null)` clear semantics) so no previous request's identity
   leaks. The Rust adapter is request-scoped — the context travels with `request.extensions`, there
   is no global state to clear.
5. The store's `requireContext` tier (`store.setRequireContext(true)`) is set by the integrator.
   With it on, a request without context is rejected by the store as `NoContextError` (core machine
   code `no_context`, same permission class as `PermissionError`) → mapped to `403`. The adapter adds
   no second layer of judgement.
6. **Fail-secure assembly guard.** If the host already has context enforcement on (`requireContext`
   true) but no `contextProvider` is configured, assembly fails immediately with
   `ERR_SECURE_CONFIG` instead of deferring the mistake to run time.

### Error response shape

```json
HTTP 4xx/5xx
{ "error": { "code": "<store error name or adapter error name>", "message": "<raw error message, verbatim>" } }
```

- `code`: prefers the store error object's `code`/`name`; otherwise the adapter error code.
- `message`: the error object's `message`, passed through verbatim — no rewriting, no translation,
  no sugar-coating.
- A success response (2xx) must **not** contain an `error` field or any error-semantics wording.

### Status-code mapping table (identical across runtimes)

| Case | Status | code |
|---|---|---|
| Resource not registered (defensive; theoretically unreachable after route generation) | 404 | `RESOURCE_NOT_FOUND` |
| Single `queryOne` returns empty | 404 | `NOT_FOUND` |
| Download `GET /{resource}/{id}/file`: record missing / file field empty | 404 | `NOT_FOUND` |
| Download `GET /{resource}/{id}/file`: `fileResolver` read finds no readable replica (core stable prefix `ERR_RESOURCE_NOT_FOUND:`) | 404 | `NOT_FOUND` |
| Missing / non-object JSON request body | 400 | `INVALID_BODY` |
| Illegal `p.*` value (JSON parse failure) | 400 | `INVALID_PARAM` |
| GQL parse failure (store-raised; core stable prefix `ERR_GQL_PARSE:`) | 400 | `GQL_PARSE` |
| store `PermissionError` (RBAC denial) | 403 | store's original code/name |
| store `NoContextError` (`requireContext` on, no ctx; core prefix `ERR_NO_CONTEXT:`, machine code `no_context`) | 403 | store's original code/name |
| Any other store error (database, connection, dialect, …) | 500 | store's original code/name |
| Upload `POST /{resource}/{id}/file`: empty body | 400 | `EMPTY_BODY` |
| Upload `POST /{resource}/{id}/file`: body exceeds `uploadLimit` | 413 | `UPLOAD_TOO_LARGE` |
| Upload `POST /{resource}/{id}/file`: `uploadResolver` not injected | 501 | `UPLOAD_NOT_CONFIGURED` |
| Download `GET /{resource}/{id}/file`: `fileResolver` not injected | 501 | `FILE_NOT_CONFIGURED` |
| `contextProvider` throws a non-permission error | 401 | `CONTEXT_ERROR` |

### Decision order (must be identical across runtimes)

1. **Resolver not injected** (upload without `uploadResolver` / download without `fileResolver`)
   → 501 `UPLOAD_NOT_CONFIGURED` / `FILE_NOT_CONFIGURED`.
2. **Adapter guards** (body / param legality, empty body, over-limit) → 400
   `INVALID_BODY` / `INVALID_PARAM` / `EMPTY_BODY`, 413 `UPLOAD_TOO_LARGE`.
3. **Permission-class errors** (by error type / core machine code, **never** by message string
   matching) → 403: `store.PermissionError` and `store.NoContextError`.
4. **Core stable-prefix errors** (prefix stripped after construction; no fragile matching against
   Chinese message text): `ERR_GQL_PARSE:` → 400 `GQL_PARSE`; `ERR_RESOURCE_NOT_FOUND:` → 404
   `NOT_FOUND`. Both carry the prefix-stripped core message.
5. **`queryOne` empty result** (including the file sub-route "record missing / field empty") → 404.
6. **Everything else** → 500, passed through.

> For the file sub-route: resolver not injected ⇒ 501 (layer 1, before all else); `queryOne` empty ⇒
> record does not exist ⇒ 404 `NOT_FOUND`; the record exists but the field is empty (`null` / missing
> / any `field` dot-path segment missing) ⇒ file does not exist ⇒ 404 `NOT_FOUND` (different message,
> same status). The upload route adds two more: empty body ⇒ 400 `EMPTY_BODY`; body over `uploadLimit`
> ⇒ 413 `UPLOAD_TOO_LARGE`.
>
> Two kinds of resource read error (core `resourceOpen` semantics): **zero replica rows** throws
> `ERR_RESOURCE_NOT_FOUND:` ⇒ 404 `NOT_FOUND` (the resource genuinely does not exist); **a replica row
> exists but every provider `get` fails** ⇒ core rethrows the last provider error verbatim ⇒ 500
> passthrough (an IO/degradation failure, not "not found"). The two are never conflated.

### Forbidden (per `no-error-masking`)

- ❌ `catch` then return `{"data": null}` or an empty array to launder an error into success.
- ❌ Fabricating a `message` with `|| 'unknown error'` in a 500 — if there is no message, set it to
  `null` explicitly and keep the code.
- ❌ Treating a neutral store return as an error: `update`/`remove` affecting 0 rows returns `200`
  with the store's raw result; whether that is a business anomaly is the caller's call.

## Response headers

### `x-cache`

- **Meaning**: annotates whether this HTTP response went through a cache. It is an **annotation**,
  not a cache implementation. Values are always `HIT` / `MISS` / `BYPASS`.
  - `BYPASS`: the response did not go through a cache — **the only value actually produced in this
    version** (no cache is implemented).
  - `HIT` / `MISS`: reserved for a future cache implementation, reported by a host provider.
- **Source**: host `store.cacheStatus()` (node) / `store.cache_status()` (py). `store-api` invents no
  cache semantics — it only passes the annotation through.
  - Host does not provide the method → always `BYPASS`.
  - Host returns an illegal value / throws → always `BYPASS` (the host side logs illegal/exceptional
    cases via its feedback channel — never silent).
- **Coverage**: attached to **all** HTTP responses (2xx and 4xx/5xx alike).
- **Forbidden**: the annotation must be true — never emit `HIT` / `MISS` without a cache.

> This is implemented on node and py. The go and rust adapters do not emit `x-cache`.

### Download response headers (`GET /{resource}/{id}/file`)

- **`content-type`**: the download media type, decided by the `contentType` returned from
  `fileResolver`; if omitted → falls back to `application/octet-stream`. (No `fileResolver` injected ⇒
  501 `FILE_NOT_CONFIGURED`, no headers produced.)
- **`content-disposition`**: always `attachment; filename="<fileName>"`; `fileName` comes from
  `fileResolver`'s return value, defaulting to `file`.
- **Semantic boundary**: the skin does not infer MIME (to avoid inventing semantics); `contentType` /
  `fileName` are decided by the integrator's `fileResolver`. The download body is a **byte stream**,
  not a JSON envelope.

## Scenarios

Every scenario follows the same shape: **business context → request → response → why it wins**.
Resource names, field names and method names are taken from this repository's source and the
`conformance/cases/*.json` files.

### 1. Internal admin back-office CRUD

**Context.** An internal tool needs create/read/update/delete over a `user` resource. Hand-writing a
controller per resource is boilerplate that drifts.

**Request.**

```bash
# create
curl -s -X POST http://localhost:3000/api/user \
  -H 'content-type: application/json' \
  -d '{"name":"alice","age":30}'

# read one
curl -s http://localhost:3000/api/user/abc

# partial update
curl -s -X PATCH http://localhost:3000/api/user/abc \
  -H 'content-type: application/json' \
  -d '{"age":31}'

# delete
curl -s -X DELETE http://localhost:3000/api/user/abc
```

**Response** (201 on create, extracted from `conformance/cases/users-crud.json`):

```json
{ "data": { "_id": "abc", "name": "alice", "age": 30 } }
```

**Why it wins.** Five routes are generated per resource with no controller code. `PATCH` is a partial
update mapping straight onto store's `update`; the adapter does not validate business fields (that is
the schema's job). Notably there is **no `PUT`** — the store has no full-replacement semantics, so
neither does the API.

### 2. A filtered list over a relation/field, using full GQL

**Context.** A dashboard needs users with `age >= 18`, and you refuse to hand-roll a query-string
mini-language.

**Request.**

```bash
curl -s -G 'http://localhost:3000/api/user' \
  --data-urlencode 'q=($condition: @c0) { name, age }' \
  --data-urlencode 'p.c0={"age":{"$gte":18}}'
```

**Response.**

```json
{ "data": [ { "_id": "abc", "name": "alice", "age": 30 } ] }
```

**Why it wins.** The adapter prepends the resource head and binds `@c0`; the filter is **GQL**, so
every operator GQL supports works with zero adapter changes. `q` decides the projection entirely —
the adapter adds nothing. Hand-written REST would need a bespoke parser; here the grammar is store's.

### 3. Offset pagination (`$limit` / `$offset`)

**Context.** A list page shows 10 rows at a time.

**Request.**

```bash
curl -s -G 'http://localhost:3000/api/user' \
  --data-urlencode 'q=($limit: @l, $offset: @o) { name, age }' \
  --data-urlencode 'p.l=10' \
  --data-urlencode 'p.o=20'
```

**Response.**

```json
{ "data": [ /* at most 10 rows */ ] }
```

**Why it wins.** Paging is GQL's, bound through `p.*`. Note the **values are `@name` references** —
`p.l=10` coerces the string `"10"` to the integer `10` and binds `@l`; writing `$limit: 10` as a
literal would be rejected by core with `ERR_GQL_PARSE:` (→ 400 `GQL_PARSE`). store-api never adds a
`total` field — that would be semantic invention; a count is a separate GQL aggregate query
(specific aggregate syntax: 待核实 / out of scope for this spec).

### 4. Soft delete & audit through the archive mirror

**Context.** Deletes must not vanish; the store automatically keeps an `XxxDeleted` archive mirror.

**Request.**

```bash
curl -s -X DELETE http://localhost:3000/api/user/abc
```

**Response.**

```json
{ "data": { /* store's raw remove result */ } }
```

**Why it wins.** `DELETE` maps to store's `remove`, which handles archive orchestration inside the
store (transactional) — the HTTP layer does nothing special. The auto-derived `userDeleted` archive
table gets **no route** (archive-table filtering), so the audit copy is not accidentally exposed as a
writable REST resource. If you delete a non-existent id, `remove` affects 0 rows and still returns
`200` with the raw result — the API does not invent a 404 there (that is the caller's judgement).

### 5. Resource download and upload (`fileResolver` / `uploadResolver`)

**Context.** A `user` record carries a file reference. You want HTTP download and upload without
store-api doing storage IO, signing URLs or making permission calls.

**Download request.**

```bash
curl -s -D - http://localhost:3000/api/user/abc/file -o out.bin
```

**Download response headers.**

```
content-type: application/octet-stream
content-disposition: attachment; filename="file"
x-cache: BYPASS
```

(body is a byte stream, not JSON)

**Upload request.**

```bash
curl -s -X POST http://localhost:3000/api/user/abc/file \
  -H 'content-type: application/octet-stream' \
  --data-binary @payload.bin
```

**Upload response.**

```json
{ "data": { "_id": "abc", "name": "alice", "file": "sha1abc" } }
```

**Why it wins.** Download is `queryOne` (locate the record and the field) → `fileResolver` (produce
bytes); upload is `queryOne` → `uploadResolver` (store bytes, return a `ref`) → `update` (write the
`ref` back). Because the write-back goes through **`update`**, RBAC **write** permission is enforced
automatically — the skin never judges permissions itself. Bytes are carried by an
`application/octet-stream` buffered parser whose size is capped by `uploadLimit` (default 32 MB).

Target a nested field with the `field` dot-path:

```bash
curl -s -X POST 'http://localhost:3000/api/user/abc/file?field=images.full' \
  -H 'content-type: application/octet-stream' \
  --data-binary @cover.jpg
```

### 6. The 501 semantics of an un-injected resolver

**Context.** You deploy with the streaming routes present but byte storage not yet wired up.

**Request.**

```bash
curl -s -o - -w '\n%{http_code}\n' http://localhost:3000/api/user/abc/file
curl -s -X POST http://localhost:3000/api/user/abc/file \
  -H 'content-type: application/octet-stream' --data-binary @x.bin
```

**Response.**

```json
{ "error": { "code": "FILE_NOT_CONFIGURED", "message": "..." } }
```

```json
{ "error": { "code": "UPLOAD_NOT_CONFIGURED", "message": "..." } }
```

(both HTTP 501)

**Why it wins.** This is the `no-error-masking` rule in action: a capability that is not wired up is
**explicitly refused** with `501 NOT_IMPLEMENTED`, before any `queryOne` runs — never a silent
text/plain fallback, never a fake empty file. You can tell "not configured" (501) apart from "record
missing" (404) programmatically.

### 7. RBAC context injection: 403 vs 401

**Context.** Your identity comes from a JWT in a header, and the store must enforce RBAC.

**Request (denied).**

```bash
curl -s -o - -w '\n%{http_code}\n' http://localhost:3000/api/user \
  -H 'x-user: denied'
```

**Response.**

```json
{ "error": { "code": "PERMISSION_DENIED", "message": "权限不足" } }
```

(HTTP 403)

**Request (broken token).**

```bash
curl -s -o - -w '\n%{http_code}\n' http://localhost:3000/api/user \
  -H 'x-user: broken'
```

**Response.**

```json
{ "error": { "code": "CONTEXT_ERROR", "message": "token 解析失败" } }
```

(HTTP 401)

**Why it wins.** The adapter parses no headers itself; it hands the request to `contextProvider`, and
the **store** decides. A permission denial is 403 (RBAC); a context-construction failure is 401 — and
401 is the only case rendered as unauthenticated. With `requireContext` on but no provider, assembly
fails fast at startup with `ERR_SECURE_CONFIG` rather than failing every request at run time.

### 8. Multi-tenant routing

**Context.** Each tenant gets its own mount point, all sharing one adapter and one store.

**Request.**

```bash
curl -s http://localhost:3000/t/acme/api/user/abc
```

**Why it wins.** The prefix is applied by the framework's own mechanism (Fastify's `prefix` register
option, FastAPI's `create_app(prefix=...)`, FastAPI `include_router`, axum's `.nest(...)`), not by a
store-api re-invention. The tenant segment is purely an HTTP concern; the adapter still binds `{id}`
to the `_id` equality condition and adds no tenant semantics of its own. Whether tenant isolation is
enforced is entirely the store's RBAC's job (via `contextProvider`).

### 9. `x-cache` pass-through annotation

**Context.** You want the cache status observed by the host surfaced on every response, without
store-api implementing a cache.

**Request.**

```bash
curl -s -D - http://localhost:3000/api/user/abc -o /dev/null | grep -i x-cache
```

**Response.**

```
x-cache: BYPASS
```

**Why it wins.** With no cache implemented the header is always `BYPASS` — and it is attached to
**every** response, including 404s and 500s, so a client can rely on it always being present. When a
host later reports `HIT` / `MISS` via `store.cacheStatus()`, the same header carries it with zero API
change. The annotation cannot lie: without a cache it never says `HIT`.

### 10. Error semantics that are not masked

**Context.** A database connection times out mid-query, *and* separately a client asks for a
non-existent record.

**Request.**

```bash
curl -s -o - -w '\n%{http_code}\n' http://localhost:3000/api/user/nope
```

**Response.**

```json
{ "error": { "code": "NOT_FOUND", "message": "记录不存在: _id=nope" } }
```

(HTTP 404)

**Request (connection failure surfaces as 500).**

```json
{ "error": { "code": "CONN_TIMEOUT", "message": "连接超时" } }
```

(HTTP 500)

**Why it wins.** `queryOne` returning empty is a genuine 404; a store/infra failure is a 500 carrying
store's raw `code` and `message`. The two are never conflated — a failing resolver/provider does not
masquerade as a 404, and a missing record does not masquerade as a 500. A success response never
carries an `error` field.

## Customization / injection points

All options are adapter-level; none of them add new REST semantics.

| Option (node / py / go / rust) | Meaning | Default |
|---|---|---|
| `idField` / `id_field` | Primary-key field name for single-record routes | `_id` (must be identical everywhere) |
| `contextProvider` / `context_provider` / `ContextProvider` | Per-request context hook (see §Context) | none |
| `fileField` / `file_field` | Adapter-level default field for the byte sub-routes when `?field=` is absent | `file` |
| `fileResolver` / `file_resolver` | Download seam `(req, rec, id) => { body, contentType, fileName }` | none → download returns 501 |
| `uploadResolver` / `upload_resolver` | Upload seam `(req, rec, id) => { ref }` | none → upload returns 501 |
| `uploadLimit` / `upload_limit` | Upload body size cap (over-limit → 413) | 32 MB (`33554432`) |
| `resources` | Explicit resource-name array; overrides the default `list()` minus archive tables | derived |
| `errors.PermissionError` (node) / `permission_error` (py) | Explicitly supply the store permission-error class to avoid a hard dependency on the host package | `store.PermissionError` |
| `policyProvider` / `policy_provider` (node / py) | Per-request policy override hook `(ctx) => overrides \| null`; requires the host to expose `baseView()` / `base_view()`; needs `withScope` / `with_scope` as an optional peer | none |

Notes:

- Injecting `uploadResolver` makes upload write the returned `ref` back via `store.update` under the
  field path (the write goes through RBAC, so write permission is enforced automatically).
- If the host is `requireContext`-enforcing but no `contextProvider` is passed, assembly throws
  `ERR_SECURE_CONFIG`.
- If `policyProvider` is injected but the host lacks `baseView()` / `base_view()`, assembly throws
  `ERR_POLICY_PROVIDER_CONFIG` (fail-fast; the skin never holds its own registry).

## Boundaries / explicitly not supported

- **`PUT`**: store's `update` is partial. `PUT`'s full-replacement semantics do not exist in store —
  not invented here.
- **Relation sub-routes** `/{resource}/{id}/{relation}`: not provided. Relation data is fetched via
  relation expansion in a list query. The **only exception** is the resource byte route
  `/{resource}/{id}/file` (download `GET` / upload `POST`), whose bytes come from the integrator's
  `fileResolver` / `uploadResolver` — the skin only moves them.
- **Route pluralization**: route segments are the schema resource name verbatim; no singular/plural
  transformation (per-runtime pluralization rules would inevitably drift).
- **Batch endpoints** (`POST /{resource}/batch`, …): store has `insertMany`/`updateMany`, but how
  their condition/batch semantics map to REST conventions is undecided — not in the spec for now.
- **Structured query language in the query string**: only GQL, transmitted and bound. No second
  filter/sort/page grammar.
- **No caching implementation**: `x-cache` is an annotation only.
- **No authentication**: the adapter never parses auth headers.
- **Resource `_id` is content-addressed** (sha1), only computable by the store's `resourcePut` —
  clients must **not** send `_id` on `POST /Resource` (otherwise content addressing could be forged).
- **`ResourceBinding` (`businessTable` / `businessId`) is metadata only** — it records ownership and
  is **not** an authorization gate; write authorization always goes through `update` / the store's
  permission pipeline.
- The upload parser registration is an **instance-level side effect**; when multiple plugins share
  one instance, mind the registration order.

## FAQ

**Q1. Does store-api authenticate requests?**
No. It parses no auth headers. You supply identity through `contextProvider`; the store's RBAC does
the rest.

**Q2. Why is my `GET /{resource}` list returning only `_id`?**
Because GQL's contract is that omitting the projection returns only `_id`. If you passed a `q` without
a projection, `q` decides the projection entirely — the adapter does not add one. Either write the
projection in `q` or omit `q` (the adapter then generates a full-field projection from the schema
`fields`).

**Q3. Why is `?p.c0={"age":{"$gte":18}}` bound as an object but `?p.x=42` as a number?**
All `p.*` values arrive as strings and are coerced: `true`/`false`/`null` become literals, integer and
float regex matches become numbers, `{`/`[`-prefixed strings are JSON-parsed; otherwise the raw string
is kept.

**Q4. Can I pass a literal in `q`, like `$limit: 10`?**
No. GQL parameter values must be `@name` references; you pass the value through `p.l=10` and write
`$limit: @l`. A literal is rejected by core with `ERR_GQL_PARSE:` → HTTP 400 `GQL_PARSE`.

**Q5. Why do I get 401 `CONTEXT_ERROR`?**
Your `contextProvider` threw a non-permission error (e.g. a JWT failed to parse). That is the only
401 case. A permission denial — or a missing context under `requireContext` — is 403, not 401.

**Q6. Why do I get 501 on `/{resource}/{id}/file`?**
Because the matching resolver was not injected (`fileResolver` for download → `FILE_NOT_CONFIGURED`,
`uploadResolver` for upload → `UPLOAD_NOT_CONFIGURED`). This check happens before any `queryOne`, so
"not configured" is always distinguishable from "record missing" (404).

**Q7. What is the difference between the two 404s on the download route?**
Both are 404 `NOT_FOUND` but different messages: (a) `queryOne` returns empty, or the field is absent
⇒ "record missing / file missing"; (b) the resolver read finds no readable replica
(`ERR_RESOURCE_NOT_FOUND:`) ⇒ the resource object is genuinely gone. A resolver failure that is *not*
that prefix (e.g. `io down`) is **500**, never 404.

**Q8. Does `PATCH` on a non-existent id create it?**
No. `update` is not an upsert here. It affects 0 rows and returns `200` with the store's raw result
(`null` on some runtimes); store-api does not turn that into a 404.

**Q9. Does `DELETE` hard-delete?**
It maps to store's `remove`, which handles archive orchestration inside the store (an `XxxDeleted`
archive mirror). The archive table gets no route.

**Q10. Can I restrict which resources are exposed?**
Yes — pass the `resources` option with an explicit name array. By default the adapter exposes
`list()` minus the archive tables (a name ending in `Deleted` whose base name is also registered).

**Q11. Why is my route `/api/users` 404 while `/api/user` works?**
Route segments are the schema name verbatim; there is no pluralization. Register your schema as
`users` if you want `/api/users`.

**Q12. Is there a cache?**
No. `x-cache` is a pass-through annotation of the host's cache status; without a host cache it is
always `BYPASS`. It is attached to all responses, including errors.

## Development

Run each runtime's own test suite from its directory.

### Node (`node/`)

```bash
cd store-api/node
npm install
npm test          # runs `node --test` over test/*.test.js
```

The smoke and error-matrix suites use a mock store (no real database) and mirror the conformance
cases.

### Python (`py/`)

```bash
cd store-api/py
pip install -e ".[dev]"
pytest            # testpaths = ["tests"]
```

`tests/test_smoke.py` and `tests/test_error_matrix.py` are isomorphic to the Node suites.

### Go (`go/`)

```bash
cd store-api/go
go test ./...     # adapter_test.go + conformance_test.go (consumes ../conformance/cases/*.json)
```

The Go conformance runner directly consumes the shared `../conformance/cases/*.json` files.

### Rust (`rust/`)

```bash
cd store-api/rust
cargo test                       # default features include the rust-store bridge
cargo test --no-default-features # pure-library (mock smoke) form
```

`tests/smoke.rs`, `tests/conformance.rs` and `tests/rust_store_e2e.rs` (real host + real SQLite over
the full HTTP stack) run by default.

### Conformance discipline

Any change to route / param / error semantics must: change `spec/` first, then implement on the
runtimes, then add a case under `conformance/cases/`. Spec, code and cases that disagree are a
defect. See [`conformance/README.md`](./conformance/README.md).

## Related projects

- [`../nodejs-store`](../nodejs-store) — Node host (npm)
- [`../py-store`](../py-store) — Python host (PyPI, `import py_store`)
- `../rust-store` — Rust core + host
- `../go-store` — Go host (SQLite only)
- `store-graphql` / `store-grpc` / `store-mcp` — sibling protocol skins over the same GQL semantics
- `store-gateway` — combiner

## License

MIT
