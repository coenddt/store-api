# store-api-go

**store-api 的 Go 适配器**（store-api-go，与 node/py 同构的第四端）：为 go-store 已注册 schema 自动生成 RESTful 路由。路由/参数/错误/上下文语义以 [`../spec/*.md`](../spec/) 为唯一依据（与 store-api-node / store-api-py 多端 parity，改动先改 spec）。

设计哲学：**REST 只是 GQL 的 HTTP 皮** —— 适配层零语义发明，凡 store 没有的语义本层一律不提供（无 PUT、无其它子资源路由、无路由复数化；`/file` 子路由是 spec 明文规定的唯一例外，由 resolver 注入字节能力）。

## 路由表（spec/01）

| 方法 | 路径 | store 调用 | 成功响应 |
|---|---|---|---|
| GET | `/{prefix}/{resource}` | `Query(gql, params)` | `200 {"data": [...]}` |
| GET | `/{prefix}/{resource}/{id}` | `QueryOne(...)` | `200 {"data": {...}}` |
| POST | `/{prefix}/{resource}` | `Insert(...)` | `201 {"data": <插入结果>}` |
| PATCH | `/{prefix}/{resource}/{id}` | `Update(...)` | `200 {"data": <更新结果>}` |
| DELETE | `/{prefix}/{resource}/{id}` | `Remove(...)` | `200 {"data": <删除结果>}` |
| GET | `/{prefix}/{resource}/{id}/file` | `QueryOne(...)` → `FileResolver` | `200` 字节流 |
| POST | `/{prefix}/{resource}/{id}/file` | `QueryOne(...)` → `UploadResolver` → `Update(...)` | `200 {"data": <更新结果>}` |

归档表（`XxxDeleted` 且 `Xxx` 已注册）自动不生成路由；`Resources` 选项可显式圈定暴露面。

## file 子路由（spec/01/02/03/05）

字节能力由接入方注入，适配层不碰存储：

```go
storeapi.New(mux, store, storeapi.Options{
    Prefix:      "/api",
    FileField:   "file", // ?field= 缺省值
    UploadLimit: 32 << 20,
    FileResolver: func(r *http.Request, rec map[string]any, id string) (*storeapi.FileOut, error) {
        return &storeapi.FileOut{Body: bytes, ContentType: "image/png", FileName: "a.png"}, nil
    },
    UploadResolver: func(r *http.Request, rec map[string]any, id string) (*storeapi.UploadOut, error) {
        return &storeapi.UploadOut{Ref: "s3://bucket/key"}, nil
    },
})
```

判定顺序（与 node/rust 逐档一致）：未注入 resolver `501` → 空体 `400 EMPTY_BODY` / 超限 `413 UPLOAD_TOO_LARGE`
→ `?field=` 不在可写白名单 `400 INVALID_PARAM` → queryOne 空 `404` → 字段空 `404` → resolver 错误
（`ERR_RESOURCE_NOT_FOUND:` ⇒ `404 NOT_FOUND`）→ `200`。上传回写走 `update`，RBAC 写权限自动生效。

`?field=` 白名单由 schema 字段树生成：只允许标量 `string` / `file`（`object` 下钻为 `a.b`），
排除 `_id`、`$` 前缀与原型污染段；下载（读）不受白名单限制。
点路径回写在 go 端会还原为嵌套对象后整体写回（core SQL 方言只接受列级键）。

## 查询能力全部来自 GQL（spec/02）

`q` 是资源头之后的 GQL 片段（条件 / 排序 / 分页 / 关系展开 / 聚合），`p.<name>` 绑定 `@<name>` 命名参数（标量自动转型；对象/数组给合法 JSON；重复出现成数组）。

## 快速上手

```go
package main

import (
	"net/http"

	gostore "github.com/coenddt/go-store"
	storeapi "github.com/coenddt/store-api-go"
)

func main() {
	store, _ := gostore.Open("sqlite://app.db")
	// store.Register(...) + store.EnsureTables(...) 先行（spec：注册完再挂路由）

	mux := http.NewServeMux()
	_ = storeapi.New(mux, store, storeapi.Options{
		Prefix:   "/api",
		IDField:  "_id",
		ContextProvider: func(r *http.Request) (*gostore.Context, error) {
			// 身份来源完全由接入方决定（spec/04：适配器不碰任何认证头）
			token := r.Header.Get("X-User")
			if token == "" {
				return nil, nil // 显式空上下文
			}
			return &gostore.Context{UserID: token, Roles: []string{"admin"}}, nil
		},
	})
	http.ListenAndServe(":3000", mux)
	// GET /api/user?q=($condition: @c0){ name }  &  p.c0={"age":{"$gte":18}}
}
```

标准库 `net/http` 实现，零框架依赖。

## 错误映射（spec/03，no-error-masking）

- 统一形态 `{"error":{"code":..., "message":<store 原文，原样透传>}}`；message 取不到置 null，禁伪造。
- 判定顺序：适配层守卫（400 / 404 / 413 / 501）→ 权限前缀 `ERR_PERMISSION:`（403）→
  `ERR_NO_CONTEXT:`（403）→ `ERR_GQL_PARSE:`（400）→ queryOne 空（404）→ `ERR_NOT_FOUND:` 写探针未命中（404）→
  `ERR_RESOURCE_NOT_FOUND:` resolver 零副本（404）→ 其余 500 透传。
- `update` 影响 0 行返回 `200 {"data": null}`（中性返回不当错误）。
- 权限错误按 core 稳定前缀判定（`ERR_PERMISSION:`），禁按文案匹配。

## REST 分页正确用法

core 的 GQL 语法中**参数值一律是 `@name` 命名引用**（py-store/nodejs-store 同构），不接受字面量。分页写法：

```
GET /api/user?q=($limit: @l, $skip: @s)&p.l=10&p.s=20
```

（spec/02 的示例已按此更正，v1 修订。）


## 错误映射补充：GQL 解析失败

GQL 解析失败（core 稳定前缀 `ERR_GQL_PARSE:`，spec/03 v1 修订）→ `400 {"error":{"code":"GQL_PARSE","message":<core 原文>}}`。此缺口曾在 node/py 实现中未落地（统一 500），现已四端同步修复并由 conformance 用例锁定。

## 相关仓库

- `../../go-store`：被适配的 Go 宿主（独立仓库，module 名与目录解耦，可独立发版）
- `../spec` / `../conformance`：共享契约与用例；`../node` / `../py`：node/py 适配器
- Go 端 conformance runner：`conformance_test.go` 直接消费 `../conformance/cases/*.json`（含用例级 `opts` 装配 mock resolver 与 `raw:<bytes>` 字节体约定）
