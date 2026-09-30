//! 开箱桥接（feature = "rust-store"）：rust-store 纯 Rust 宿主 → 本 crate 的 `Store` trait。
//!
//! 新类型包裹（orphan rule：trait 与宿主类型分属两仓，桥接实现必须落在本地新类型上）。
//! 上下文生命周期：store-api 的 trait 以 `set_context(Value)` 注入，宿主是显式入参模型，
//! 适配器用 `RwLock<Option<Context>>` 持有当前请求上下文，每次调用取出传给宿主。

use std::sync::Arc;

use async_trait::async_trait;
use rust_store_core::permission::{context_from_value, Context};
use serde_json::{Map, Value};
use tokio::sync::RwLock;

use crate::Store;

pub struct RustStoreAdapter {
    host: Arc<rust_store::Store>,
    ctx: RwLock<Option<Context>>,
}

impl RustStoreAdapter {
    pub fn new(host: Arc<rust_store::Store>) -> Self {
        Self {
            host,
            ctx: RwLock::new(None),
        }
    }

    /// 底层宿主（注册 schema / DDL / 维护逃生口）
    pub fn host(&self) -> &Arc<rust_store::Store> {
        &self.host
    }
}

#[async_trait]
impl Store for RustStoreAdapter {
    async fn list(&self) -> Vec<String> {
        self.host.list_schemas()
    }

    async fn query(&self, gql: &str, params: &Map<String, Value>) -> Result<Vec<Value>, String> {
        let ctx = self.ctx.read().await.clone();
        self.host.query(gql, params, ctx.as_ref()).await
    }

    async fn query_one(
        &self,
        gql: &str,
        params: &Map<String, Value>,
    ) -> Result<Option<Value>, String> {
        let ctx = self.ctx.read().await.clone();
        self.host.query_one(gql, params, ctx.as_ref()).await
    }

    async fn insert(&self, schema: &str, data: &Value) -> Result<Value, String> {
        let ctx = self.ctx.read().await.clone();
        self.host.insert(schema, data, ctx.as_ref()).await
    }

    async fn update(
        &self,
        schema: &str,
        condition: &Value,
        data: &Value,
    ) -> Result<Option<Value>, String> {
        let ctx = self.ctx.read().await.clone();
        self.host.update(schema, condition, data, ctx.as_ref()).await
    }

    async fn remove(&self, schema: &str, condition: &Value) -> Result<Value, String> {
        let ctx = self.ctx.read().await.clone();
        self.host.remove(schema, condition, ctx.as_ref()).await
    }

    async fn set_context(&self, ctx: Value) {
        *self.ctx.write().await = context_from_value(&ctx);
    }

    fn is_permission_error(&self, err: &String) -> bool {
        // core 类型级契约：权限错误一律带 ERR_PERMISSION 前缀（core command/mod.rs ERR_PERM_PREFIX）
        err.starts_with("ERR_PERMISSION")
    }
}
