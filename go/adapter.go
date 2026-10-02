// adapter.go —— store-api 的 Go 适配器：为 go-store 已注册 schema 自动生成 RESTful 路由。
//
// 路由/参数/错误/上下文语义全部以 ../store-api/spec/*.md 为唯一依据（多端 parity，改动先改 spec）。
// 设计哲学：REST 只是 GQL 的 HTTP 皮 —— 适配层零语义发明，凡 store 没有的语义本层一律不提供。
package storeapi

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strings"

	"github.com/coenddt/go-store"
)

// Options 适配器选项（语义对齐 store-api-node / store-api-py）。
type Options struct {
	// Prefix 路由前缀（如 "/api"），空串 = 根路径。
	Prefix string
	// IDField 单条路由主键字段名（spec/01：多端默认 "_id" 且必须一致）。空串取默认。
	IDField string
	// ContextProvider 每请求上下文钩子（spec/04）。nil 时不注入。
	// 返回的 ctx 为 nil 同样按「空上下文」语义传递（防身份跨请求残留）。
	// error 非 nil 时按 spec/04 分类：PermissionError ⇒ 403；其余 ⇒ 401 CONTEXT_ERROR。
	ContextProvider func(r *http.Request) (*gostore.Context, error)
	// Resources 显式资源名；nil 时取 store.ListSchemas() 并过滤归档表。
	Resources []string
}

// archiveSuffix 归档表后缀（spec/01：XxxDeleted 且 Xxx 也在列表中 ⇒ 归档表，不生成路由）。
const archiveSuffix = "Deleted"

// FilterArchived 归档表过滤（spec/01，多端逐字一致）。
func FilterArchived(names []string) []string {
	set := make(map[string]struct{}, len(names))
	for _, n := range names {
		set[n] = struct{}{}
	}
	out := make([]string, 0, len(names))
	for _, n := range names {
		if strings.HasSuffix(n, archiveSuffix) {
			base := strings.TrimSuffix(n, archiveSuffix)
			if _, ok := set[base]; ok {
				continue
			}
		}
		out = append(out, n)
	}
	return out
}

// New 生成挂载到任意 net/http 服务的路由（标准库 ServeMux，零框架依赖）。
// 用法：
//
//	mux := http.NewServeMux()
//	storeapi.New(mux, store, storeapi.Options{Prefix: "/api"})
//	http.ListenAndServe(":3000", mux)
func New(mux *http.ServeMux, st *gostore.Store, opts Options) error {
	if st == nil {
		return fmt.Errorf("storeapi.New 需要 store（go-store 实例，须已 register）")
	}
	if opts.IDField == "" {
		opts.IDField = "_id"
	}
	prefix := strings.TrimSuffix(opts.Prefix, "/")

	names := opts.Resources
	if names == nil {
		list, err := st.ListSchemas()
		if err != nil {
			return fmt.Errorf("storeapi.New 读取 schema 列表失败: %w", err)
		}
		names = FilterArchived(list)
	}

	// 投影段缓存（spec/01+02：投影取自 schema 的 fields 键列表；fields 为空时省略投影段）
	projections := make(map[string]string, len(names))
	for _, name := range names {
		if keys := st.SchemaFieldsKeys(name); len(keys) > 0 {
			projections[name] = " { " + strings.Join(keys, ", ") + " }"
		}
	}

	for _, name := range names {
		collection := name
		idField := opts.IDField
		base := prefix + "/" + name

		handler := func(serve func(w http.ResponseWriter, r *http.Request, ctx *gostore.Context)) http.HandlerFunc {
			return func(w http.ResponseWriter, r *http.Request) {
				// spec/04：provider 错误优先分类（403/401），之后业务错误走各自映射。
				// （资源防御无需显式守卫：本适配器按已知资源注册具体路由，未注册名
				// 天然 404——与 rust 版通配路由 {resource} 需 RESOURCE_NOT_FOUND 守卫的形态不同。）
				ctx := (*gostore.Context)(nil)
				if opts.ContextProvider != nil {
					c, err := opts.ContextProvider(r)
					if err != nil {
						writeError(w, mapContextError(err))
						return
					}
					ctx = c
				}
				serve(w, r, ctx)
			}
		}

		// GET /{resource} —— 列表（q 透传 + p.* 绑定；spec/02）
		mux.HandleFunc("GET "+base, handler(func(w http.ResponseWriter, r *http.Request, ctx *gostore.Context) {
			params, err := parseQueryParams(r.URL.Query())
			if err != nil {
				writeError(w, err)
				return
			}
			// spec/01+02：q 缺失 → 全字段投影（省略投影 = 只回 _id）；q 存在 → 投影由 q 决定
			gql := collection + r.URL.Query().Get("q")
			if r.URL.Query().Get("q") == "" {
				gql = collection + projections[collection]
			}
			rows, err := st.Query(r.Context(), gql, params, ctx)
			if err != nil {
				writeError(w, err)
				return
			}
			writeJSON(w, http.StatusOK, map[string]any{"data": rows})
		}))

		// GET /{resource}/{id} —— 单条（spec/01 主键等值条件）。
		// 投影段取自 schema 元数据的 fields 键列表（spec/01 明文），fields 为空时省略投影段。
		projection := ""
		if keys := st.SchemaFieldsKeys(collection); len(keys) > 0 {
			projection = " { " + strings.Join(keys, ", ") + " }"
		}
		mux.HandleFunc("GET "+base+"/{id}", handler(func(w http.ResponseWriter, r *http.Request, ctx *gostore.Context) {
			id := r.PathValue("id")
			one, err := st.QueryOne(r.Context(),
				fmt.Sprintf("%s($condition: @c0)%s", collection, projection),
				map[string]any{"c0": map[string]any{idField: id}},
				ctx)
			if err != nil {
				writeError(w, err)
				return
			}
			if one == nil {
				writeError(w, notFound(fmt.Sprintf("记录不存在: %s=%s", idField, id)))
				return
			}
			writeJSON(w, http.StatusOK, map[string]any{"data": one})
		}))

		// POST /{resource} —— 插入（201）
		mux.HandleFunc("POST "+base, handler(func(w http.ResponseWriter, r *http.Request, ctx *gostore.Context) {
			body, err := requireBody(r)
			if err != nil {
				writeError(w, err)
				return
			}
			doc, err := st.Insert(r.Context(), collection, body, ctx)
			if err != nil {
				writeError(w, err)
				return
			}
			writeJSON(w, http.StatusCreated, map[string]any{"data": doc})
		}))

		// PATCH /{resource}/{id} —— 部分更新（spec/01：不提供 PUT——store 无全量替换语义）
		mux.HandleFunc("PATCH "+base+"/{id}", handler(func(w http.ResponseWriter, r *http.Request, ctx *gostore.Context) {
			body, err := requireBody(r)
			if err != nil {
				writeError(w, err)
				return
			}
			id := r.PathValue("id")
			doc, err := st.Update(r.Context(), collection,
				map[string]any{idField: id}, body, ctx)
			if err != nil {
				writeError(w, err)
				return
			}
			// spec/03：update 影响 0 行返回 200 与 store 原始结果（null），是否业务异常由调用方判断
			writeJSON(w, http.StatusOK, map[string]any{"data": doc})
		}))

		// DELETE /{resource}/{id} —— 删除（归档编排）
		mux.HandleFunc("DELETE "+base+"/{id}", handler(func(w http.ResponseWriter, r *http.Request, ctx *gostore.Context) {
			id := r.PathValue("id")
			out, err := st.Remove(r.Context(), collection,
				map[string]any{idField: id}, ctx)
			if err != nil {
				writeError(w, err)
				return
			}
			writeJSON(w, http.StatusOK, map[string]any{"data": out})
		}))
	}
	return nil
}

// requireBody 请求体守卫（spec/01：必须 JSON 对象；缺失/非对象 ⇒ 400 INVALID_BODY）。
func requireBody(r *http.Request) (map[string]any, error) {
	b, err := io.ReadAll(io.LimitReader(r.Body, 1<<20))
	if err != nil {
		return nil, invalidBody(fmt.Sprintf("请求体读取失败: %v", err))
	}
	var body map[string]any
	if err := json.Unmarshal(b, &body); err != nil {
		return nil, invalidBody("请求体必须是 JSON 对象")
	}
	if body == nil {
		return nil, invalidBody("请求体必须是 JSON 对象")
	}
	return body, nil
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.WriteHeader(status)
	enc := json.NewEncoder(w)
	enc.SetEscapeHTML(false)
	_ = enc.Encode(v)
}
