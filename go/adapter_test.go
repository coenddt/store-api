// adapter_test.go —— store-api-go 端到端测试：真实 SQLite 落库 + REST 全链路。
// 场景覆盖 spec 路由表/参数映射/错误映射/上下文注入，与 store-api 的 conformance 契约对齐。
package storeapi

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"

	"github.com/coenddt/go-store"
)

// freshServer 起一个真实 SQLite（内存库）+ 已注册 schema 的 REST 服务。
func freshServer(t *testing.T, opts Options) (*httptest.Server, *gostore.Store) {
	t.Helper()
	st, err := gostore.Open("sqlite::memory:")
	if err != nil {
		t.Fatalf("Open: %v", err)
	}
	t.Cleanup(func() { st.Close() })

	orderSchema := map[string]any{
		"name":       "order",
		"collection": "orders",
		"idPrefix":   "o",
		"fields": map[string]any{
			"status": map[string]any{"type": "string"},
			"amount": map[string]any{"type": "number"},
			"owner":  map[string]any{"type": "string"},
		},
		"computes": map[string]any{
			"statusUpper": map[string]any{"type": "string", "fn": true, "fnRef": "goUpper"},
		},
	}
	if err := st.Register(orderSchema); err != nil {
		t.Fatalf("Register: %v", err)
	}
	if err := st.EnsureTables(orderSchema); err != nil {
		t.Fatalf("EnsureTables: %v", err)
	}
	st.RegisterSyncCompute("goUpper", func(ctx *gostore.Context, doc map[string]any) (any, error) {
		v, _ := doc["status"].(string)
		return v + "!", nil
	})

	mux := http.NewServeMux()
	if err := New(mux, st, opts); err != nil {
		t.Fatalf("New: %v", err)
	}
	srv := httptest.NewServer(mux)
	t.Cleanup(srv.Close)
	return srv, st
}

func doJSON(t *testing.T, method, rawURL, body string) (int, map[string]any) {
	t.Helper()
	var req *http.Request
	if body == "" {
		req, _ = http.NewRequest(method, rawURL, nil)
	} else {
		req, _ = http.NewRequest(method, rawURL, strings.NewReader(body))
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("%s %s: %v", method, rawURL, err)
	}
	defer resp.Body.Close()
	var out map[string]any
	_ = json.NewDecoder(resp.Body).Decode(&out)
	return resp.StatusCode, out
}

func TestRESTFullCRUD(t *testing.T) {
	srv, _ := freshServer(t, Options{Prefix: "/api"})
	ctx := context.Background()
	_ = ctx

	// POST → 201 + 计算列不在写路径
	code, body := doJSON(t, "POST", srv.URL+"/api/order",
		`{"status":"paid","amount":2.5,"owner":"u1"}`)
	if code != 201 {
		t.Fatalf("POST 应 201, got %d: %v", code, body)
	}
	id, _ := body["data"].(map[string]any)["_id"].(string)
	if id == "" {
		t.Fatalf("POST 返回缺 _id: %v", body)
	}

	// GET 列表（q 条件 + p.* 绑定）→ 计算列在出参
	listQuery := url.Values{}
	listQuery.Set("q", "($condition: @c0){ status, statusUpper, amount, owner }")
	listQuery.Set("p.c0", `{"amount":{"$gte":1}}`)
	code, body = doJSON(t, "GET", srv.URL+"/api/order?"+listQuery.Encode(), "")
	if code != 200 {
		t.Fatalf("GET 列表应 200, got %d: %v", code, body)
	}
	rows, _ := body["data"].([]any)
	if len(rows) != 1 {
		t.Fatalf("列表应 1 行: %v", body)
	}
	r0, _ := rows[0].(map[string]any)
	if r0["statusUpper"] != "paid!" {
		t.Fatalf("计算列 statusUpper = %v", r0["statusUpper"])
	}

	// GET 单条（主键路由）
	code, body = doJSON(t, "GET", srv.URL+"/api/order/"+id, "")
	if code != 200 || body["data"].(map[string]any)["_id"] != id {
		t.Fatalf("GET 单条: %d %v", code, body)
	}

	// GET 单条未命中 → 404 NOT_FOUND
	code, body = doJSON(t, "GET", srv.URL+"/api/order/nope", "")
	if code != 404 {
		t.Fatalf("未命中应 404, got %d: %v", code, body)
	}
	errObj, _ := body["error"].(map[string]any)
	if errObj["code"] != "NOT_FOUND" {
		t.Fatalf("404 code = %v", errObj["code"])
	}

	// PATCH 部分更新
	code, body = doJSON(t, "PATCH", srv.URL+"/api/order/"+id, `{"status":"shipped"}`)
	if code != 200 || body["data"].(map[string]any)["status"] != "shipped" {
		t.Fatalf("PATCH: %d %v", code, body)
	}

	// PATCH 未命中 → 200 + data:null（spec/03：0 行不算错误）
	code, body = doJSON(t, "PATCH", srv.URL+"/api/order/nope", `{"status":"x"}`)
	if code != 200 || body["data"] != nil {
		t.Fatalf("PATCH 未命中应 200+null: %d %v", code, body)
	}

	// DELETE → 归档 + 删除计数
	code, body = doJSON(t, "DELETE", srv.URL+"/api/order/"+id, "")
	if code != 200 {
		t.Fatalf("DELETE 应 200: %d %v", code, body)
	}
	data, _ := body["data"].(map[string]any)
	if data["deletedCount"] != float64(1) || data["archivedCount"] != float64(1) {
		t.Fatalf("DELETE 计数: %v", data)
	}

	// GQL 解析失败：spec/03 表格写 400，但 node/py 适配器实现中无 code 的 store
	// 错误统一 500（errors.js mapError 无 parse 判定分支）——spec 与实现存在既有缺口。
	// Go 版与 node 行为保持一致（多端 parity 优先）：500 + store 原始 message 透传。
	badQuery := url.Values{}
	badQuery.Set("q", "($bogus")
	code, body = doJSON(t, "GET", srv.URL+"/api/order?"+badQuery.Encode(), "")
	if code != 500 {
		t.Fatalf("GQL 错误应与 node 一致取 500, got %d: %v", code, body)
	}
	errObj, ok := body["error"].(map[string]any)
	if !ok || errObj["message"] == nil {
		t.Fatalf("错误形态缺失 error 对象/message: %v", body)
	}
}

func TestRESTParamErrors(t *testing.T) {
	srv, _ := freshServer(t, Options{Prefix: "/api"})

	// p.* 非法 JSON → 400 INVALID_PARAM（禁静默当字符串）
	badParam := url.Values{}
	badParam.Set("q", "($condition: @c0)")
	badParam.Set("p.c0", "{bad")
	code, body := doJSON(t, "GET", srv.URL+"/api/order?"+badParam.Encode(), "")
	if code != 400 {
		t.Fatalf("非法参数应 400, got %d", code)
	}
	errObj, _ := body["error"].(map[string]any)
	if errObj["code"] != "INVALID_PARAM" {
		t.Fatalf("code = %v", errObj["code"])
	}

	// POST 非 JSON 对象 → 400 INVALID_BODY
	code, body = doJSON(t, "POST", srv.URL+"/api/order", `[1,2]`)
	if code != 400 {
		t.Fatalf("数组体应 400, got %d", code)
	}
	errObj, _ = body["error"].(map[string]any)
	if errObj["code"] != "INVALID_BODY" {
		t.Fatalf("code = %v", errObj["code"])
	}
}

func TestRESTContextAndPermission(t *testing.T) {
	// provider 把 header X-User 映射为角色；guest 写 → 403
	provider := func(r *http.Request) (*gostore.Context, error) {
		role := r.Header.Get("X-Role")
		if role == "" {
			return nil, errSimulated("缺少角色")
		}
		return &gostore.Context{UserID: "u-" + role, Roles: []string{role}}, nil
	}
	srv, _ := freshServer(t, Options{Prefix: "/api", ContextProvider: provider})

	// 无角色 → provider 错误 → 401 CONTEXT_ERROR
	code, body := doJSON(t, "GET", srv.URL+"/api/order", "")
	if code != 401 {
		t.Fatalf("缺上下文应 401, got %d: %v", code, body)
	}
	errObj, _ := body["error"].(map[string]any)
	if errObj["code"] != "CONTEXT_ERROR" {
		t.Fatalf("code = %v", errObj["code"])
	}

	// guest POST → store 权限拒绝 → 403
	req, _ := http.NewRequest("POST", srv.URL+"/api/order", strings.NewReader(`{"status":"s","amount":1}`))
	req.Header.Set("X-Role", "guest")
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("POST: %v", err)
	}
	var out map[string]any
	_ = json.NewDecoder(resp.Body).Decode(&out)
	resp.Body.Close()
	if resp.StatusCode != 403 {
		t.Fatalf("guest 写应 403, got %d: %v", resp.StatusCode, out)
	}
	errObj, _ = out["error"].(map[string]any)
	if errObj == nil || errObj["message"] == nil {
		t.Fatalf("403 应透传 store 原始错误: %v", out)
	}
}

func TestFilterArchived(t *testing.T) {
	got := FilterArchived([]string{"order", "orderDeleted", "user", "userTag"})
	if len(got) != 3 {
		t.Fatalf("归档过滤: %v", got)
	}
	for _, n := range got {
		if n == "orderDeleted" {
			t.Fatalf("orderDeleted 应被过滤")
		}
	}
}

type errSimulated string

func (e errSimulated) Error() string { return string(e) }
