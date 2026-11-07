// file.go —— 资源字节子路由（GET/POST /{resource}/{id}/file）。
//
// 语义唯一依据：store-api/spec/01-routing.md、02-params.md、03-errors.md、05-response-headers.md。
// 与 node/rust 端逐档对齐：未注入 resolver 501 → 空体 400 / 超限 413 / 白名单 400
// → queryOne 空 404 → 字段空 404 → resolver 错误按 core 前缀分类 → 200。
package storeapi

import (
	"fmt"
	"io"
	"net/http"
	"sort"
	"strings"

	"github.com/coenddt/go-store"
)

// serveDownload GET /{resource}/{id}/file（spec/03 判定顺序第 1 层：未注入 resolver ⇒ 501）。
func serveDownload(
	w http.ResponseWriter,
	r *http.Request,
	ctx *gostore.Context,
	st *gostore.Store,
	opts Options,
	collection, idField string,
) {
	if opts.FileResolver == nil {
		writeError(w, fileNotConfigured("GET /{resource}/{id}/file 未注入 fileResolver"))
		return
	}
	id := r.PathValue("id")
	// 下载（读）不受上传白名单限制（spec/02）
	fp, err := fieldPathOf(r, opts.FileField, nil)
	if err != nil {
		writeError(w, err)
		return
	}

	rec, err := queryFileRecord(r, ctx, st, collection, idField, id, fp)
	if err != nil {
		writeError(w, err)
		return
	}
	if rec == nil {
		writeError(w, notFound(fmt.Sprintf("记录不存在: %s=%s", idField, id)))
		return
	}
	if _, ok := pickField(rec, fp); !ok {
		writeError(w, notFound(fmt.Sprintf("文件不存在: %s=%s", fp, id)))
		return
	}

	out, err := opts.FileResolver(r, rec, id)
	if err != nil {
		// resolver 错误按 core 前缀分类（含 ERR_RESOURCE_NOT_FOUND: ⇒ 404 NOT_FOUND）
		writeError(w, err)
		return
	}
	if out == nil {
		writeError(w, &adapterError{
			Code:    "STORE_ERROR",
			Message: fmt.Sprintf("fileResolver 未返回结果: %s(%s=%s)", collection, idField, id),
			Status:  http.StatusInternalServerError,
		})
		return
	}

	contentType := out.ContentType
	if contentType == "" {
		contentType = "application/octet-stream"
	}
	fileName := out.FileName
	if fileName == "" {
		fileName = "file"
	}
	// 响应头注入防线：fileName 的 CR/LF 必须清洗（与 node 端一致）
	fileName = strings.NewReplacer("\r", " ", "\n", " ").Replace(fileName)

	w.Header().Set("Content-Type", contentType)
	w.Header().Set("Content-Disposition", fmt.Sprintf("attachment; filename=%q", fileName))
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write(out.Body)
}

// serveUpload POST /{resource}/{id}/file（spec/03 判定顺序第 1 层：未注入 resolver ⇒ 501）。
func serveUpload(
	w http.ResponseWriter,
	r *http.Request,
	ctx *gostore.Context,
	st *gostore.Store,
	opts Options,
	collection, idField string,
	allowed map[string]struct{},
) {
	if opts.UploadResolver == nil {
		writeError(w, uploadNotConfigured("POST /{resource}/{id}/file 未注入 uploadResolver"))
		return
	}

	// 多读 1 字节即可判定超限；空体与超限均在 resolver 调用前拒绝。
	limit := opts.UploadLimit
	body, err := io.ReadAll(io.LimitReader(r.Body, limit+1))
	if err != nil {
		writeError(w, tooLarge(fmt.Sprintf("上传字节体超过上限 %d 字节: %v", limit, err)))
		return
	}
	if len(body) == 0 {
		writeError(w, emptyBody(fmt.Sprintf("上传请求体为空: POST /{resource}/{id}/file")))
		return
	}
	if int64(len(body)) > limit {
		writeError(w, tooLarge(fmt.Sprintf("上传字节体 %d 字节超过上限 %d 字节", len(body), limit)))
		return
	}

	fp, err := fieldPathOf(r, opts.FileField, allowed)
	if err != nil {
		writeError(w, err)
		return
	}

	id := r.PathValue("id")
	rec, err := queryFileRecord(r, ctx, st, collection, idField, id, fp)
	if err != nil {
		writeError(w, err)
		return
	}
	if rec == nil {
		writeError(w, notFound(fmt.Sprintf("记录不存在: %s=%s", idField, id)))
		return
	}

	out, err := opts.UploadResolver(r, rec, id)
	if err != nil {
		writeError(w, err)
		return
	}
	if out == nil || out.Ref == "" {
		// 禁静默：resolver 未给出引用即请求期显式失败
		writeError(w, &adapterError{
			Code:    "STORE_ERROR",
			Message: fmt.Sprintf("uploadResolver 未返回 ref: %s(%s=%s)", collection, idField, id),
			Status:  http.StatusInternalServerError,
		})
		return
	}

	// 回写走 update 语义 → RBAC 写权限自动生效（spec/01）。
	// core 的 SQL 方言只接受列级键（含点号的键会被 writable_col 丢弃），故点路径
	// 在此还原为嵌套对象后整体写回；标量路径保持原样。
	data := writebackData(rec, fp, out.Ref)
	doc, err := st.Update(r.Context(), collection,
		map[string]any{idField: id}, data, ctx)
	if err != nil {
		writeError(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"data": doc})
}

// queryFileRecord 取 file 子路由的目标记录；投影 = schema 标量字段 ∪ field 根字段
// （与 rust 端 file_projection 同构：schema 未声明资源引用字段时仍能取到）。
func queryFileRecord(
	r *http.Request,
	ctx *gostore.Context,
	st *gostore.Store,
	collection, idField, id, fieldPath string,
) (map[string]any, error) {
	root := fieldPath
	if i := strings.IndexByte(root, '.'); i >= 0 {
		root = root[:i]
	}
	keys := append([]string{}, st.SchemaFieldsKeys(collection)...)
	found := false
	for _, k := range keys {
		if k == root {
			found = true
			break
		}
	}
	if root != "" && !found {
		keys = append(keys, root)
	}
	projection := ""
	if len(keys) > 0 {
		projection = " { " + strings.Join(keys, ", ") + " }"
	}
	return st.QueryOne(r.Context(),
		fmt.Sprintf("%s($condition: @c0)%s", collection, projection),
		map[string]any{"c0": map[string]any{idField: id}},
		ctx)
}

// pickField 点路径取值（spec/02：任一段缺失 ⇒ 视同字段为空）。
func pickField(rec map[string]any, fieldPath string) (any, bool) {
	if rec == nil {
		return nil, false
	}
	var cur any = rec
	for _, seg := range strings.Split(fieldPath, ".") {
		m, ok := cur.(map[string]any)
		if !ok {
			return nil, false
		}
		v, ok := m[seg]
		if !ok || v == nil {
			return nil, false
		}
		cur = v
	}
	return cur, true
}

// fieldPathOf 请求级字段点路径：?field= 优先，缺省回落 opts.FileField。
// allowed 非 nil（上传）时必须命中白名单；下载传 nil 不限制（spec/02 决策 S3）。
func fieldPathOf(r *http.Request, fallback string, allowed map[string]struct{}) (string, error) {
	f := r.URL.Query().Get("field")
	if f == "" {
		return fallback, nil
	}
	if allowed != nil {
		if _, ok := allowed[f]; !ok {
			names := make([]string, 0, len(allowed))
			for k := range allowed {
				names = append(names, k)
			}
			sort.Strings(names)
			list := strings.Join(names, ", ")
			if list == "" {
				list = "(无)"
			}
			return "", invalidParam(fmt.Sprintf("field 不在可写字段白名单: %s；允许: %s", f, list))
		}
	}
	return f, nil
}

// 上传字段白名单：主键与原型污染向量不得作为写入路径（spec/02）。
var forbiddenFieldSegments = map[string]struct{}{
	"_id": {}, "__proto__": {}, "constructor": {}, "prototype": {},
}

// allowedFieldPaths 由 schema 字段树生成可写点路径白名单（spec/02）：
// 只含标量 string / file 字段；object 字段下钻为 a.b；$ 前缀与禁用段一律排除。
func allowedFieldPaths(fields map[string]any) map[string]struct{} {
	out := map[string]struct{}{}
	walkFieldPaths(fields, "", out)
	return out
}

func walkFieldPaths(fields map[string]any, prefix string, out map[string]struct{}) {
	for key, raw := range fields {
		if strings.HasPrefix(key, "$") {
			continue
		}
		if _, bad := forbiddenFieldSegments[key]; bad {
			continue
		}
		path := key
		if prefix != "" {
			path = prefix + "." + key
		}
		defn, _ := raw.(map[string]any)
		if nested, ok := defn["fields"].(map[string]any); ok {
			walkFieldPaths(nested, path, out)
			continue
		}
		typ, _ := defn["type"].(string)
		if typ == "string" || typ == "file" {
			out[path] = struct{}{}
		}
	}
}

// writebackData 把「字段点路径 + 引用」翻译为 update data。
// 标量字段原样；点路径按 rec 的现有对象合并后整体写回（见 serveUpload 注释）。
func writebackData(rec map[string]any, fieldPath string, ref string) map[string]any {
	i := strings.IndexByte(fieldPath, '.')
	if i < 0 {
		return map[string]any{fieldPath: ref}
	}
	root := fieldPath[:i]
	base := map[string]any{}
	if cur, ok := rec[root].(map[string]any); ok {
		for k, v := range cur {
			base[k] = v
		}
	}
	segs := strings.Split(fieldPath, ".")
	var cursor map[string]any = base
	for n := 1; n < len(segs); n++ {
		if n == len(segs)-1 {
			cursor[segs[n]] = ref
			break
		}
		next, ok := cursor[segs[n]].(map[string]any)
		if !ok {
			next = map[string]any{}
			cursor[segs[n]] = next
		}
		cursor = next
	}
	return map[string]any{root: base}
}
