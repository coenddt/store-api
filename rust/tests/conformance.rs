//! conformance runner：直接消费 ../conformance/cases/*.json 执行并断言 HTTP 行为。
//! 与 node/py/go 端同一份用例（conformance/README.md：语义变更必须先改 spec，
//! 再各端实现，再补这里的用例——四端不一致即为缺陷）。
//! 真实宿主 + 真实内存 SQLite，走完整 HTTP 栈（同 rust_store_e2e 形态）。
#![cfg(feature = "rust-store")]

use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::Arc;
use store_api_rs::adapter::RustStoreAdapter;
use store_api_rs::{create_router, Options};
use tower::ServiceExt;

/// 与 cases/*.json 的格式契约一致（node/go runner 同构）
#[derive(serde::Deserialize)]
struct ConformanceCase {
    name: String,
    resource: String,
    steps: Vec<ConformanceStep>,
}

#[derive(serde::Deserialize)]
struct ConformanceStep {
    method: String,
    path: String,
    #[serde(default)]
    body: Option<Value>,
    #[serde(default)]
    use_created_id: bool,
    #[serde(default)]
    expect: Expect,
}

#[derive(serde::Deserialize, Default)]
struct Expect {
    #[serde(default)]
    status: u16,
    #[serde(default)]
    error_code: Option<String>,
    #[serde(default)]
    data_keys: Vec<String>,
}

async fn fresh_router(resource: &str) -> axum::Router {
    let host = Arc::new(
        rust_store::Store::connect_sqlite("sqlite::memory:")
            .await
            .expect("连接失败"),
    );
    host.register(&json!({
        "name": resource,
        "collection": resource,
        "idPrefix": "u",
        "fields": {
            "name": { "type": "string" },
            "age": { "type": "number" }
        }
    }))
    .expect("注册失败");
    let pool = host.sqlite_pool().expect("SQLite 源").clone();
    for ddl in [
        &format!("CREATE TABLE \"{resource}\" (_id TEXT PRIMARY KEY, name TEXT, age REAL, createdAt INTEGER, updatedAt INTEGER, \"__present\" TEXT)"),
        &format!("CREATE TABLE \"{resource}_deleted\" (_id TEXT PRIMARY KEY, name TEXT, age REAL, createdAt INTEGER, updatedAt INTEGER, deletedAt INTEGER, \"__present\" TEXT)"),
    ] {
        sqlx::query(ddl).execute(&pool).await.expect("DDL 失败");
    }
    // 前缀由调用方 nest 控制（与 fastify prefix 机制对齐；conformance 用例带 /api 前缀）
    axum::Router::new().nest(
        "/api",
        create_router(
            Arc::new(RustStoreAdapter::new(host)),
            Options { resources: None, id_field: "_id".into(), context_provider: None },
        )
        .await,
    )
}

/// 用例 path 规范化：query 串按惯例不预转义（含空格/大括号/引号），重编码成合法 URL
fn normalize_path(p: &str) -> String {
    match p.split_once('?') {
        None => p.to_string(),
        Some((base, q)) => {
            let reencoded: Vec<String> = q.split('&').map(|pair| {
                match pair.split_once('=') {
                    Some((k, v)) => {
                        let kk = form_urlencoded::byte_serialize(k.as_bytes()).collect::<String>();
                        let vv = form_urlencoded::byte_serialize(v.as_bytes()).collect::<String>();
                        format!("{kk}={vv}")
                    }
                    None => form_urlencoded::byte_serialize(pair.as_bytes()).collect(),
                }
            }).collect();
            format!("{base}?{}", reencoded.join("&"))
        }
    }
}

#[tokio::test]
async fn conformance_cases() {
    let cases_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../conformance/cases");
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(cases_dir)
        .expect("conformance 用例目录")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "至少应有一份用例");

    for path in &paths {
        let raw = std::fs::read_to_string(path).expect("读用例");
        let c: ConformanceCase = serde_json::from_str(&raw).expect("用例 JSON 解析");
        let app = fresh_router(&c.resource).await;
        let mut created_id = String::new();

        for (i, step) in c.steps.iter().enumerate() {
            let uri = normalize_path(&step.path.replace("{id}", &created_id));
            let mut builder = Request::builder().method(step.method.as_str()).uri(&uri);
            if step.body.is_some() {
                builder = builder.header("content-type", "application/json");
            }
            let req = builder
                .body(Body::from(step.body.as_ref().map(|v| v.to_string()).unwrap_or_default()))
                .expect("构造请求");
            let res = app.clone().oneshot(req).await.expect("请求");
            let status = res.status().as_u16();
            let bytes = res.into_body().collect().await.expect("body").to_bytes();
            let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

            assert_eq!(status, step.expect.status, "case {} step {} ({} {}) 响应: {}",
                c.name, i, step.method, uri, String::from_utf8_lossy(&bytes));
            if let (Some(want), true) = (&step.expect.error_code, body["error"].is_object()) {
                assert_eq!(body["error"]["code"], *want,
                    "case {} step {} error.code", c.name, i);
            }
            if !step.expect.data_keys.is_empty() {
                for k in &step.expect.data_keys {
                    assert!(body["data"].get(k).is_some(),
                        "case {} step {} data 缺键 {}（响应: {}）",
                        c.name, i, k, String::from_utf8_lossy(&bytes));
                }
            }
            if let Some(id) = body["data"]["_id"].as_str() {
                if !id.is_empty() {
                    created_id = id.to_string();
                }
            }
        }
        println!("conformance {}: {} steps 全部通过", c.name, c.steps.len());
    }
}
