// params.go —— 查询参数 → GQL params 绑定。
// 规范依据：store-api/spec/02-params.md（多端逐字一致，改动必须先改 spec）。
package storeapi

import (
	"encoding/json"
	"net/url"
	"regexp"
	"strconv"
)

var (
	intRe   = regexp.MustCompile(`^-?\d+$`)
	floatRe = regexp.MustCompile(`^-?\d+\.\d+$`)
)

// convertValue p.<name> 值的类型转换（spec/02，多端逐字一致）：
// true/false/null → 字面量；整数/浮点正则全匹配 → 数值；
// `{`/`[` 开头 → JSON 解析（失败报 INVALID_PARAM，禁静默当字符串）；其余 → 字符串原样。
func convertValue(raw string) (any, error) {
	switch raw {
	case "true":
		return true, nil
	case "false":
		return false, nil
	case "null":
		return nil, nil
	}
	if intRe.MatchString(raw) {
		n, err := strconv.ParseInt(raw, 10, 64)
		if err != nil {
			// 超出 int64 的纯整数串：保持字符串原样（spec：首个命中即止，转换语义以正则全匹配为准）
			return raw, nil
		}
		return n, nil
	}
	if floatRe.MatchString(raw) {
		f, err := strconv.ParseFloat(raw, 64)
		if err != nil {
			return raw, nil
		}
		return f, nil
	}
	if len(raw) > 0 && (raw[0] == '{' || raw[0] == '[') {
		var v any
		if err := json.Unmarshal([]byte(raw), &v); err != nil {
			return nil, invalidParam("参数 JSON 解析失败: " + err.Error())
		}
		return v, nil
	}
	return raw, nil
}

// parseQueryParams 从 URL query 收集 p.* 参数（spec/02）：
// 同名重复出现 → 数组（每元素各自转换）；单次 → 标量。
func parseQueryParams(q url.Values) (map[string]any, error) {
	params := map[string]any{}
	for key, vals := range q {
		if len(key) < 2 || key[:2] != "p." {
			continue
		}
		name := key[2:]
		if len(vals) == 1 {
			v, err := convertValue(vals[0])
			if err != nil {
				return nil, err
			}
			params[name] = v
			continue
		}
		arr := make([]any, len(vals))
		for i, raw := range vals {
			v, err := convertValue(raw)
			if err != nil {
				return nil, err
			}
			arr[i] = v
		}
		params[name] = arr
	}
	return params, nil
}
