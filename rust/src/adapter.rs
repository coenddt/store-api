//! 开箱桥接（feature = "rust-store"）：rust-store 纯 Rust 宿主 → 本 crate 的 `Store` trait。
//!
//! 新类型包裹（orphan rule：trait 与宿主类型分属两仓，桥接实现必须落在本地新类型上）。
//! 上下文生命周期：trait 方法显式携带 `ctx: Option<Value>`（与宿主入参模型同构），
//! 适配器无状态转发——禁止全局持有「当前上下文」（并发请求会互相覆盖身份）。

use std::sync::Arc;

use async_trait::async_trait;
use rust_store_core::permission::{context_from_value, Context};
use serde_json::{Map, Value};

use crate::Store;

pub struct RustStoreAdapter {
    host: Arc<rust_store::Store>,
}

impl RustStoreAdapter {
    pub fn new(host: Arc<rust_store::Store>) -> Self {
        Self { host }
    }

    /// 底层宿主（注册 schema / DDL / 维护逃生口）
    pub fn host(&self) -> &Arc<rust_store::Store> {
        &self.host
    }

    fn to_ctx(ctx: Option<Value>) -> Option<Context> {
        // core 契约：null/非对象 → None（permission.rs context_from_value）
        ctx.as_ref().and_then(context_from_value)
    }
}

#[async_trait]
impl Store for RustStoreAdapter {
    async fn list(&self) -> Vec<String> {
        self.host.list_schemas()
    }

    async fn query(&self, gql: &str, params: &Map<String, Value>, ctx: Option<Value>) -> Result<Vec<Value>, String> {
        let ctx = Self::to_ctx(ctx);
        self.host.query(gql, params, ctx.as_ref()).await
    }

    async fn query_one(
        &self,
        gql: &str,
        params: &Map<String, Value>,
        ctx: Option<Value>,
    ) -> Result<Option<Value>, String> {
        let ctx = Self::to_ctx(ctx);
        self.host.query_one(gql, params, ctx.as_ref()).await
    }

    async fn insert(&self, schema: &str, data: &Value, ctx: Option<Value>) -> Result<Value, String> {
        let ctx = Self::to_ctx(ctx);
        self.host.insert(schema, data, ctx.as_ref()).await
    }

    async fn update(
        &self,
        schema: &str,
        condition: &Value,
        data: &Value,
        ctx: Option<Value>,
    ) -> Result<Option<Value>, String> {
        let ctx = Self::to_ctx(ctx);
        self.host.update(schema, condition, data, ctx.as_ref()).await
    }

    async fn remove(&self, schema: &str, condition: &Value, ctx: Option<Value>) -> Result<Value, String> {
        let ctx = Self::to_ctx(ctx);
        // Box::pin 具体化 future 类型：宿主 remove 的事务借用链（Txn→Conn 枚举）
        // 在 async-trait 的 Box<dyn Future + Send> 泛化检查下触发保守误报
        Box::pin(self.host.remove(schema, condition, ctx.as_ref())).await
    }

    async fn schema_fields(&self, schema: &str) -> Vec<String> {
        self.host.schema_fields(schema).unwrap_or_default()
    }

    fn is_permission_error(&self, err: &String) -> bool {
        // core 类型级契约：权限错误一律带 ERR_PERMISSION 前缀（core command/mod.rs ERR_PERM_PREFIX）
        err.starts_with("ERR_PERMISSION")
    }
}
