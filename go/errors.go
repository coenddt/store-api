// errors.go —— 错误 → HTTP 状态码映射。
// 规范依据：store-api/spec/03-errors.md（判定顺序多端一致，改动必须先改 spec）。
// 遵循 no-error-masking：message 原样透传、取不到置 null（禁伪造），成功响应不得带 error 字段。
package storeapi

import (
	"encoding/json"
	"net/http"
	"strings"

	"github.com/coenddt/go-store"
)

// adapterError 适配层自身守卫错误（带稳定 code）。
type adapterError struct {
	Code    string
	Message string
	Status  int
}

func (e *adapterError) Error() string { return e.Message }

func invalidBody(msg string) error      { return &adapterError{"INVALID_BODY", msg, http.StatusBadRequest} }
func invalidParam(msg string) error     { return &adapterError{"INVALID_PARAM", msg, http.StatusBadRequest} }
func notFound(msg string) error         { return &adapterError{"NOT_FOUND", msg, http.StatusNotFound} }


// mapContextError 分类 provider 错误（spec/04）：
// PermissionError ⇒ 403（RBAC 拒绝，与业务权限拒绝同一语义）；其余 ⇒ 401 CONTEXT_ERROR。
// go-store 的 PermissionError 判定按 core 的稳定前缀 ERR_PERMISSION:（core command/mod.rs：
// 「Host 按前缀识别，不对中文文案做脆弱匹配」——即 Go 侧的错误类型判定依据）。
func mapContextError(err error) error {
	if isPermissionError(err) {
		return &adapterError{permCode(err), err.Error(), http.StatusForbidden}
	}
	msg := ""
	if err != nil {
		msg = err.Error()
	}
	return &adapterError{"CONTEXT_ERROR", msg, http.StatusUnauthorized}
}

// isPermissionError 按稳定前缀判定（core ERR_PERM_PREFIX 契约），禁按文案匹配。
func isPermissionError(err error) bool {
	return err != nil && strings.HasPrefix(err.Error(), "ERR_PERMISSION:")
}

// gqlParsePrefix GQL 解析失败的稳定前缀（core pipeline/parse.rs；spec/03 v1 修订：
// 与 ERR_PERM_PREFIX 同构的类型级契约，四端按前缀判定 → 400 GQL_PARSE）。
const gqlParsePrefix = "ERR_GQL_PARSE:"

func isGqlParseError(err error) bool {
	return err != nil && strings.HasPrefix(err.Error(), gqlParsePrefix)
}

// gqlParseMessage 剥离稳定前缀取 core 原文。
func gqlParseMessage(err error) string {
	return strings.TrimPrefix(err.Error(), gqlParsePrefix)
}

// permCode 权限错误码：剥离稳定前缀取语义段（如 "无写入权限"），保证 code 非空可判。
func permCode(err error) string {
	s := strings.TrimPrefix(err.Error(), "ERR_PERMISSION:")
	if i := strings.IndexByte(s, ':'); i >= 0 {
		s = s[:i]
	}
	if s == "" {
		s = "ERR_PERMISSION"
	}
	return s
}

// errorPayload 统一错误形态（spec/03）：
// {"error":{"code":"<store 错误名或适配层错误名>","message":"<原文，原样透传>"}}
// message 取不到时显式 null（禁伪造）。
func errorPayload(code string, message any) map[string]any {
	return map[string]any{
		"error": map[string]any{"code": code, "message": message},
	}
}

// writeError 单一错误出口（spec/03 判定顺序）：
// 适配层守卫(400/404/401) → 权限前缀(403) → 其余 store 错误(500 透传)。
func writeError(w http.ResponseWriter, err error) {
	var status int
	var code string
	var message any

	var ae *adapterError
	if ok := asAdapterError(err, &ae); ok {
		status, code, message = ae.Status, ae.Code, ae.Message
	} else if isPermissionError(err) {
		status, code, message = http.StatusForbidden, permCode(err), err.Error()
	} else if isGqlParseError(err) {
		// spec/03 判定顺序第 3 层：GQL 解析失败 → 400 GQL_PARSE（message 剥前缀取原文）
		status, code, message = http.StatusBadRequest, "GQL_PARSE", gqlParseMessage(err)
	} else {
		// 其余 store 错误（数据库、连接、方言等）500 透传；message 为空时显式 null
		status, code = http.StatusInternalServerError, "STORE_ERROR"
		if err != nil {
			code, message = splitStoreCode(err.Error())
		}
	}

	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.WriteHeader(status)
	enc := json.NewEncoder(w)
	enc.SetEscapeHTML(false)
	_ = enc.Encode(errorPayload(code, message))
}

// splitStoreCode 把 go-store 错误串拆成 code 与 message：
// 形如 "ERR_PERMISSION:无写入权限" 的带码错误拆分；其余 code=STORE_ERROR、message 原文。
func splitStoreCode(s string) (string, any) {
	for _, prefix := range []string{"ERR_PERMISSION:", "ERR_NO_CONTEXT:", "ERR_TEXT2QUERY:"} {
		if strings.HasPrefix(s, prefix) {
			rest := strings.TrimPrefix(s, prefix)
			code := strings.TrimSuffix(prefix, ":")
			if i := strings.IndexByte(rest, ':'); i >= 0 && len(rest) > i+1 {
				code, rest = rest[:i], rest[i+1:]
			}
			return code, rest
		}
	}
	return "STORE_ERROR", s
}

func asAdapterError(err error, target **adapterError) bool {
	if e, ok := err.(*adapterError); ok {
		*target = e
		return true
	}
	return false
}

// 确保 gostore 依赖（前缀契约文档引用）不被误删
var _ = gostore.Context{}
