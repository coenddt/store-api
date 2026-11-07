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
    /// schema 结构定义（宿主 `Store::schema_definition` 的镜像）：`{name, collection, fields}`，
    /// `fields` 每项含 `type` / `fields`（嵌套 object，可递归）。未知 schema 返回 `Err`。
    /// 用于 spec/02 决策 S3 的上传 `?field=` 白名单（类型语义留在协议皮，宿主只暴露信息）。
    async fn schema_definition(&self, _schema: &str) -> Result<Value, StoreErr> {
        Err("schema_definition 未实现".into())
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

/// 下载 resolver 产物（spec/05：`contentType` / `fileName` 由 resolver 决定，皮不推断 MIME）
#[derive(Debug, Clone)]
pub struct FileOut {
    pub body: Vec<u8>,
    pub content_type: Option<String>,
    pub file_name: Option<String>,
}

/// 上传 resolver 产物（`ref` 回写到记录字段；缺省视为未实现 → 500 透传，禁静默）
#[derive(Debug, Clone)]
pub struct UploadOut {
    pub reference: String,
}

/// 下载接缝（spec/03 决策 D）：未注入 ⇒ 501 `FILE_NOT_CONFIGURED`
pub type FileResolver =
    Arc<dyn for<'a> Fn(&'a HeaderMap, &'a Value, &'a str) -> BoxFuture<'a, Result<FileOut, String>> + Send + Sync>;

/// 上传接缝：未注入 ⇒ 501 `UPLOAD_NOT_CONFIGURED`
pub type UploadResolver =
    Arc<dyn for<'a> Fn(&'a HeaderMap, &'a Value, &'a str) -> BoxFuture<'a, Result<UploadOut, String>> + Send + Sync>;

#[derive(Clone, Default)]
pub struct Options {
    /// 显式资源名；缺省取 store.list() 并过滤归档表
    pub resources: Option<Vec<String>>,
    /// 单条路由主键字段名（spec/01，三端一致，默认 '_id'）
    pub id_field: String,
    pub context_provider: Option<ContextProvider>,
    /// file 子路由缺省字段点路径（spec/02 `field` 的缺省回落，默认 'file'）
    pub file_field: Option<String>,
    pub file_resolver: Option<FileResolver>,
    pub upload_resolver: Option<UploadResolver>,
    /// 上传字节体上限（默认 32 MiB，超限 413）
    pub upload_limit: Option<usize>,
}

#[derive(Clone)]
struct AppState {
    store: Arc<dyn Store>,
    resources: Vec<String>,
    id_field: Arc<str>,
    context_provider: Option<ContextProvider>,
    file_field: Arc<str>,
    file_resolver: Option<FileResolver>,
    upload_resolver: Option<UploadResolver>,
    upload_limit: usize,
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
        file_field: Arc::from(opts.file_field.unwrap_or_else(|| "file".into()).as_str()),
        file_resolver: opts.file_resolver,
        upload_resolver: opts.upload_resolver,
        upload_limit: opts.upload_limit.unwrap_or(32 * 1024 * 1024),
    };
    Router::new()
        .route("/{resource}", get(list_resource).post(create_resource))
        .route("/{resource}/{id}", get(get_one).patch(update_one).delete(delete_one))
        .route(
            "/{resource}/{id}/file",
            get(download_file).post(upload_file),
        )
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

/// spec/02：`field` 为点路径，缺省回落适配器选项 `file_field`（空串视同未传）
fn field_path_of(raw: Option<&str>, fallback: &str) -> String {
    raw.and_then(|s| {
        form_urlencoded::parse(s.as_bytes())
            .find(|(k, _)| k == "field")
            .map(|(_, v)| v.to_string())
    })
    .filter(|s| !s.is_empty())
    .unwrap_or_else(|| fallback.to_string())
}

/// spec/02 决策 S3：上传写入路径白名单的禁用段（主键与原型污染向量）。
/// `?field=` 必须精确命中「schema 声明为 string|file 的叶子点路径」，
/// 未声明 / object 中间节点 / 非 string|file 类型一律 400 `INVALID_PARAM`。
const FORBIDDEN_FIELD_SEGMENTS: [&str; 4] = ["_id", "__proto__", "constructor", "prototype"];

/// 递归收集可写叶子点路径（与 node `allowedFieldPaths` / py `_allowed_field_paths` 同构）：
/// object 下钻为 `a.b`，只收 `string` / `file` 标量，跳过禁用段与 `$` 前缀。
fn allowed_field_paths(fields: &Map<String, Value>, prefix: &str, out: &mut Vec<String>) {
    for (key, defn) in fields {
        if key.starts_with('$') || FORBIDDEN_FIELD_SEGMENTS.contains(&key.as_str()) {
            continue;
        }
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        if let Some(nested) = defn.get("fields").and_then(|v| v.as_object()) {
            allowed_field_paths(nested, &path, out);
            continue;
        }
        let ty = defn.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if ty == "string" || ty == "file" {
            out.push(path);
        }
    }
}

/// 上传 field 白名单校验（spec/02 决策 S3）。失败返回 400 `INVALID_PARAM` 且带可用列表；
/// 取不到 schema 定义属配置/宿主故障 ⇒ 500 `STORE_ERROR`（禁静默放行）。
async fn check_upload_field(
    state: &AppState,
    resource: &str,
    field_path: &str,
) -> Result<(), StoreApiError> {
    let defn = state.store.schema_definition(resource).await.map_err(|e| StoreApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: "STORE_ERROR".into(),
        message: Some(format!("读取 schema 定义失败({resource}): {e}")),
    })?;
    let mut allowed = Vec::new();
    if let Some(fields) = defn.get("fields").and_then(|v| v.as_object()) {
        allowed_field_paths(fields, "", &mut allowed);
    }
    allowed.sort();
    allowed.dedup();
    if allowed.iter().any(|p| p == field_path) {
        return Ok(());
    }
    let list = if allowed.is_empty() {
        "（无）".to_string()
    } else {
        allowed.join(", ")
    };
    Err(StoreApiError::invalid_param(format!(
        "field 不在可写白名单: {field_path}；可用: {list}（spec/02 决策 S3）"
    )))
}

/// 点路径取值：任一段缺失 ⇒ None（spec/02：视同「文件字段为空」）
fn pick_field<'a>(rec: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = rec;
    for seg in path.split('.') {
        cur = cur.get(seg)?;
        if cur.is_null() {
            return None;
        }
    }
    Some(cur)
}

/// file 子路由投影：schema 标量字段 ∪ `field` 点路径根字段
/// （schema 未声明资源引用字段时仍能取到，与 core「未声明字段按裸列名宽松处理」一致）
async fn file_projection(state: &AppState, resource: &str, field_path: &str) -> String {
    let mut fields = state.store.schema_fields(resource).await;
    let root = field_path.split('.').next().unwrap_or(field_path);
    if !root.is_empty() && !fields.iter().any(|f| f == root) {
        fields.push(root.to_string());
    }
    if fields.is_empty() {
        String::new()
    } else {
        format!(" {{ {} }}", fields.join(", "))
    }
}

/// `GET /{resource}/{id}/file`（spec/03 判定顺序：未注入 resolver 501 → 记录不存在 404
/// → 字段为空 404 → resolver 错误按 core 前缀分类 → 字节流）
async fn download_file(
    State(state): State<AppState>,
    Path((resource, id)): Path<(String, String)>,
    RawQuery(raw): RawQuery,
    Extension(rc): Extension<RequestCtx>,
    headers: HeaderMap,
) -> Response {
    let Some(resolver) = state.file_resolver.clone() else {
        return StoreApiError::file_not_configured(&format!(
            "GET /{resource}/{id}/file 未注入 fileResolver"
        ))
        .into_response();
    };
    if let Err(e) = resolve_resource(&state, &resource) {
        return e.into_response();
    }
    let field_path = field_path_of(raw.as_deref(), &state.file_field);
    let mut params = Map::new();
    params.insert("c0".into(), json!({ state.id_field.as_ref(): id }));
    let proj = file_projection(&state, &resource, &field_path).await;
    let gql = format!("{resource}($condition: @c0){proj}");
    let rec = match state.store.query_one(&gql, &params, rc.0).await {
        Ok(Some(doc)) => doc,
        Ok(None) => {
            return StoreApiError::not_found(format!(
                "记录不存在: {}={id}",
                state.id_field
            ))
            .into_response()
        }
        Err(e) => return store_err_response(&state, &e),
    };
    if pick_field(&rec, &field_path).is_none() {
        return StoreApiError::not_found(format!("文件不存在: {field_path}={id}")).into_response();
    }
    let out = match resolver(&headers, &rec, &id).await {
        Ok(o) => o,
        // resolver 抛 ERR_RESOURCE_NOT_FOUND: ⇒ 404（map_store_err 分类）；其余 500 透传
        Err(e) => return store_err_response(&state, &e),
    };
    let mut res = Response::new(axum::body::Body::from(out.body));
    let ct = out.content_type.unwrap_or_else(|| "application/octet-stream".into());
    let file_name = out.file_name.unwrap_or_else(|| "file".into());
    let ct = match axum::http::HeaderValue::from_str(&ct) {
        Ok(v) => v,
        Err(e) => {
            return StoreApiError::invalid_param(format!("contentType 不是合法 header 值: {e}"))
                .into_response()
        }
    };
    let cd = match axum::http::HeaderValue::from_str(&format!("attachment; filename=\"{file_name}\"")) {
        Ok(v) => v,
        Err(e) => {
            return StoreApiError::invalid_param(format!("fileName 不是合法 header 值: {e}"))
                .into_response()
        }
    };
    res.headers_mut().insert(axum::http::header::CONTENT_TYPE, ct);
    res.headers_mut().insert(axum::http::header::CONTENT_DISPOSITION, cd);
    res
}

/// `POST /{resource}/{id}/file`（spec/03 判定顺序：未注入 resolver 501 → 空体 400 /
/// 超限 413 → 记录不存在 404 → resolver 未回 ref 500 → 回写 update）
async fn upload_file(
    State(state): State<AppState>,
    Path((resource, id)): Path<(String, String)>,
    RawQuery(raw): RawQuery,
    Extension(rc): Extension<RequestCtx>,
    headers: HeaderMap,
    request: axum::extract::Request,
) -> Response {
    let Some(resolver) = state.upload_resolver.clone() else {
        return StoreApiError::upload_not_configured(&format!(
            "POST /{resource}/{id}/file 未注入 uploadResolver"
        ))
        .into_response();
    };
    if let Err(e) = resolve_resource(&state, &resource) {
        return e.into_response();
    }
    // 多读 1 字节即可判定超限；read 失败的唯一成因是 length limit
    let limit = state.upload_limit;
    let body = match axum::body::to_bytes(request.into_body(), limit + 1).await {
        Ok(b) => b,
        Err(e) => {
            return StoreApiError::too_large(&format!("上传字节体超过上限 {limit} 字节: {e}"))
                .into_response()
        }
    };
    if body.is_empty() {
        return StoreApiError::empty_body(&format!("上传请求体为空: POST /{resource}/{id}/file"))
            .into_response();
    }
    if body.len() > limit {
        return StoreApiError::too_large(&format!("上传字节体 {} 字节超过上限 {limit} 字节", body.len()))
            .into_response();
    }
    let field_path = field_path_of(raw.as_deref(), &state.file_field);
    if let Err(e) = check_upload_field(&state, &resource, &field_path).await {
        return e.into_response();
    }
    let mut params = Map::new();
    params.insert("c0".into(), json!({ state.id_field.as_ref(): id }));
    let proj = file_projection(&state, &resource, &field_path).await;
    let gql = format!("{resource}($condition: @c0){proj}");
    let rec = match state.store.query_one(&gql, &params, rc.0.clone()).await {
        Ok(Some(doc)) => doc,
        Ok(None) => {
            return StoreApiError::not_found(format!(
                "记录不存在: {}={id}",
                state.id_field
            ))
            .into_response()
        }
        Err(e) => return store_err_response(&state, &e),
    };
    let out = match resolver(&headers, &rec, &id).await {
        Ok(o) => o,
        Err(e) => return store_err_response(&state, &e),
    };
    if out.reference.is_empty() {
        // 禁静默：resolver 未给出引用即请求期显式失败
        return StoreApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "STORE_ERROR".into(),
            message: Some(format!(
                "uploadResolver 未返回 ref: {resource}({}={id})",
                state.id_field
            )),
        }
        .into_response();
    }
    // 回写走 update 语义 → RBAC 写权限自动生效；键用点路径原样（store 非操作符键自动包 $set）
    let mut data = Map::new();
    data.insert(field_path.clone(), json!(out.reference));
    let condition = json!({ state.id_field.as_ref(): id });
    match state.store.update(&resource, &condition, &Value::Object(data), rc.0).await {
        Ok(doc) => ok_json(json!({ "data": doc })),
        Err(e) => store_err_response(&state, &e),
    }
}
