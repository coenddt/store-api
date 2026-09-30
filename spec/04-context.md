# 04 — 请求上下文 / RBAC 注入

store 的 RBAC 以请求上下文为基准（`store.setContext(ctx)` / `store.set_context(ctx)`）。store-api 自身**不做认证**，只提供把 HTTP 请求翻译成上下文的钩子。

## 适配器选项

| 选项 | 类型 | 语义 |
|---|---|---|
| `contextProvider` | `(req) => ctx \| Promise<ctx>`（Node）/ `(request) => ctx \| awaitable`（Python） | 每请求调用；返回值作为本请求的 store 上下文。返回 `null`/`undefined` 时必须**显式注入空上下文**（Rust 适配器 `set_context(null)` 语义为清除；有状态持有的运行时禁止残留上一请求上下文，防身份跨请求泄漏），由 store 自身的 `requireContext` 档位决定是否拒绝。 |

## 行为

1. 每个请求进入 handler 前，若配置了 `contextProvider`，调用它并将结果经 `store.setContext(ctx)`（Node）/ `store.set_context(ctx)`（Python）注入。
2. `contextProvider` 抛出的错误按 spec/03-errors.md 的判定顺序分类：`PermissionError` ⇒ `403`（RBAC 拒绝，与业务 handler 中的权限拒绝同一语义）；其余错误视为**认证/上下文构造失败** ⇒ `401`，code 取 `CONTEXT_ERROR`，message 原样透传。这是唯一以 401 呈现的场景；store RBAC 拒绝（上下文已设置但权限不足）同样是 403（见 03-errors）。
3. 适配器不解析任何认证头（Authorization / Cookie / API-Key 一概不碰）——身份来源完全由 `contextProvider` 决定，接入方自行接 JWT / Session 等。
4. store 的 `requireContext` 档位（`store.setRequireContext(true)`）由接入方自行设置；开启后未注入上下文的请求会被 store 以 `PermissionError` 拒绝，按 403 映射，适配层不加第二层判断。

## 并发说明

store 上下文是进程级/任务级状态，双端框架的并发模型不同（Node 单线程事件循环 / Python asyncio），适配层不做额外的上下文隔离机制——上下文的生命周期安全性由 store 自身保证，适配层不发明包装。
