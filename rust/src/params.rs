//! 查询参数 → GQL params 绑定。
//! 规范依据：spec/02-params.md（三端逐字一致，改动必须先改 spec）。

use serde_json::{Map, Value};

/// 与整数正则 `^-?\d+$` 严格等价
fn is_int(s: &str) -> bool {
    let body = s.strip_prefix('-').unwrap_or(s);
    !body.is_empty() && body.bytes().all(|c| c.is_ascii_digit())
}

/// 与浮点正则 `^-?\d+\.\d+$` 严格等价
fn is_float(s: &str) -> bool {
    let body = s.strip_prefix('-').unwrap_or(s);
    match body.split_once('.') {
        Some((int_part, frac_part)) => {
            !int_part.is_empty()
                && !frac_part.is_empty()
                && int_part.bytes().all(|c| c.is_ascii_digit())
                && frac_part.bytes().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

/// p.<name> 值的类型转换：true/false/null → 字面量；整数/浮点正则全匹配 → 数值；
/// `{`/`[` 开头 → JSON 解析（失败返回错误原文，禁静默当字符串）；其余 → 字符串原样。
pub fn convert_value(raw: &str) -> Result<Value, String> {
    match raw {
        "true" => return Ok(Value::Bool(true)),
        "false" => return Ok(Value::Bool(false)),
        "null" => return Ok(Value::Null),
        _ => {}
    }
    if is_int(raw) {
        if let Ok(i) = raw.parse::<i64>() {
            return Ok(Value::Number(i.into()));
        }
    }
    if is_float(raw) {
        if let Ok(f) = raw.parse::<f64>() {
            if let Some(n) = serde_json::Number::from_f64(f) {
                return Ok(Value::Number(n));
            }
        }
    }
    if raw.starts_with('{') || raw.starts_with('[') {
        return serde_json::from_str::<Value>(raw).map_err(|e| format!("参数 JSON 解析失败: {e}"));
    }
    Ok(Value::String(raw.to_string()))
}

/// 从原始 query string 收集 p.* 参数。同名重复出现 → 数组（每元素各自转换）；单次 → 标量。
pub fn parse_query_params(query: &str) -> Result<Map<String, Value>, String> {
    let mut ordered: Vec<(String, Value)> = Vec::new();
    for (k, v) in form_urlencoded::parse(query.as_bytes()) {
        let key = k.to_string();
        if !key.starts_with("p.") {
            continue;
        }
        let converted = convert_value(&v.to_string())?;
        ordered.push((key[2..].to_string(), converted));
    }
    let mut out = Map::new();
    for (name, v) in ordered {
        match out.remove(&name) {
            Some(Value::Array(mut arr)) => {
                arr.push(v);
                out.insert(name, Value::Array(arr));
            }
            Some(prev) => {
                out.insert(name, Value::Array(vec![prev, v]));
            }
            None => {
                out.insert(name, v);
            }
        }
    }
    Ok(out)
}
