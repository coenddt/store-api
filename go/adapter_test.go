// adapter_test.go —— store-api-go 端到端测试：真实 SQLite 落库 + REST 全链路。
// 场景覆盖 spec 路由表/参数映射/错误映射/上下文注入，与 store-api 的 conformance 契约对齐。
package storeapi

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"

	"github.com/coenddt/go-store"
)

// TestWriteErrorNotFoundIs404 锁定 spec/03 v2 收口：写探针未命中（ERR_NOT_FOUND:）⇒ 404 + not_found。
func TestWriteErrorNotFoundIs404(t *testing.T) {
	rec := httptest.NewRecorder()
	writeError(rec, errors.New("ERR_NOT_FOUND:目标记录不存在"))
	if rec.Code != http.StatusNotFound {
		t.Fatalf("ERR_NOT_FOUND 应 404, got %d", rec.Code)
	}
	var body map[string]any
	if err := json.Unmarshal(rec.Body.Bytes(), &body); err != nil {
		t.Fatalf("响应非 JSON: %v", err)
	}
	errObj, _ := body["error"].(map[string]any)
	if errObj["code"] != "not_found" || errObj["message"] != "ERR_NOT_FOUND:目标记录不存在" {
		t.Fatalf("code/message 不符: %v", body)
	}
}

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

	// GQL 解析失败 → 400 GQL_PARSE（spec/03 v1 修订已四端落地：core ERR_GQL_PARSE 稳定前缀）
	badQuery := url.Values{}
	badQuery.Set("q", "($bogus")
	code, body = doJSON(t, "GET", srv.URL+"/api/order?"+badQuery.Encode(), "")
	if code != 400 {
		t.Fatalf("GQL 解析失败应 400, got %d: %v", code, body)
	}
	errObj, ok := body["error"].(map[string]any)
	if !ok || errObj["code"] != "GQL_PARSE" || errObj["message"] == nil {
		t.Fatalf("应为 GQL_PARSE + 原文: %v", body)
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
	srv, st := freshServer(t, Options{Prefix: "/api", ContextProvider: provider})

	// 定性依据：core permission::evaluate 对「schema 未配置 write 白名单」的默认姿态是
	// UnconfiguredPolicy::Open（permission.rs：role_list 缺失 ⇒ 返回 unconfigured == Open），
	// 且 spec/04 明确「适配层不加第二层判断」（RBAC 由 store 判决）。因此接口层 201 符合契约，
	// 原用例缺的是「拒写角色」配置——与 rust 端同族用例 rust_store_e2e.rs:31
	// `host.set_deny_write_roles(vec!["guest"])` 同构，此处显式补上（非改断言）。
	if err := st.SetDenyWriteRoles([]string{"guest"}); err != nil {
		t.Fatalf("SetDenyWriteRoles: %v", err)
	}

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

	// 对照：admin 写仍 201（证明 403 来自 deny_write_roles，而非接口层整体拒写）
	req, _ = http.NewRequest("POST", srv.URL+"/api/order", strings.NewReader(`{"status":"s","amount":1}`))
	req.Header.Set("X-Role", "admin")
	resp, err = http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("POST admin: %v", err)
	}
	_ = json.NewDecoder(resp.Body).Decode(&out)
	resp.Body.Close()
	if resp.StatusCode != 201 {
		t.Fatalf("admin 写应 201, got %d: %v", resp.StatusCode, out)
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

// freshFileServer 起一个带资源引用字段（file / images.full）的 REST 服务。
func freshFileServer(t *testing.T, opts Options) *httptest.Server {
	t.Helper()
	st, err := gostore.Open("sqlite::memory:")
	if err != nil {
		t.Fatalf("Open: %v", err)
	}
	t.Cleanup(func() { st.Close() })
	schema := map[string]any{
		"name": "asset", "collection": "assets", "idPrefix": "a",
		"fields": map[string]any{
			"title": map[string]any{"type": "string"},
			"file":  map[string]any{"type": "string"},
			"images": map[string]any{
				"type":   "object",
				"fields": map[string]any{"full": map[string]any{"type": "string"}},
			},
		},
	}
	if err := st.Register(schema); err != nil {
		t.Fatalf("Register: %v", err)
	}
	if err := st.EnsureTables(schema); err != nil {
		t.Fatalf("EnsureTables: %v", err)
	}
	mux := http.NewServeMux()
	if err := New(mux, st, opts); err != nil {
		t.Fatalf("New: %v", err)
	}
	srv := httptest.NewServer(mux)
	t.Cleanup(srv.Close)
	return srv
}

func doBytes(t *testing.T, method, rawURL string, body []byte) (*http.Response, []byte) {
	t.Helper()
	req, _ := http.NewRequest(method, rawURL, strings.NewReader(string(body)))
	req.Header.Set("Content-Type", "application/octet-stream")
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("%s %s: %v", method, rawURL, err)
	}
	defer resp.Body.Close()
	raw, _ := io.ReadAll(resp.Body)
	return resp, raw
}

// TestFileRouteErrorLadder 锁定 file 子路由的错误档位（spec/03 判定顺序）：
// 501 → 413 / 400 EMPTY_BODY → 400 INVALID_PARAM → 404。
func TestFileRouteErrorLadder(t *testing.T) {
	// 档 1：未注入 resolver ⇒ 501（先于请求体判定）
	unconfigured := freshFileServer(t, Options{Prefix: "/api"})
	code, body := doJSON(t, "POST", unconfigured.URL+"/api/asset", `{"title":"t"}`)
	if code != 201 {
		t.Fatalf("前置建记录: %d %v", code, body)
	}
	id, _ := body["data"].(map[string]any)["_id"].(string)
	resp, raw := doBytes(t, "GET", unconfigured.URL+"/api/asset/"+id+"/file", nil)
	if resp.StatusCode != 501 || !strings.Contains(string(raw), "FILE_NOT_CONFIGURED") {
		t.Fatalf("未注入 fileResolver 应 501 FILE_NOT_CONFIGURED: %d %s", resp.StatusCode, raw)
	}
	resp, raw = doBytes(t, "POST", unconfigured.URL+"/api/asset/"+id+"/file", []byte("x"))
	if resp.StatusCode != 501 || !strings.Contains(string(raw), "UPLOAD_NOT_CONFIGURED") {
		t.Fatalf("未注入 uploadResolver 应 501 UPLOAD_NOT_CONFIGURED: %d %s", resp.StatusCode, raw)
	}

	// 档 2~5：注入 resolver，上限 4 字节
	srv := freshFileServer(t, Options{
		Prefix:      "/api",
		UploadLimit: 4,
		FileResolver: func(_ *http.Request, _ map[string]any, _ string) (*FileOut, error) {
			return &FileOut{Body: []byte("mock-bytes")}, nil
		},
		UploadResolver: func(_ *http.Request, _ map[string]any, _ string) (*UploadOut, error) {
			return &UploadOut{Ref: "ref://mock"}, nil
		},
	})
	code, body = doJSON(t, "POST", srv.URL+"/api/asset", `{"title":"t"}`)
	if code != 201 {
		t.Fatalf("前置建记录: %d %v", code, body)
	}
	id, _ = body["data"].(map[string]any)["_id"].(string)

	resp, raw = doBytes(t, "POST", srv.URL+"/api/asset/"+id+"/file", nil)
	if resp.StatusCode != 400 || !strings.Contains(string(raw), "EMPTY_BODY") {
		t.Fatalf("空体应 400 EMPTY_BODY: %d %s", resp.StatusCode, raw)
	}
	resp, raw = doBytes(t, "POST", srv.URL+"/api/asset/"+id+"/file", []byte("12345"))
	if resp.StatusCode != 413 || !strings.Contains(string(raw), "UPLOAD_TOO_LARGE") {
		t.Fatalf("超限应 413 UPLOAD_TOO_LARGE: %d %s", resp.StatusCode, raw)
	}
	resp, raw = doBytes(t, "POST", srv.URL+"/api/asset/"+id+"/file?field=nope", []byte("12"))
	if resp.StatusCode != 400 || !strings.Contains(string(raw), "INVALID_PARAM") {
		t.Fatalf("白名单外字段应 400 INVALID_PARAM: %d %s", resp.StatusCode, raw)
	}
	resp, raw = doBytes(t, "POST", srv.URL+"/api/asset/nope/file", []byte("12"))
	if resp.StatusCode != 404 || !strings.Contains(string(raw), "NOT_FOUND") {
		t.Fatalf("记录不存在应 404 NOT_FOUND: %d %s", resp.StatusCode, raw)
	}

	// 成功下载：字节流 + content-type/content-disposition 缺省值（spec/05）
	code, body = doJSON(t, "POST", srv.URL+"/api/asset", `{"title":"t","file":"ref://x"}`)
	if code != 201 {
		t.Fatalf("前置建记录: %d %v", code, body)
	}
	id, _ = body["data"].(map[string]any)["_id"].(string)
	resp, raw = doBytes(t, "GET", srv.URL+"/api/asset/"+id+"/file", nil)
	if resp.StatusCode != 200 || string(raw) != "mock-bytes" {
		t.Fatalf("下载应 200 + 原始字节: %d %s", resp.StatusCode, raw)
	}
	if ct := resp.Header.Get("Content-Type"); ct != "application/octet-stream" {
		t.Fatalf("缺省 content-type = %q", ct)
	}
	if cd := resp.Header.Get("Content-Disposition"); cd != `attachment; filename="file"` {
		t.Fatalf("缺省 content-disposition = %q", cd)
	}

	// 字段为空 ⇒ 404（不调用 resolver）
	code, body = doJSON(t, "POST", srv.URL+"/api/asset", `{"title":"t"}`)
	if code != 201 {
		t.Fatalf("前置建记录: %d %v", code, body)
	}
	id, _ = body["data"].(map[string]any)["_id"].(string)
	resp, raw = doBytes(t, "GET", srv.URL+"/api/asset/"+id+"/file", nil)
	if resp.StatusCode != 404 || !strings.Contains(string(raw), "NOT_FOUND") {
		t.Fatalf("字段为空应 404 NOT_FOUND: %d %s", resp.StatusCode, raw)
	}
}

// TestFileUploadDotPathWriteback 第二轮：点路径回写与下载（go 宿主方言差异的等价性验证）。
// node/py/rust 用点路径字面键 { "images.full": ref } 交由宿主按点路径写入；go-store 的
// core SQL 方言只认列级键，故适配层展开为 { images: { full: ref } } 整列回写，必须以
// rec 为基底保留同级字段。本用例锁定「结果等价」这一契约（spec/01 只规定回写后记录，
// 不规定键形态）。
func TestFileUploadDotPathWriteback(t *testing.T) {
	srv := freshFileServer(t, Options{
		Prefix: "/api",
		UploadResolver: func(_ *http.Request, _ map[string]any, _ string) (*UploadOut, error) {
			return &UploadOut{Ref: "ref://full-new"}, nil
		},
		FileResolver: func(_ *http.Request, rec map[string]any, _ string) (*FileOut, error) {
			imgs, _ := rec["images"].(map[string]any)
			full, _ := imgs["full"].(string)
			return &FileOut{
				Body:        []byte(full),
				ContentType: "text/plain",
				FileName:    "a\r\nX-Injected: 1.txt",
			}, nil
		},
	})

	code, body := doJSON(t, "POST", srv.URL+"/api/asset", `{"title":"t","images":{"thumb":"ref://thumb-old"}}`)
	if code != 201 {
		t.Fatalf("前置建记录: %d %v", code, body)
	}
	id, _ := body["data"].(map[string]any)["_id"].(string)

	code, body = doJSON(t, "POST", srv.URL+"/api/asset/"+id+"/file?field=images.full", `bytes`)
	if code != 200 {
		t.Fatalf("点路径上传应 200: %d %v", code, body)
	}
	imgs, _ := body["data"].(map[string]any)["images"].(map[string]any)
	if imgs["full"] != "ref://full-new" {
		t.Fatalf("点路径未写入: %v", imgs)
	}
	if imgs["thumb"] != "ref://thumb-old" {
		t.Fatalf("同级字段被覆盖丢失（与三端点路径回写不等价）: %v", imgs)
	}

	// 下载侧：下载不受白名单限制，且 fileName 的 CR/LF 必须清洗（防响应头注入）
	resp, raw := doBytes(t, "GET", srv.URL+"/api/asset/"+id+"/file?field=images.full", nil)
	if resp.StatusCode != 200 || string(raw) != "ref://full-new" {
		t.Fatalf("点路径下载应 200 + 引用值: %d %s", resp.StatusCode, raw)
	}
	cd := resp.Header.Get("Content-Disposition")
	if strings.ContainsAny(cd, "\r\n") || cd != `attachment; filename="a  X-Injected: 1.txt"` {
		t.Fatalf("content-disposition 未清洗 CR/LF: %q", cd)
	}
	if resp.Header.Get("X-Injected") != "" {
		t.Fatalf("响应头被注入")
	}

	// F5 取证：未注册资源路由由 ServeMux 兜底（非 JSON RESOURCE_NOT_FOUND）
	resp, raw = doBytes(t, "GET", srv.URL+"/api/unknown/1/file", nil)
	t.Logf("未注册资源路径: HTTP %d body=%q（spec/03 L20 为防御性条目，go 路由表静态生成）", resp.StatusCode, strings.TrimSpace(string(raw)))
}
