//! 错误 → HTTP 状态码映射。
//! 规范依据：spec/03-errors.md（判定顺序三端一致，改动必须先改 spec）。
//! 遵循 no-error-masking：message 原样透传、取不到置 null（禁伪造），成功响应不得带 error 字段。

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

/// 适配层自身守卫错误（400/401/404），message 为确定性描述
#[derive(Debug, Clone)]
pub struct StoreApiError {
    pub status: StatusCode,
    pub code: String,
    pub message: Option<String>,
}

impl StoreApiError {
    pub fn invalid_body(message: &str) -> Self {
        Self { status: StatusCode::BAD_REQUEST, code: "INVALID_BODY".into(), message: Some(message.into()) }
    }
    pub fn invalid_param(message: String) -> Self {
        Self { status: StatusCode::BAD_REQUEST, code: "INVALID_PARAM".into(), message: Some(message) }
    }
    pub fn not_found(message: String) -> Self {
        Self { status: StatusCode::NOT_FOUND, code: "NOT_FOUND".into(), message: Some(message) }
    }
    pub fn context_error(message: Option<String>) -> Self {
        Self { status: StatusCode::UNAUTHORIZED, code: "CONTEXT_ERROR".into(), message }
    }
    /// spec/03 判定顺序第 1 层：下载路由未注入 `fileResolver` → 501
    pub fn file_not_configured(message: &str) -> Self {
        Self { status: StatusCode::NOT_IMPLEMENTED, code: "FILE_NOT_CONFIGURED".into(), message: Some(message.into()) }
    }
    /// spec/03 判定顺序第 1 层：上传路由未注入 `uploadResolver` → 501
    pub fn upload_not_configured(message: &str) -> Self {
        Self { status: StatusCode::NOT_IMPLEMENTED, code: "UPLOAD_NOT_CONFIGURED".into(), message: Some(message.into()) }
    }
    /// spec/03 判定顺序第 2 层：上传字节体为空 → 400
    pub fn empty_body(message: &str) -> Self {
        Self { status: StatusCode::BAD_REQUEST, code: "EMPTY_BODY".into(), message: Some(message.into()) }
    }
    /// spec/03 判定顺序第 2 层：上传字节体超过上限 → 413
    pub fn too_large(message: &str) -> Self {
        Self { status: StatusCode::PAYLOAD_TOO_LARGE, code: "UPLOAD_TOO_LARGE".into(), message: Some(message.into()) }
    }
}

pub fn error_payload(code: &str, message: Option<String>) -> Value {
    json!({ "error": { "code": code, "message": message } })
}

impl IntoResponse for StoreApiError {
    fn into_response(self) -> Response {
        (self.status, Json(error_payload(&self.code, self.message))).into_response()
    }
}

/// store 错误的分类码。store 层错误形态为 String（core 契约）：
/// 权限错误带 `ERR_PERMISSION:` 前缀（core command/mod.rs:80 `ERR_PERM_PREFIX`，
/// 这是 core 的**类型级契约**，与 node 端 instanceof 同级，非 message 语义匹配）。
pub fn store_code(err: &str) -> String {
    // 仅认 core 的稳定前缀集合（ERR_PERM_PREFIX / ERR_NO_CONTEXT / ERR_TEXT2QUERY /
    // ERR_GQL_PARSE / ERR_NOT_FOUND，见 core command/mod.rs 与 pipeline/parse.rs）；
    // 无前缀错误一律 STORE_ERROR——禁把错误文案整串当 code（与 node/go 版对齐）。
    //
    // A1：`ERR_NOT_FOUND:`（写探针未命中行）**不是权限错误**，故 code 取 core 的
    // machine code `not_found`（不是 ERR_PERMISSION）；本函数只定 code、不改 HTTP 状态，
    // 状态语义见 [`map_store_err`]（未命中仍走 500 透传，不臆造 404）。
    const KNOWN: [(&str, &str); 5] = [
        ("ERR_PERMISSION:", "ERR_PERMISSION"),
        // machine code 统一为 no_context（与 node/py 同口径；map_store_err 亦按此判定）
        ("ERR_NO_CONTEXT:", "no_context"),
        ("ERR_TEXT2QUERY:", "ERR_TEXT2QUERY"),
        ("ERR_GQL_PARSE:", "GQL_PARSE"),
        ("ERR_NOT_FOUND:", "not_found"),
    ];
    for (prefix, class) in KNOWN {
        // 宿主可能只给去冒号的 machine 前缀（如 code = "ERR_NOT_FOUND"），与 node/py 同口径
        if err == prefix.trim_end_matches(':') {
            return class.to_string();
        }
        if let Some(rest) = err.strip_prefix(prefix) {
            return match rest.split_once(':') {
                Some((seg, _)) if !seg.is_empty() => seg.to_string(),
                _ => class.to_string(),
            };
        }
    }
    "STORE_ERROR".to_string()
}

pub fn store_message(err: &str) -> Option<String> {
    if err.is_empty() { None } else { Some(err.to_string()) }
}

/// 判定顺序（spec/03-errors.md）：
/// 适配层守卫(400) → 权限类拒绝(403) → queryOne 空结果(404) → 其余 500 透传
pub fn map_store_err(err: &str, is_permission_error: impl Fn(&str) -> bool) -> StoreApiError {
    if is_permission_error(err) {
        return StoreApiError {
            status: StatusCode::FORBIDDEN,
            code: store_code(err),
            message: store_message(err),
        };
    }
    // spec/03 判定顺序第 3 层：权限类同档 —— NoContext（require_context 开启且 ctx 缺失，
    // core 稳定前缀 ERR_NO_CONTEXT:，machine code `no_context`）⇒ 403，与 PermissionError 同档。
    if err.starts_with("ERR_NO_CONTEXT:") {
        return StoreApiError {
            status: StatusCode::FORBIDDEN,
            code: "no_context".into(),
            message: store_message(err),
        };
    }
    // spec/03 判定顺序第 3 层：GQL 解析失败（core 稳定前缀）→ 400 GQL_PARSE，
    // message 剥前缀取原文（与 go/node/py 同语义）。
    if let Some(msg) = err.strip_prefix("ERR_GQL_PARSE:") {
        return StoreApiError {
            status: StatusCode::BAD_REQUEST,
            code: "GQL_PARSE".into(),
            message: Some(msg.to_string()),
        };
    }
    // spec/03 判定顺序第 4 层：资源无任何可读副本（core 稳定前缀 ERR_RESOURCE_NOT_FOUND:）
    // → 404 NOT_FOUND，message 剥前缀取原文（与 node/py 同构；禁按文案匹配）。
    if let Some(msg) = err.strip_prefix("ERR_RESOURCE_NOT_FOUND:") {
        return StoreApiError {
            status: StatusCode::NOT_FOUND,
            code: "NOT_FOUND".into(),
            message: Some(msg.to_string()),
        };
    }
    StoreApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: store_code(err),
        message: store_message(err),
    }
}

#[cfg(test)]
mod tests {
    use super::{map_store_err, store_code};
    use axum::http::StatusCode;

    /// A1：行不存在 ≠ 越权——不得映射为 ERR_PERMISSION / 403
    #[test]
    fn not_found_is_not_permission() {
        let err = "ERR_NOT_FOUND:目标记录不存在（写条件未命中任何行，_id 不存在或已被删除）";
        assert_eq!(store_code(err), "not_found");
        let mapped = map_store_err(err, |e| e.starts_with("ERR_PERMISSION"));
        assert_eq!(mapped.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(mapped.code, "not_found");
        assert_eq!(mapped.message.as_deref(), Some(err));
    }

    /// 对照：真正越权仍是原 code + 403
    #[test]
    fn permission_keeps_original_semantics() {
        let err = "ERR_PERMISSION:RBAC:无权写入";
        let mapped = map_store_err(err, |e| e.starts_with("ERR_PERMISSION"));
        assert_eq!(mapped.status, StatusCode::FORBIDDEN);
        assert_eq!(mapped.code, "RBAC");
        assert_eq!(mapped.message.as_deref(), Some(err));
    }

    /// 跨端 code 口径：同一 `ERR_*` 输入与 node/py store_code 同 code
    #[test]
    fn err_prefix_machine_codes() {
        assert_eq!(store_code("ERR_NOT_FOUND:目标记录不存在（写条件未命中任何行，_id 不存在或已被删除）"), "not_found");
        assert_eq!(store_code("ERR_NOT_FOUND"), "not_found");
        assert_eq!(store_code("ERR_NO_CONTEXT:缺少请求上下文"), "no_context");
        assert_eq!(store_code("ERR_NO_CONTEXT"), "no_context");
        assert_eq!(store_code("ERR_PERMISSION:RBAC:无权写入"), "RBAC");
        assert_eq!(store_code("ERR_TEXT2QUERY:档位不支持"), "ERR_TEXT2QUERY");
        assert_eq!(store_code("ERR_GQL_PARSE:意外的 token"), "GQL_PARSE");
        // 未知错误保持原样（rust 端既有口径 STORE_ERROR）
        assert_eq!(store_code("连接超时"), "STORE_ERROR");
    }
}
