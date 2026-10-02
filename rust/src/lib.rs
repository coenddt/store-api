//! store-api-rs — axum 适配器：为已注册 schema 自动生成 RESTful 路由。
//!
//! 路由/参数/错误/上下文语义全部以 spec/*.md 为唯一依据（三端 parity，改动先改 spec）。
//!
//! ```ignore
//! let router = create_router(Arc::new(MyStore::new()), Options::default());
//! axum::serve(listener, router).await?;
//! ```

pub mod errors;
pub mod params;

#[cfg(feature = "rust-store")]
pub mod adapter;

use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::{Extension, Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use futures::future::BoxFuture;
use serde_json::{json, Map, Value};

use errors::{map_store_err, StoreApiError};

/// store 层错误（对齐 rust-store 宿主：`Result<_, String>`；权限错误带 `ERR_PERMISSION:` 前缀）
pub type StoreErr = String;

/// 数据层调用面 trait —— rust-store 宿主（`rust_store::Store`）的直接镜像，
/// 与 nodejs-store / py-store 的 store 实例同构。真实接入实现本 trait 包一层即可。
///
/// 上下文为**显式传参**（`ctx: Option<Value>`，对齐宿主的入参模型）：中间件把每请求
/// 上下文放进 request extensions，handler 取出后随调用传递。禁止在适配层用全局状态
/// 持有「当前上下文」——并发请求会互相覆盖（spec/04 警告的身份跨请求泄漏）。
#[async_trait]
pub trait Store: Send + Sync {
    async fn list(&self) -> Vec<String>;
    async fn query(&self, gql: &str, params: &Map<String, Value>, ctx: Option<Value>) -> Result<Vec<Value>, StoreErr>;
    async fn query_one(&self, gql: &str, params: &Map<String, Value>, ctx: Option<Value>) -> Result<Option<Value>, StoreErr>;
    async fn insert(&self, schema: &str, data: &Value, ctx: Option<Value>) -> Result<Value, StoreErr>;
    async fn update(&self, schema: &str, condition: &Value, data: &Value, ctx: Option<Value>) -> Result<Option<Value>, StoreErr>;
    async fn remove(&self, schema: &str, condition: &Value, ctx: Option<Value>) -> Result<Value, StoreErr>;
    /// schema 的标量字段名列表（spec/01+02：适配器据此生成显式投影——
    /// GQL 省略投影段 = 只返回 _id。空 Vec = 无投影，data 仅 _id 属上游 schema 定义不完整的显式后果）
    async fn schema_fields(&self, _schema: &str) -> Vec<String> {
        Vec::new()
    }
    /// 权限错误判定（spec/03：按类型判定。Rust 形态 = core `ERR_PERMISSION:` 前缀契约）
    fn is_permission_error(&self, err: &StoreErr) -> bool {
        err.starts_with("ERR_PERMISSION")
    }
}

/// 每请求上下文载体（request extensions 中转；None = 本请求无上下文）
#[derive(Clone, Default)]
pub struct RequestCtx(pub Option<Value>);

/// 归档表过滤（spec/01-routing.md）：`XxxDeleted` 且 `Xxx` 也在列表中 ⇒ 视为归档表。
pub fn filter_archived(names: Vec<String>) -> Vec<String> {
    let set: std::collections::HashSet<String> = names.iter().cloned().collect();
    names
        .into_iter()
        .filter(|n| {
            !(n.ends_with("Deleted") && set.contains(&n[..n.len() - "Deleted".len()].to_string()))
        })
        .collect()
}

/// 每请求上下文钩子（spec/04）：返回 Ok(Some(ctx)) 注入；Ok(None) 不注入；
/// Err(权限) → 403，Err(其余) → 401 CONTEXT_ERROR
pub type ContextProvider =
    Arc<dyn for<'a> Fn(&'a HeaderMap) -> BoxFuture<'a, Result<Option<Value>, String>> + Send + Sync>;

#[derive(Clone, Default)]
pub struct Options {
    /// 显式资源名；缺省取 store.list() 并过滤归档表
    pub resources: Option<Vec<String>>,
    /// 单条路由主键字段名（spec/01，三端一致，默认 '_id'）
    pub id_field: String,
    pub context_provider: Option<ContextProvider>,
}

#[derive(Clone)]
struct AppState {
    store: Arc<dyn Store>,
    resources: Vec<String>,
    id_field: Arc<str>,
    context_provider: Option<ContextProvider>,
}

/// 生成 RESTful Router（前缀由调用方 `.nest("/api", router)` 自行控制，与 fastify prefix 机制对齐）
pub async fn create_router(store: Arc<dyn Store>, opts: Options) -> Router {
    let resources = match opts.resources.clone() {
        Some(r) => r,
        None => filter_archived(store.list().await),
    };
    let state = AppState {
        store,
        resources,
        id_field: Arc::from(opts.id_field.as_str()),
        context_provider: opts.context_provider,
    };
    Router::new()
        .route("/{resource}", get(list_resource).post(create_resource))
        .route("/{resource}/{id}", get(get_one).patch(update_one).delete(delete_one))
        .layer(middleware::from_fn_with_state(state.clone(), inject_context))
        .with_state(state)
}

/// spec/04：上下文中间件。PermissionError ⇒ 403（RBAC 拒绝）；其余 ⇒ 401 CONTEXT_ERROR。
async fn inject_context(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut request: axum::extract::Request,
    next: Next,
) -> Response {
    let ctx = match &state.context_provider {
        None => None,
        Some(provider) => match provider(&headers).await {
            Ok(c) => c,
            Err(e) => {
                if state.store.is_permission_error(&e.to_string()) {
                    let mapped = map_store_err(&e, |x| state.store.is_permission_error(&x.to_string()));
                    return mapped.into_response();
                }
                return StoreApiError::context_error(if e.is_empty() { None } else { Some(e) })
                    .into_response();
            }
        },
    };
    // spec/04：ctx 显式随请求传递（None = 本请求无上下文，由 store 的 requireContext
    // 档位决定是否拒绝）——适配层无全局状态，天然无跨请求身份残留。
    request.extensions_mut().insert(RequestCtx(ctx));
    next.run(request).await
}

fn ok_json(v: Value) -> Response {
    (StatusCode::OK, Json(v)).into_response()
}

/// 资源名校验：path 中的资源必须在注册清单内（否则 404，防御未知资源穿透）
fn resolve_resource(state: &AppState, resource: &str) -> Result<(), StoreApiError> {
    if state.resources.iter().any(|r| r == resource) {
        Ok(())
    } else {
        Err(StoreApiError {
            status: StatusCode::NOT_FOUND,
            code: "RESOURCE_NOT_FOUND".into(),
            message: Some(format!("资源未注册: {resource}")),
        })
    }
}

fn store_err_response(state: &AppState, err: &StoreErr) -> Response {
    let mapped = map_store_err(err, |x| state.store.is_permission_error(&x.to_string()));
    mapped.into_response()
}

async fn list_resource(
    State(state): State<AppState>,
    Path(resource): Path<String>,
    RawQuery(raw): RawQuery,
    Extension(rc): Extension<RequestCtx>,
) -> Response {
    if let Err(e) = resolve_resource(&state, &resource) {
        return e.into_response();
    }
    let params = match raw.as_deref().map(params::parse_query_params) {
        Some(Err(msg)) => return StoreApiError::invalid_param(msg).into_response(),
        Some(Ok(p)) => p,
        None => Map::new(),
    };
    let q = raw.as_deref().and_then(|s| extract_q(s)).unwrap_or_default();
    // spec/02：q 缺失 → 全字段投影（省略投影 = 只回 _id）；q 存在 → 投影完全由 q 决定
    let gql = if q.is_empty() {
        let proj = schema_projection(&state, &resource).await;
        format!("{resource}{proj}")
    } else {
        format!("{resource}{q}")
    };
    match state.store.query(&gql, &params, rc.0).await {
        Ok(rows) => ok_json(json!({ "data": rows })),
        Err(e) => store_err_response(&state, &e),
    }
}

/// 显式投影串（spec/01+02）：字段列表来自 store 元数据；空列表 → 无投影（data 仅 _id，显式后果）
async fn schema_projection(state: &AppState, resource: &str) -> String {
    let fields = state.store.schema_fields(resource).await;
    if fields.is_empty() {
        String::new()
    } else {
        format!(" {{ {} }}", fields.join(", "))
    }
}

/// 从原始 query string 取 `q` 参数的原始值（不 decode 两次——form_urlencoded 已解）
fn extract_q(query: &str) -> Option<String> {
    form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == "q")
        .map(|(_, v)| v.to_string())
}

async fn get_one(
    State(state): State<AppState>,
    Path((resource, id)): Path<(String, String)>,
    Extension(rc): Extension<RequestCtx>,
) -> Response {
    if let Err(e) = resolve_resource(&state, &resource) {
        return e.into_response();
    }
    let mut params = Map::new();
    params.insert("c0".into(), json!({ state.id_field.as_ref(): id }));
    let proj = schema_projection(&state, &resource).await;
    let gql = format!("{resource}($condition: @c0){proj}");
    match state.store.query_one(&gql, &params, rc.0).await {
        Ok(Some(doc)) => ok_json(json!({ "data": doc })),
        Ok(None) => StoreApiError::not_found(format!("记录不存在: {}={}", state.id_field, id))
            .into_response(),
        Err(e) => store_err_response(&state, &e),
    }
}

async fn read_json_body(request: axum::extract::Request) -> Result<Value, StoreApiError> {
    let body = axum::body::to_bytes(request.into_body(), 4 << 20)
        .await
        .map_err(|e| StoreApiError::invalid_body(&format!("请求体读取失败: {e}")))?;
    if body.is_empty() {
        return Err(StoreApiError::invalid_body("请求体必须是 JSON 对象"));
    }
    let v: Value = serde_json::from_slice(&body)
        .map_err(|e| StoreApiError::invalid_body(&format!("请求体不是合法 JSON: {e}")))?;
    if !v.is_object() {
        return Err(StoreApiError::invalid_body("请求体必须是 JSON 对象"));
    }
    Ok(v)
}

async fn create_resource(
    State(state): State<AppState>,
    Path(resource): Path<String>,
    Extension(rc): Extension<RequestCtx>,
    request: axum::extract::Request,
) -> Response {
    if let Err(e) = resolve_resource(&state, &resource) {
        return e.into_response();
    }
    let body = match read_json_body(request).await {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    match state.store.insert(&resource, &body, rc.0).await {
        Ok(created) => (StatusCode::CREATED, Json(json!({ "data": created }))).into_response(),
        Err(e) => store_err_response(&state, &e),
    }
}

async fn update_one(
    State(state): State<AppState>,
    Path((resource, id)): Path<(String, String)>,
    Extension(rc): Extension<RequestCtx>,
    request: axum::extract::Request,
) -> Response {
    if let Err(e) = resolve_resource(&state, &resource) {
        return e.into_response();
    }
    let body = match read_json_body(request).await {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let condition = json!({ state.id_field.as_ref(): id });
    match state.store.update(&resource, &condition, &body, rc.0).await {
        Ok(Some(doc)) => ok_json(json!({ "data": doc })),
        Ok(None) => ok_json(json!({ "data": Value::Null })),
        Err(e) => store_err_response(&state, &e),
    }
}

async fn delete_one(
    State(state): State<AppState>,
    Path((resource, id)): Path<(String, String)>,
    Extension(rc): Extension<RequestCtx>,
) -> Response {
    if let Err(e) = resolve_resource(&state, &resource) {
        return e.into_response();
    }
    let condition = json!({ state.id_field.as_ref(): id });
    match state.store.remove(&resource, &condition, rc.0).await {
        Ok(out) => ok_json(json!({ "data": out })),
        Err(e) => store_err_response(&state, &e),
    }
}
