# store-api-go

**store-api 的 Go 适配器**（store-api-go，与 node/py 同构的第四端）：为 go-store 已注册 schema 自动生成 RESTful 路由。路由/参数/错误/上下文语义以 [`../spec/*.md`](../spec/) 为唯一依据（与 store-api-node / store-api-py 多端 parity，改动先改 spec）。

设计哲学：**REST 只是 GQL 的 HTTP 皮** —— 适配层零语义发明，凡 store 没有的语义本层一律不提供（无 PUT、无子资源路由、无路由复数化）。

## 路由表（spec/01）

| 方法 | 路径 | store 调用 | 成功响应 |
|---|---|---|---|
| GET | `/{prefix}/{resource}` | `Query(gql, params)` | `200 {"data": [...]}` |
| GET | `/{prefix}/{resource}/{id}` | `QueryOne(...)` | `200 {"data": {...}}` |
| POST | `/{prefix}/{resource}` | `Insert(...)` | `201 {"data": <插入结果>}` |
| PATCH | `/{prefix}/{resource}/{id}` | `Update(...)` | `200 {"data": <更新结果>}` |
| DELETE | `/{prefix}/{resource}/{id}` | `Remove(...)` | `200 {"data": <删除结果>}` |

归档表（`XxxDeleted` 且 `Xxx` 已注册）自动不生成路由；`Resources` 选项可显式圈定暴露面。

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
- 判定顺序：适配层守卫(400) → 权限前缀 `ERR_PERMISSION:`(403) → queryOne 空(404) → 其余 500 透传。
- `update` 影响 0 行返回 `200 {"data": null}`（中性返回不当错误）。
- 权限错误按 core 稳定前缀判定（`ERR_PERMISSION:`），禁按文案匹配。

## 已知 spec 缺口（如实记录）

spec/03 表格中「GQL 解析失败 ⇒ 400」在现有 node/py 实现中同样未落地（无 code 的 store 错误统一 500）。Go 版与 node 行为保持一致（多端 parity 优先），如需落 spec 需三端同步补判定 + conformance 用例，属独立改动。

## 相关仓库

- `../../go-store`：被适配的 Go 宿主（独立仓库，module 名与目录解耦，可独立发版）
- `../spec` / `../conformance`：共享契约与用例；`../node` / `../py`：node/py 适配器
- Go 端 conformance runner：`conformance_test.go` 直接消费 `../conformance/cases/*.json`
