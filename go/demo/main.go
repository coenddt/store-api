// demo/main.go —— 全链路验收：文件库 SQLite + 真实 HTTP REST + Go 计算列。
// 运行：go run ./demo，随后按输出的端口用任意 HTTP 客户端验收。
package main

import (
	"fmt"
	"net/http"
	"os"

	gostore "github.com/coenddt/go-store"
	storeapi "github.com/coenddt/store-api-go"
)

func main() {
	db := "demo.db"
	_ = os.Remove(db) // 每次验收从零开始

	store, err := gostore.Open("sqlite://" + db)
	must(err, "Open")
	defer store.Close()

	orderSchema := map[string]any{
		"name": "order", "collection": "orders", "idPrefix": "o",
		"fields": map[string]any{
			"status": map[string]any{"type": "string"},
			"amount": map[string]any{"type": "number"},
			"owner":  map[string]any{"type": "string"},
		},
		"computes": map[string]any{
			"statusUpper": map[string]any{"type": "string", "fn": true, "fnRef": "goUpper"},
			"amountAud":   map[string]any{"type": "number", "asyncFn": true, "fnRef": "goAud"},
		},
	}
	must(store.Register(orderSchema), "Register")
	must(store.EnsureTables(orderSchema), "EnsureTables")

	// Go 定义计算列（同步 fn + 批量 asyncFn）
	store.RegisterSyncCompute("goUpper", func(ctx *gostore.Context, doc map[string]any) (any, error) {
		v, _ := doc["status"].(string)
		return v + "!", nil
	})
	store.RegisterCompute("goAud", func(ctx *gostore.Context, docs []map[string]any) error {
		for _, d := range docs {
			if amt, ok := d["amount"].(float64); ok {
				d["amountAud"] = amt * 15000
			}
		}
		return nil
	})

	mux := http.NewServeMux()
	must(storeapi.New(mux, store, storeapi.Options{
		Prefix: "/api",
		ContextProvider: func(r *http.Request) (*gostore.Context, error) {
			role := r.Header.Get("X-Role")
			if role == "" {
				return nil, nil
			}
			return &gostore.Context{UserID: "u-demo", Roles: []string{role}}, nil
		},
	}), "storeapi.New")

	fmt.Println("REST 服务已就绪: http://127.0.0.1:8091/api/order （文件库 demo.db）")
	fmt.Println(http.ListenAndServe("127.0.0.1:8091", mux))
}

func must(err error, what string) {
	if err != nil {
		fmt.Fprintf(os.Stderr, "%s 失败: %v\n", what, err)
		os.Exit(1)
	}
}
