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
    // 首个冒号前缀（如 ERR_PERMISSION / ERR_TEXT2QUERY）即分类；无前缀则整体为码
    err.split(':').next().unwrap_or("STORE_ERROR").to_string()
}

pub fn store_message(err: &str) -> Option<String> {
    if err.is_empty() { None } else { Some(err.to_string()) }
}

/// 判定顺序（spec/03-errors.md）：
/// 适配层守卫(400) → 权限拒绝(403) → queryOne 空结果(404) → 其余 500 透传
pub fn map_store_err(err: &str, is_permission_error: impl Fn(&str) -> bool) -> StoreApiError {
    if is_permission_error(err) {
        return StoreApiError {
            status: StatusCode::FORBIDDEN,
            code: store_code(err),
            message: store_message(err),
        };
    }
    StoreApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: store_code(err),
        message: store_message(err),
    }
}
