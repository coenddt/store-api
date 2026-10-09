"""store-api-py — FastAPI 适配器：为已注册 schema 自动生成 RESTful 路由。

路由/参数/错误/上下文语义全部以 spec/*.md 为唯一依据（双端 parity，改动先改 spec）。
"""

from __future__ import annotations

import functools
import inspect
import json
from typing import Any, Callable

from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse, Response

from .errors import (
    StoreApiError,
    context_error,
    empty_body,
    error_payload,
    file_not_configured,
    invalid_body,
    map_error,
    not_found,
    too_large,
    upload_not_configured,
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
    file_field: str = "file",
    file_resolver: Callable[[Request, dict, str], Any] | None = None,
    upload_resolver: Callable[[Request, dict, str], Any] | None = None,
    upload_limit: int = 32 * 1024 * 1024,
) -> FastAPI:
    """为 store（py-store 的 store 实例，需已 init + register）生成 RESTful FastAPI 应用。

    - prefix: 路由前缀，如 '/api'
    - id_field: 单条路由主键字段名（双端一致，spec/01-routing.md，默认 '_id'）
    - context_provider: 每请求上下文钩子（spec/04-context.md）；非权限类抛错 ⇒ 401 CONTEXT_ERROR
    - resources: 显式资源名；缺省取 store.list() 并过滤归档表
    - permission_error: store 权限错误类；缺省取 store.PermissionError
    - file_resolver: 下载字节接缝 (request, rec, rid) => {body, contentType, fileName}；缺省时下载 501
    - upload_resolver: 上传字节接缝 (request, rec, rid) => {ref}；缺省时上传 501
    - upload_limit: 上传字节体上限（超限 413），默认 32MB
    """
    app = FastAPI(title="store-api")
    if permission_error is None:
        permission_error = getattr(store, "PermissionError", None)

    # fail-secure 装配守卫：宿主已开启上下文强制却未配 context_provider 时，装配期即
    # 拒绝（fail-fast）。否则服务能启动、每个请求却在运行期以 ERR_NO_CONTEXT 失败——
    # 配置错误被推迟成运行时事故（no-error-masking：不允许静默带错运行）。
    require_context = getattr(store, "require_context", None)
    if callable(require_context) and require_context() and context_provider is None:
        raise RuntimeError(
            "ERR_SECURE_CONFIG: 宿主已开启上下文强制（fail-secure）但未配置 context_provider；"
            "请注入从请求解析身份的 context_provider（spec/04-context.md），"
            "无需鉴权的内部服务请显式 store.set_require_context(False) 后再装配"
        )

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
            # spec/04：返回 None 时同样显式注入空上下文（store.set_context(None) 语义为清除，
            # 有状态持有的运行时禁止残留上一请求上下文，防身份跨请求泄漏）
            store.set_context(ctx)
            return await call_next(request)

    # B6：`x-cache` 响应头注记位——取值唯一来源为宿主 store.cache_status（未实现时恒 BYPASS）；
    # 覆盖所有响应（含错误响应）。非法值回落 BYPASS（宿主侧已对非法/异常走反馈通道留痕）。
    _CACHE_VALUES = ("HIT", "MISS", "BYPASS")

    @app.middleware("http")
    async def _annotate_cache(request: Request, call_next):
        response = await call_next(request)
        fn = getattr(store, "cache_status", None)
        v = fn() if callable(fn) else "BYPASS"
        response.headers["x-cache"] = v if v in _CACHE_VALUES else "BYPASS"
        return response

    names = resources if resources is not None else filter_archived(store.list())
    for name in names:
        # 显式投影（spec/01+02）：GQL 省略投影段 = 只返回 _id（三端 core 契约）。
        # 列表路由仅在 q 缺失时用它；q 存在时投影完全由 q 决定，适配层不追加。
        proj = _schema_projection(store, name)
        _register_resource(
            app, store, name, prefix, id_field, permission_error, proj,
            file_field, file_resolver, upload_resolver, upload_limit,
        )

    return app


def _schema_projection(store: Any, name: str) -> str:
    """从 store 元数据生成显式投影串（' { f1, f2 }'）。
    取值顺序：store.get(name)（nodejs 形态）→ py_store.schema 模块（py-store 未在 Store 类暴露 get）。
    两者皆不可得 / fields 为空 → 空串（无投影，data 仅 _id——上游 schema 定义不完整的显式后果）。"""
    fields: dict | None = None
    get = getattr(store, "get", None)
    if callable(get):
        try:
            meta = get(name)
        except KeyError:
            meta = None
        if isinstance(meta, dict):
            fields = meta.get("fields")
        else:
            fields = getattr(meta, "fields", None)
    if fields is None:
        try:
            from py_store import schema as py_schema

            fields = (py_schema.get(name) or {}).get("fields")
        except Exception:
            fields = None
    keys = list((fields or {}).keys())
    return f" {{ {', '.join(keys)} }}" if keys else ""


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


def _pick_field(rec: dict | None, field_path: str) -> Any:
    """点路径取字段（决策 C）：'images.full' → rec['images']['full']；任一段缺失 → None。"""
    if rec is None:
        return None
    cur: Any = rec
    for seg in field_path.split("."):
        if not isinstance(cur, dict):
            return None
        cur = cur.get(seg)
        if cur is None:
            return None
    return cur


async def _read_upload_body(request: Request, upload_limit: int) -> bytes:
    """上传字节体（决策 B'）：空体 → 400 EMPTY_BODY；超限 → 413 UPLOAD_TOO_LARGE。

    FastAPI/starlette 无 Fastify 式 parser/bodyLimit，故 upload_limit 是 py 端唯一防线。
    """
    body = await request.body()
    if not body:
        raise empty_body(f"上传请求体为空: {request.method} {request.url.path}")
    if len(body) > upload_limit:
        raise too_large(f"上传字节体 {len(body)} 字节超过上限 {upload_limit}")
    return body


def _set_by_path(field_path: str, value: Any) -> dict:
    """上传回写的 set 体：字面键，交由 store 寻址。

    store 对非 `$` 前缀键自动包 `$set`（py-store/src/py_store/crud/write.py），
    点路径键 `$set {'a.b': v}` 即嵌套写入 —— 与 node 端同策略（parity）。
    """
    return {field_path: value}


def _register_resource(
    app: FastAPI,
    store: Any,
    name: str,
    prefix: str,
    id_field: str,
    permission_error: type[BaseException] | None,
    proj: str,
    file_field: str,
    file_resolver: Callable[[Request, dict, str], Any] | None,
    upload_resolver: Callable[[Request, dict, str], Any] | None,
    upload_limit: int,
) -> None:
    # 工厂函数隔离闭包：handler 签名只含 Request 与路径参数，
    # 否则 FastAPI 会把捕获变量解析成查询参数（缺参即 422）
    base = f"{prefix}/{name}"

    def _field_path(request: Request) -> str:
        # 请求级字段路径：?field= 优先，缺省回落插件级 file_field（spec/02-params.md）
        f = request.query_params.get("field")
        return f if f else file_field

    @app.get(base)
    @_guard(permission_error)
    async def list_resource(request: Request):
        params = parse_query_params(request.query_params)
        # q 缺失 → 全字段投影（spec/02）；q 存在 → 投影完全由 q 决定
        gql = name + (request.query_params.get("q") or proj)
        return {"data": await store.query(gql, params)}

    @app.get(base + "/{rid}")
    @_guard(permission_error)
    async def get_one(rid: str):
        data = await store.query_one(f"{name}($condition: @c0){proj}", {"c0": {id_field: rid}})
        if data is None:
            raise not_found(f"记录不存在: {id_field}={rid}")
        return {"data": data}

    @app.get(base + "/{rid}/file")
    @_guard(permission_error)
    async def get_file(rid: str, request: Request):
        # spec/03 判定顺序第 1 层：未注入 file_resolver → 501（先于 query_one 判定）
        if file_resolver is None:
            raise file_not_configured(f"GET {base}/{{rid}}/file 未注入 file_resolver")
        fp = _field_path(request)
        rec = await store.query_one(f"{name}($condition: @c0){proj}", {"c0": {id_field: rid}})
        if rec is None:
            raise not_found(f"记录不存在: {id_field}={rid}")
        raw = _pick_field(rec, fp)
        if raw is None:
            raise not_found(f"文件不存在: {fp}={rid}")
        out = file_resolver(request, rec, rid)
        if inspect.isawaitable(out):
            out = await out
        return Response(
            content=out["body"],
            media_type=out.get("contentType", "application/octet-stream"),
            headers={"content-disposition": f'attachment; filename="{out.get("fileName", "file")}"'},
        )

    @app.post(base + "/{rid}/file")
    @_guard(permission_error)
    async def upload_file(rid: str, request: Request):
        # spec/03 判定顺序第 1 层：未注入 upload_resolver → 501
        if upload_resolver is None:
            raise upload_not_configured(f"POST {base}/{{rid}}/file 未注入 upload_resolver")
        await _read_upload_body(request, upload_limit)  # 空体 400 / 超限 413
        fp = _field_path(request)
        rec = await store.query_one(f"{name}($condition: @c0){proj}", {"c0": {id_field: rid}})
        if rec is None:
            raise not_found(f"记录不存在: {id_field}={rid}")
        # 接缝：resolver 负责字节落库并返回引用（决策 B：与 file_resolver 对称）
        out = upload_resolver(request, rec, rid)
        if inspect.isawaitable(out):
            out = await out
        ref = out.get("ref") if isinstance(out, dict) else None
        if ref is None:
            raise RuntimeError(  # → map_error 末层 500 原样透传（禁静默）
                f"upload_resolver 未返回 ref: {name}({id_field}={rid})"
            )
        # 回写：走 update 语义 → RBAC 写权限自动生效（决策 A）
        data = await store.update(name, {id_field: rid}, _set_by_path(fp, ref))
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
