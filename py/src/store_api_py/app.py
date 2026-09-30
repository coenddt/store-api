"""store-api-py — FastAPI 适配器：为已注册 schema 自动生成 RESTful 路由。

路由/参数/错误/上下文语义全部以 spec/*.md 为唯一依据（双端 parity，改动先改 spec）。
"""

from __future__ import annotations

import functools
import inspect
import json
from typing import Any, Callable

from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse

from .errors import (
    StoreApiError,
    context_error,
    error_payload,
    invalid_body,
    map_error,
    not_found,
)
from .params import parse_query_params

ARCHIVE_SUFFIX = "Deleted"

ContextProvider = Callable[[Request], Any]


def filter_archived(names: list[str]) -> list[str]:
    """归档表过滤（spec/01-routing.md）：`XxxDeleted` 且 `Xxx` 也在列表中 ⇒ 视为归档表。"""
    s = set(names)
    return [n for n in names if not (n.endswith(ARCHIVE_SUFFIX) and n[: -len(ARCHIVE_SUFFIX)] in s)]


async def _resolve_ctx(provider: ContextProvider, request: Request) -> Any:
    ctx = provider(request)
    if inspect.isawaitable(ctx):
        ctx = await ctx
    return ctx


def create_app(
    store: Any,
    *,
    prefix: str = "",
    id_field: str = "_id",
    context_provider: ContextProvider | None = None,
    resources: list[str] | None = None,
    permission_error: type[BaseException] | None = None,
) -> FastAPI:
    """为 store（py-store 的 store 实例，需已 init + register）生成 RESTful FastAPI 应用。

    - prefix: 路由前缀，如 '/api'
    - id_field: 单条路由主键字段名（双端一致，spec/01-routing.md，默认 '_id'）
    - context_provider: 每请求上下文钩子（spec/04-context.md）；非权限类抛错 ⇒ 401 CONTEXT_ERROR
    - resources: 显式资源名；缺省取 store.list() 并过滤归档表
    - permission_error: store 权限错误类；缺省取 store.PermissionError
    """
    app = FastAPI(title="store-api")
    if permission_error is None:
        permission_error = getattr(store, "PermissionError", None)

    def _respond(mapped: StoreApiError) -> JSONResponse:
        return JSONResponse(status_code=mapped.status_code, content=error_payload(mapped.code, mapped.message))

    if context_provider is not None:

        @app.middleware("http")
        async def _inject_context(request: Request, call_next):
            try:
                ctx = await _resolve_ctx(context_provider, request)
            except Exception as e:  # noqa: BLE001 — spec/04-context.md：PermissionError ⇒ 403（RBAC 拒绝）；其余 ⇒ 401，message 原样透传
                if permission_error is not None and isinstance(e, permission_error):
                    return _respond(map_error(e, permission_error))
                return _respond(context_error(str(e) or None))
            if ctx is not None:
                store.set_context(ctx)
            return await call_next(request)

    names = resources if resources is not None else filter_archived(store.list())
    for name in names:
        _register_resource(app, store, name, prefix, id_field, permission_error)

    return app


def _guard(permission_error: type[BaseException] | None) -> Callable:
    """单一错误出口：handler 抛出的所有错误统一走 map_error 判定（禁 catch 后洗成成功）。"""

    def deco(fn: Callable) -> Callable:
        @functools.wraps(fn)  # 保留原签名，FastAPI 靠 inspect.signature 注入 Request/路径参数
        async def wrapper(*args, **kwargs):
            try:
                return await fn(*args, **kwargs)
            except Exception as e:  # noqa: BLE001 — 错误必须映射为带语义的 HTTP 响应，禁止静默
                mapped = map_error(e, permission_error)
                return JSONResponse(status_code=mapped.status_code, content=error_payload(mapped.code, mapped.message))

        return wrapper

    return deco


def _register_resource(
    app: FastAPI,
    store: Any,
    name: str,
    prefix: str,
    id_field: str,
    permission_error: type[BaseException] | None,
) -> None:
    # 工厂函数隔离闭包：handler 签名只含 Request 与路径参数，
    # 否则 FastAPI 会把捕获变量解析成查询参数（缺参即 422）
    base = f"{prefix}/{name}"

    @app.get(base)
    @_guard(permission_error)
    async def list_resource(request: Request):
        params = parse_query_params(request.query_params)
        gql = name + (request.query_params.get("q") or "")
        return {"data": await store.query(gql, params)}

    @app.get(base + "/{rid}")
    @_guard(permission_error)
    async def get_one(rid: str):
        data = await store.query_one(f"{name}($condition: @c0)", {"c0": {id_field: rid}})
        if data is None:
            raise not_found(f"记录不存在: {id_field}={rid}")
        return {"data": data}

    @app.post(base, status_code=201)
    @_guard(permission_error)
    async def create_resource(request: Request):
        return {"data": await store.insert(name, await _read_json_body(request))}

    @app.patch(base + "/{rid}")
    @_guard(permission_error)
    async def update_one(rid: str, request: Request):
        return {"data": await store.update(name, {id_field: rid}, await _read_json_body(request))}

    @app.delete(base + "/{rid}")
    @_guard(permission_error)
    async def delete_one(rid: str):
        return {"data": await store.remove(name, {id_field: rid})}


async def _read_json_body(request: Request) -> dict:
    try:
        body = await request.json()
    except json.JSONDecodeError as e:
        raise invalid_body(f"请求体不是合法 JSON: {e}") from e
    if not isinstance(body, dict):
        raise invalid_body("请求体必须是 JSON 对象")
    return body
