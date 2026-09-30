//! 端到端桥接测试（feature = "rust-store"）：真实 rust-store 宿主 + 真实内存 SQLite，
//! 走完整 HTTP 栈验证 store-api 垂直链路：axum → Store trait → 适配器 → 宿主 → core → sqlx。
#![cfg(feature = "rust-store")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::Arc;
use store_api_rs::adapter::RustStoreAdapter;
use store_api_rs::{create_router, Options};
use tower::ServiceExt;

async fn fresh_router() -> axum::Router {
    let host = Arc::new(
        rust_store::Store::connect_sqlite("sqlite::memory:")
            .await
            .expect("连接失败"),
    );
    host.register(&json!({
        "name": "user",
        "collection": "user",
        "idPrefix": "u",
        "fields": {
            "name": { "type": "string" },
            "age": { "type": "number" }
        }
    }))
    .expect("注册失败");

    // SQL 后端物理 DDL（含 __present 哨兵列契约）
    let pool = host.sqlite_pool().clone();
    for ddl in [
        "CREATE TABLE \"user\" (_id TEXT PRIMARY KEY, name TEXT, age REAL, createdAt INTEGER, updatedAt INTEGER, \"__present\" TEXT)",
        "CREATE TABLE \"user_deleted\" (_id TEXT PRIMARY KEY, name TEXT, age REAL, createdAt INTEGER, updatedAt INTEGER, deletedAt INTEGER, \"__present\" TEXT)",
    ] {
        sqlx::query(ddl).execute(&pool).await.expect("DDL 失败");
    }
    POOL.set(pool).expect("池只设一次");

    // 上下文提供者：x-user 请求头 → core 上下文 JSON（userId 键，见 core context_from_value）
    let provider: store_api_rs::ContextProvider = Arc::new(|headers: &axum::http::HeaderMap| {
        let who = headers.get("x-user").and_then(|v| v.to_str().ok()).map(String::from);
        Box::pin(async move { Ok(who.map(|u| json!({ "userId": u, "roles": [u] }))) }) as futures::future::BoxFuture<_>
    });

    let adapter = Arc::new(RustStoreAdapter::new(host));
    create_router(
        adapter,
        Options {
            resources: None,
            id_field: "_id".into(),
            context_provider: Some(provider),
        },
    )
    .await
}

static POOL: std::sync::OnceLock<sqlx::SqlitePool> = std::sync::OnceLock::new();

async fn send(app: axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    let req = builder
        .body(Body::from(body.map(|v| v.to_string()).unwrap_or_default()))
        .unwrap();
    let res = app.oneshot(req).await.expect("oneshot 失败");
    let status = res.status();
    let bytes = res.into_body().collect().await.expect("body").to_bytes();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, json)
}

#[tokio::test]
async fn real_host_http_full_chain() {
    let app = fresh_router().await;

    // POST → 201，宿主生成 idPrefix ID + 时间戳
    let (status, body) = send(app.clone(), "POST", "/user", Some(json!({ "name": "alice", "age": 30 }))).await;
    assert_eq!(status, StatusCode::CREATED, "body = {body}");
    assert_eq!(body["data"]["name"], "alice");
    let id = body["data"]["_id"].as_str().expect("应有宿主生成的 _id").to_string();
    assert!(id.starts_with('u'), "idPrefix 应生效: {id}");
    assert!(body["data"]["createdAt"].is_i64(), "时间戳应生效");

    // GET 列表：GQL 条件经 core 规划 + SQL 翻译 + 行还原
    let (status, body) = send(
        app.clone(),
        "GET",
        "/user?q=($condition:%20@c0)%20%7B%20name%2C%20age%20%7D&p.c0=%7B%22age%22%3A%7B%22%24gte%22%3A18%7D%7D",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body = {body}");
    assert_eq!(body["data"].as_array().expect("数组").len(), 1);
    assert_eq!(body["data"][0]["name"], "alice");

    // PATCH（探针重入 + RETURNING 回读）
    let (status, body) = send(app.clone(), "PATCH", &format!("/user/{id}"), Some(json!({ "age": 31 }))).await;
    assert_eq!(status, StatusCode::OK, "body = {body}");
    assert_eq!(body["data"]["age"].as_f64(), Some(31.0));

    // 权限垂直链路：guest 上下文（contextProvider 注入）→ 宿主 core 拒绝写 → 403
    let req = Request::builder()
        .method("PATCH")
        .uri(format!("/user/{id}"))
        .header("content-type", "application/json")
        .header("x-user", "guest")
        .body(Body::from(json!({ "age": 1 }).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN, "guest 写入应被拒");
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["error"]["code"], "ERR_PERMISSION");

    // DELETE（归档+删除事务化）→ 200
    let (status, body) = send(app.clone(), "DELETE", &format!("/user/{id}"), None).await;
    assert_eq!(status, StatusCode::OK, "body = {body}");
    assert_eq!(body["data"]["deletedCount"], 1);
    assert_eq!(body["data"]["archivedCount"], 1);

    // 删除后 404
    let (status, body) = send(app.clone(), "GET", &format!("/user/{id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "NOT_FOUND");

    // 归档表确有记录（直查 SQLite）
    let pool = POOL.get().expect("池未初始化");
    let archived: Option<(String,)> = sqlx::query_as("SELECT name FROM \"user_deleted\" LIMIT 1")
        .fetch_optional(pool)
        .await
        .expect("归档查询失败");
    assert_eq!(archived.map(|r| r.0), Some("alice".into()));
}
