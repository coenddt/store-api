# 00 — 总览：定位与分层

## 定位

store-api 是 common-store 数据层家族之上的 **HTTP 适配层**，不是新的数据层，也不做认证、限流、缓存**实现**等任何超出「HTTP ↔ store 调用翻译」的事（`x-cache` 仅为宿主缓存状态注记，见 `05-response-headers.md`）。

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

## 例外登记（有意突破，须留痕）

- **`/{resource}/{id}/file` 资源字节读写（`GET` 下载 / `POST` 上传）**：突破两处——
  1. `01-routing.md`「子资源路由 v1 不提供」：本路由族属 `/{resource}/{id}/<子资源>` 形态的子资源路由，为**唯一例外**；
  2. 铁律 1「零语义发明」：store 无「文件原语」。
- **理由与边界**：字节来源由接入方注入钩子 `fileResolver`（下载）/ `uploadResolver`（上传）提供，皮**不**做存储 IO、**不**拼签名 URL、**不**判权限；
  - 上传语义 = 「读记录（`queryOne`）→ 调 `uploadResolver` → 回写字段（`update`）」，写操作天然走 `update` 语义，RBAC 写权限自动生效；
  - 上传字节体由皮注册的 `application/octet-stream` 缓冲 parser 承接，上限由插件选项 `uploadLimit` 控制（默认 32MB，超限 413）；该 parser 注册为**实例级副作用**，同实例多插件共存须注意注册顺序。
  - **未注入 resolver 一律显式拒绝**（上传 `UPLOAD_NOT_CONFIGURED` / 下载 `FILE_NOT_CONFIGURED`，均 501），**不兜底、不静默**（对齐铁律 3「禁掩盖错误」）。
- **资源主键约束**：`Resource._id` = 内容哈希（sha1），**只能由 store 的 `resourcePut` 按内容计算**，禁止客户端经 `POST /Resource` 自带 `_id`（否则内容寻址可被伪造）。
- **绑定为元数据**：`ResourceBinding`（`businessTable` / `businessId`）**只记录归属关系，不构成鉴权门禁**；写鉴权一律由 `update` 走 store 权限管道判定。
- 其余子资源路由（`/{resource}/{id}/{relation}`）与 `PUT` 仍不提供。
