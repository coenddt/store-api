// conformance_test.go —— Go 端 conformance runner：直接消费 ../store-api/conformance/cases/*.json
// 执行并断言 HTTP 行为，与 node/py 两端同一份用例（语义对齐 v0，见 conformance/README.md：
// 「语义变更必须先改 spec，再三端实现，再补用例——三者不一致即为缺陷」）。
package storeapi

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/coenddt/go-store"
)

// conformanceCase 与 cases/*.json 的格式契约一致。
type conformanceCase struct {
	Name     string            `json:"name"`
	Resource string            `json:"resource"`
	Opts     conformanceOpts   `json:"opts"`
	Steps    []conformanceStep `json:"steps"`
}

// conformanceOpts 用例级适配器选项（conformance/README.md：runner 据此装配 mock resolver）。
type conformanceOpts struct {
	FileResolver         bool `json:"fileResolver"`
	UploadResolver       bool `json:"uploadResolver"`
	NoFileResolver       bool `json:"noFileResolver"`
	NoUploadResolver     bool `json:"noUploadResolver"`
	FileResolverNotFound bool `json:"fileResolverNotFound"`
}

type conformanceStep struct {
	Method       string            `json:"method"`
	Path         string            `json:"path"`
	Body         json.RawMessage   `json:"body"`
	Headers      map[string]string `json:"headers"`
	UseCreatedID bool              `json:"useCreatedId"`
	Expect       struct {
		Status    int      `json:"status"`
		ErrorCode string   `json:"errorCode"`
		DataKeys  []string `json:"dataKeys"`
	} `json:"expect"`
}

// TestConformanceCases 加载 cases 目录全部用例，对真实 SQLite（内存库）+ REST 全链路执行。
func TestConformanceCases(t *testing.T) {
	casesDir := filepath.Join("..", "conformance", "cases")
	entries, err := os.ReadDir(casesDir)
	if err != nil {
		t.Skipf("conformance 用例目录不可达（%v）；go-store 单仓验证不受影响", err)
	}

	for _, e := range entries {
		if !strings.HasSuffix(e.Name(), ".json") {
			continue
		}
		t.Run(e.Name(), func(t *testing.T) {
			raw, err := os.ReadFile(filepath.Join(casesDir, e.Name()))
			if err != nil {
				t.Fatalf("读用例失败: %v", err)
			}
			var c conformanceCase
			if err := json.Unmarshal(raw, &c); err != nil {
				t.Fatalf("用例 JSON 解析失败: %v", err)
			}
			runConformance(t, &c)
		})
	}
}

func runConformance(t *testing.T, c *conformanceCase) {
	t.Helper()

	// 真实 store + schema（用例名 users-crud 对应 user schema；新用例按 resource 映射补 schema）
	st, err := gostore.Open("sqlite::memory:")
	if err != nil {
		t.Fatalf("Open: %v", err)
	}
	defer st.Close()
	// `file` 承载 file 子路由的资源引用；`images.full` 承载 ?field= 点路径回写（spec/02）
	fields := map[string]any{
		"name": map[string]any{"type": "string"},
		"age":  map[string]any{"type": "number"},
		"file": map[string]any{"type": "string"},
		"images": map[string]any{
			"type":   "object",
			"fields": map[string]any{"full": map[string]any{"type": "string"}},
		},
	}
	if err := st.Register(map[string]any{
		"name": c.Resource, "collection": c.Resource, "idPrefix": "u",
		"fields": fields,
	}); err != nil {
		t.Fatalf("Register: %v", err)
	}
	if err := st.EnsureTables(map[string]any{
		"name": c.Resource, "collection": c.Resource,
		"fields": fields,
	}); err != nil {
		t.Fatalf("EnsureTables: %v", err)
	}

	// 用例级 mock resolver（与 rust runner 同构；未声明的档位保持未注入 ⇒ 501）
	var fileResolver func(*http.Request, map[string]any, string) (*FileOut, error)
	if (c.Opts.FileResolver || c.Opts.FileResolverNotFound) && !c.Opts.NoFileResolver {
		notFound := c.Opts.FileResolverNotFound
		fileResolver = func(_ *http.Request, _ map[string]any, _ string) (*FileOut, error) {
			if notFound {
				return nil, fmt.Errorf("ERR_RESOURCE_NOT_FOUND:资源无任何可读副本")
			}
			return &FileOut{Body: []byte("mock-bytes")}, nil
		}
	}
	var uploadResolver func(*http.Request, map[string]any, string) (*UploadOut, error)
	if c.Opts.UploadResolver && !c.Opts.NoUploadResolver {
		uploadResolver = func(_ *http.Request, _ map[string]any, _ string) (*UploadOut, error) {
			return &UploadOut{Ref: "ref://mock"}, nil
		}
	}

	mux := http.NewServeMux()
	if err := New(mux, st, Options{
		Prefix:         "/api",
		FileResolver:   fileResolver,
		UploadResolver: uploadResolver,
	}); err != nil {
		t.Fatalf("New: %v", err)
	}
	srv := httptest.NewServer(mux)
	defer srv.Close()

	var createdID string
	_ = context.Background()
	for i, step := range c.Steps {
		path := normalizeCasePath(strings.ReplaceAll(step.Path, "{id}", createdID))
		// `raw:<bytes>` 是上传用例的原始字节体约定；其余 JSON body 原样序列化（与 rust runner 一致）
		var bodyReader io.Reader
		isRaw := false
		if len(step.Body) > 0 {
			var s string
			if err := json.Unmarshal(step.Body, &s); err == nil && strings.HasPrefix(s, "raw:") {
				isRaw = true
				bodyReader = strings.NewReader(s[4:])
			} else {
				bodyReader = strings.NewReader(string(step.Body))
			}
		}
		req, err := http.NewRequest(step.Method, srv.URL+path, bodyReader)
		if err != nil {
			t.Fatalf("step %d 构造请求失败: %v", i, err)
		}
		if len(step.Body) > 0 && !isRaw {
			req.Header.Set("Content-Type", "application/json")
		}
		for k, v := range step.Headers {
			req.Header.Set(k, v)
		}
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			t.Fatalf("step %d 请求失败: %v", i, err)
		}
		raw, _ := io.ReadAll(resp.Body)
		resp.Body.Close()

		var out map[string]any
		_ = json.Unmarshal(raw, &out)

		// 断言 1：status
		if resp.StatusCode != step.Expect.Status {
			t.Errorf("step %d (%s %s) status = %d, want %d（响应: %s）",
				i, step.Method, path, resp.StatusCode, step.Expect.Status, raw)
			continue
		}
		// 断言 2：error.code
		if step.Expect.ErrorCode != "" {
			errObj, _ := out["error"].(map[string]any)
			if errObj == nil || errObj["code"] != step.Expect.ErrorCode {
				t.Errorf("step %d error.code = %v, want %q（响应: %s）",
					i, errObj, step.Expect.ErrorCode, raw)
			}
		}
		// 断言 3：dataKeys（data 中必须出现的键）
		if len(step.Expect.DataKeys) > 0 {
			data, _ := out["data"].(map[string]any)
			if data == nil {
				t.Errorf("step %d 缺少 data 对象（响应: %s）", i, raw)
			} else {
				for _, k := range step.Expect.DataKeys {
					if _, ok := data[k]; !ok {
						t.Errorf("step %d data 缺键 %q（响应: %s）", i, k, raw)
					}
				}
			}
		}
		// 记录创建的 _id（useCreatedId 步骤的占位替换源）
		if data, ok := out["data"].(map[string]any); ok {
			if id, ok := data["_id"].(string); ok && id != "" {
				createdID = id
			}
		}
	}
	if t.Failed() {
		t.Fatalf("用例 %s 有失败步骤", c.Name)
	}
	fmt.Printf("conformance %s: %d steps 全部通过\n", c.Name, len(c.Steps))
}

// normalizeCasePath 规范化用例中的原始 URL：用例 JSON 的 query 串按惯例不预转义
// （含空格/大括号/引号），http.NewRequest 要求合法 URL，故拆 query 后逐参数重编码。
func normalizeCasePath(p string) string {
	i := strings.IndexByte(p, '?')
	if i < 0 {
		return p
	}
	q, err := url.ParseQuery(p[i+1:])
	if err != nil {
		return p // 解析失败保持原样，让服务端按原始语义报错（400）
	}
	return p[:i+1] + q.Encode()
}
