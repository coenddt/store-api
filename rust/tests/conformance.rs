//! conformance runner：直接消费 ../conformance/cases/*.json 执行并断言 HTTP 行为。
//! 与 node/py/go 端同一份用例（conformance/README.md：语义变更必须先改 spec，
//! 再各端实现，再补这里的用例——四端不一致即为缺陷）。
//! 真实宿主 + 真实内存 SQLite，走完整 HTTP 栈（同 rust_store_e2e 形态）。
#![cfg(feature = "rust-store")]

use axum::body::Body;
use axum::http::{HeaderMap, Request};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::Arc;
use store_api_rs::adapter::RustStoreAdapter;
use store_api_rs::{create_router, FileOut, FileResolver, Options, UploadOut, UploadResolver};
use tower::ServiceExt;

/// 用例级适配器选项（conformance/README.md：runner 据此装配 mock resolver）
#[derive(serde::Deserialize, Default)]
struct CaseOpts {
    #[serde(default, rename = "fileResolver")]
    file_resolver: bool,
    #[serde(default, rename = "uploadResolver")]
    upload_resolver: bool,
    #[serde(default, rename = "fileResolverNotFound")]
    file_resolver_not_found: bool,
}

fn mock_file_resolver(not_found: bool) -> FileResolver {
    Arc::new(move |_h: &HeaderMap, _r: &Value, _id: &str| -> futures::future::BoxFuture<'_, Result<FileOut, String>> {
        Box::pin(async move {
            if not_found {
                Err("ERR_RESOURCE_NOT_FOUND:资源无任何可读副本".to_string())
            } else {
                Ok(FileOut {
                    body: b"mock-bytes".to_vec(),
                    content_type: None,
                    file_name: None,
                })
            }
        })
    })
}

fn mock_upload_resolver() -> UploadResolver {
    Arc::new(|_h: &HeaderMap, _r: &Value, _id: &str| -> futures::future::BoxFuture<'_, Result<UploadOut, String>> {
        Box::pin(async { Ok(UploadOut { reference: "ref://mock".into() }) })
    })
}

/// 与 cases/*.json 的格式契约一致（node/go runner 同构）
#[derive(serde::Deserialize)]
struct ConformanceCase {
    name: String,
    resource: String,
    #[serde(default)]
    opts: CaseOpts,
    steps: Vec<ConformanceStep>,
}

#[derive(serde::Deserialize)]
struct ConformanceStep {
    method: String,
    path: String,
    #[serde(default)]
    body: Option<Value>,
    #[serde(default)]
    headers: std::collections::HashMap<String, String>,
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

async fn fresh_router(resource: &str, opts: &CaseOpts) -> axum::Router {
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
            "age": { "type": "number" },
            "file": { "type": "string" },
            "images": {
                "type": "object",
                "fields": {
                    "full": { "type": "string" },
                    "thumb": { "type": "string" }
                }
            }
        }
    }))
    .expect("注册失败");
    let pool = host.sqlite_pool().expect("SQLite 源").clone();
    for ddl in [
        // 物理列名走 dialect::physical_of（逻辑名 → snake_case），与宿主持久化口径一致；
        // 用 camelCase 建表会让真实 INSERT 报 `table user has no column named created_at`。
        // `file` 承载 file 子路由的资源引用；`images` 承载 ?field= 点路径回写（spec/02），
        // object 列在 SQLite 侧存 JSON 文本（与 go runner 的 EnsureTables 口径一致）。
        &format!("CREATE TABLE \"{resource}\" (_id TEXT PRIMARY KEY, name TEXT, age REAL, file TEXT, images TEXT, created_at INTEGER, updated_at INTEGER, \"__present\" TEXT)"),
        &format!("CREATE TABLE \"{resource}_deleted\" (_id TEXT PRIMARY KEY, name TEXT, age REAL, file TEXT, images TEXT, created_at INTEGER, updated_at INTEGER, deleted_at INTEGER, \"__present\" TEXT)"),
    ] {
        sqlx::query(ddl).execute(&pool).await.expect("DDL 失败");
    }
    // 前缀由调用方 nest 控制（与 fastify prefix 机制对齐；conformance 用例带 /api 前缀）
    axum::Router::new().nest(
        "/api",
        create_router(
            Arc::new(RustStoreAdapter::new(host)),
            Options {
                resources: None,
                id_field: "_id".into(),
                context_provider: None,
                file_resolver: if opts.file_resolver || opts.file_resolver_not_found {
                    Some(mock_file_resolver(opts.file_resolver_not_found))
                } else {
                    None
                },
                upload_resolver: if opts.upload_resolver {
                    Some(mock_upload_resolver())
                } else {
                    None
                },
                ..Default::default()
            },
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
        let app = fresh_router(&c.resource, &c.opts).await;
        let mut created_id = String::new();

        for (i, step) in c.steps.iter().enumerate() {
            let uri = normalize_path(&step.path.replace("{id}", &created_id));
            let mut builder = Request::builder().method(step.method.as_str()).uri(&uri);
            // `raw:<bytes>` 是上传用例的原始字节体约定；其余 JSON body 原样序列化
            let is_raw = matches!(&step.body, Some(Value::String(s)) if s.starts_with("raw:"));
            let body_bytes = match &step.body {
                Some(Value::String(s)) if s.starts_with("raw:") => s[4..].as_bytes().to_vec(),
                Some(v) => v.to_string().into_bytes(),
                None => Vec::new(),
            };
            if step.body.is_some() && !is_raw {
                builder = builder.header("content-type", "application/json");
            }
            for (k, v) in &step.headers {
                builder = builder.header(k.as_str(), v.as_str());
            }
            let req = builder
                .body(Body::from(body_bytes))
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
