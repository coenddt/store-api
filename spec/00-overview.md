# 00 — 总览：定位与分层

## 定位

store-api 是 common-store 数据层家族之上的 **HTTP 适配层**，不是新的数据层，也不做认证、限流、缓存等任何超出「HTTP ↔ store 调用翻译」的事。

## 分层

```
HTTP 客户端
   │  RESTful (JSON)
store-api（本仓库：fastify 适配器 / fastapi 适配器）
   │  store 原生 API：query / queryOne / insert / update / remove + setContext
数据层（nodejs-store / py-store，未来 rust 宿主）
   │  GQL → 命令规划
rust-store core（解析 / 权限 / 方言）
   │
数据库（MongoDB / MySQL / PostgreSQL / SQLite）
```

## 三条铁律

1. **零语义发明**：路由、参数、错误、权限语义必须能一一对应到 store 已有语义；store 没有的语义本层不提供。无法对应的设计不允许进 spec。
2. **双端 parity 先行**：任何变更先改 spec，再双端同步实现，再补 conformance 用例。规范、代码、用例三者不一致即为缺陷。
3. **禁掩盖错误**（遵循项目 `no-error-masking` 准绳）：store 抛出的任何错误必须原样映射为带语义的 HTTP 错误响应，禁止静默吞掉、禁止回退默认值、禁止把中性结果当错误。成功响应中不得出现错误文案字段。
