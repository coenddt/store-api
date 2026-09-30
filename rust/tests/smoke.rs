//! smoke 测试：mock store（镜像 node/test/smoke.test.js 与 py/tests/test_smoke.py 的用例矩阵）。
//! conformance 纪律：三端断言逐项对应，任何一端改动必须同步其余两端。

use std::sync::Mutex;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Map, Value};
use store_api_rs::{create_router, Options, Store, StoreErr};
use tower::ServiceExt;

#[derive(Default)]
struct MockStore {
    rows: Mutex<Vec<Value>>,
    last_query: Mutex<Option<(String, Value)>>,
    last_context: Mutex<Option<Value>>,
}

impl MockStore {
    fn is_perm(err: &str) -> bool {
        err.starts_with("ERR_PERMISSION")
    }
}

#[async_trait]
impl Store for MockStore {
    async fn list(&self) -> Vec<String> {
        vec!["user".into(), "userDeleted".into()]
    }

    async fn schema_fields(&self, schema: &str) -> Vec<String> {
        if schema == "user" {
            vec!["name".into(), "age".into()]
        } else {
            Vec::new()
        }
    }

    async fn query(&self, gql: &str, params: &Map<String, Value>) -> Result<Vec<Value>, StoreErr> {
        *self.last_query.lock().unwrap() = Some((gql.to_string(), Value::Object(params.clone())));
        Ok(self.rows.lock().unwrap().clone())
    }

    async fn query_one(&self, gql: &str, params: &Map<String, Value>) -> Result<Option<Value>, StoreErr> {
        *self.last_query.lock().unwrap() = Some((gql.to_string(), Value::Object(params.clone())));
        let id = params.get("c0").and_then(|c| c.get("_id")).and_then(|v| v.as_str());
        Ok(self
            .rows
            .lock()
            .unwrap()
            .iter()
            .find(|r| r["_id"].as_str() == id)
            .cloned())
    }

    async fn insert(&self, _schema: &str, data: &Value) -> Result<Value, StoreErr> {
        let mut rows = self.rows.lock().unwrap();
        let mut row = data.clone();
        if row.get("_id").is_none() {
            row["_id"] = json!(format!("u{}", rows.len() + 1));
        }
        let id = row["_id"].clone();
        row["name"] = row.get("name").cloned().unwrap_or(Value::Null);
        rows.push(row.clone());
        let _ = id;
        Ok(row)
    }

    async fn update(&self, _schema: &str, condition: &Value, data: &Value) -> Result<Option<Value>, StoreErr> {
        let id = condition["_id"].as_str().unwrap_or_default();
        let mut rows = self.rows.lock().unwrap();
        match rows.iter_mut().find(|r| r["_id"].as_str() == Some(id)) {
            Some(row) => {
                if let Some(obj) = data.as_object() {
                    for (k, v) in obj {
                        row[k.clone()] = v.clone();
                    }
                }
                Ok(Some(row.clone()))
            }
            None => Ok(None),
        }
    }

    async fn remove(&self, _schema: &str, condition: &Value) -> Result<Value, StoreErr> {
        let id = condition["_id"].as_str().unwrap_or_default().to_string();
        let mut rows = self.rows.lock().unwrap();
        let before = rows.len();
        rows.retain(|r| r["_id"].as_str() != Some(&id));
        Ok(json!({ "deletedCount": before - rows.len(), "archivedCount": 1 }))
    }

    async fn set_context(&self, ctx: Value) {
        *self.last_context.lock().unwrap() = Some(ctx);
    }

    fn is_permission_error(&self, err: &StoreErr) -> bool {
        Self::is_perm(err)
    }
}

async fn app() -> axum::Router {
    let store = MockStore::default();
    // 显式标注 ContextProvider 类型：HRTB 闭包（for<'a> Fn(&'a HeaderMap) -> BoxFuture<'a, _>）
    let provider_ctx: store_api_rs::ContextProvider = std::sync::Arc::new(|headers: &axum::http::HeaderMap| {
        let who = headers
            .get("x-user")
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        Box::pin(async move {
            match who.as_deref() {
                Some("denied") => Err("ERR_PERMISSION:无写入权限".to_string()),
                Some("broken") => Err("token 解析失败".to_string()),
                Some(u) => Ok(Some(json!({ "uid": u }))),
                None => Ok(None),
            }
        }) as futures::future::BoxFuture<_>
    });
    create_router(
        // create_router 为 async（资源清单取自 store.list()）
        std::sync::Arc::new(store),
        Options {
            resources: None,
            id_field: "_id".into(),
            context_provider: Some(provider_ctx),
        },
    )
    .await
}

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
async fn crud_full_chain() {
    let app = app().await;

    // POST → 201 + 自动 _id
    let (status, body) = send(app.clone(), "POST", "/user", Some(json!({ "name": "a", "age": 1 }))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["data"]["name"], "a");
    let id = body["data"]["_id"].as_str().expect("应有 _id").to_string();

    // GET 列表：q 透传 + p.* 绑定（拼接结果 = 资源名 + q 原样）
    let (status, body) = send(
        app.clone(),
        "GET",
        "/user?q=($condition:%20@c0)&p.c0=%7B%22age%22%3A%7B%22%24gte%22%3A18%7D%7D",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["data"].is_array());

    // GET 单条
    let (status, body) = send(app.clone(), "GET", &format!("/user/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["name"], "a");

    // PATCH
    let (status, body) = send(app.clone(), "PATCH", &format!("/user/{id}"), Some(json!({ "age": 2 }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["age"], 2);

    // DELETE
    let (status, _) = send(app.clone(), "DELETE", &format!("/user/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);

    // 删除后 404 NOT_FOUND
    let (status, body) = send(app.clone(), "GET", &format!("/user/{id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "NOT_FOUND");
}

#[tokio::test]
async fn error_matrix() {
    let app = app().await;

    // 400 INVALID_BODY（非对象）
    let (status, body) = send(app.clone(), "POST", "/user", Some(json!([1, 2]))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "INVALID_BODY");

    // 400 INVALID_PARAM（p.* JSON 非法）
    let (status, body) = send(app.clone(), "GET", "/user?p.c0=%7B%22age%22%3A", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "INVALID_PARAM");
    assert!(body["error"]["message"].as_str().unwrap().contains("JSON"));

    // 403（contextProvider 抛权限错误）
    let req = Request::builder().method("GET").uri("/user");
    let req = req
        .header("x-user", "denied")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["error"]["code"], "ERR_PERMISSION");
    assert_eq!(body["error"]["message"], "ERR_PERMISSION:无写入权限");

    // 401 CONTEXT_ERROR（contextProvider 抛非权限错误，message 原样透传）
    let req = Request::builder()
        .method("GET")
        .uri("/user")
        .header("x-user", "broken")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["error"]["code"], "CONTEXT_ERROR");
    assert_eq!(body["error"]["message"], "token 解析失败");

    // 404 资源未注册（防御未知资源穿透）
    let (status, body) = send(app, "GET", "/nope", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "RESOURCE_NOT_FOUND");
}

#[test]
fn filter_archived_and_params_matrix() {
    assert_eq!(
        store_api_rs::filter_archived(vec!["user".into(), "userDeleted".into(), "post".into(), "logDeleted".into()]),
        vec!["user".to_string(), "post".to_string(), "logDeleted".to_string()]
    );
    let p = store_api_rs::params::parse_query_params(
        "p.a=true&p.b=false&p.c=null&p.d=42&p.e=-1.5&p.f=abc",
    )
    .unwrap();
    assert_eq!(p["a"], json!(true));
    assert_eq!(p["d"], json!(42));
    assert_eq!(p["e"], json!(-1.5));
    assert_eq!(p["f"], json!("abc"));
    assert_eq!(p["c"], json!(null));
    // 同名重复 → 数组
    let p = store_api_rs::params::parse_query_params("p.tag=a&p.tag=b").unwrap();
    assert_eq!(p["tag"], json!(["a", "b"]));
    // JSON 非法 → 错误原文
    assert!(store_api_rs::params::parse_query_params("p.o=%7B%22age%22%3A").is_err());
}
