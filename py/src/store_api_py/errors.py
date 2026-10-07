"""错误 → HTTP 状态码映射。

规范依据：spec/03-errors.md（判定顺序双端一致，改动必须先改 spec）。
遵循 no-error-masking：message 原样透传、取不到置 None（禁伪造），成功响应不得带 error 字段。
"""

from __future__ import annotations

from typing import Any

from .params import ParamError


class StoreApiError(Exception):
    """适配层自身守卫错误（400/401/404），message 为确定性描述。"""

    def __init__(self, status_code: int, code: str, message: str | None):
        super().__init__(message)
        self.status_code = status_code
        self.code = code
        self.message = message


def error_payload(code: str, message: str | None) -> dict[str, Any]:
    return {"error": {"code": code, "message": message}}


def store_code(err: BaseException) -> str:
    """store 错误的分类码：优先 err.code，缺省用错误类名，再缺省用通用码。"""
    return getattr(err, "code", None) or type(err).__name__ or "STORE_ERROR"


def invalid_body(message: str) -> StoreApiError:
    return StoreApiError(400, "INVALID_BODY", message)


def not_found(message: str) -> StoreApiError:
    return StoreApiError(404, "NOT_FOUND", message)


# ── 资源字节上传/下载守卫工厂（spec/03-errors.md） ──
def empty_body(message: str | None = None) -> StoreApiError:
    return StoreApiError(400, "EMPTY_BODY", message or "上传请求体为空")


def too_large(message: str | None = None) -> StoreApiError:
    return StoreApiError(413, "UPLOAD_TOO_LARGE", message or "上传字节体超过上限")


def upload_not_configured(message: str | None = None) -> StoreApiError:
    return StoreApiError(501, "UPLOAD_NOT_CONFIGURED", message or "未注入 upload_resolver")


def file_not_configured(message: str | None = None) -> StoreApiError:
    return StoreApiError(501, "FILE_NOT_CONFIGURED", message or "未注入 file_resolver")


def context_error(message: str | None) -> StoreApiError:
    return StoreApiError(401, "CONTEXT_ERROR", message)


def map_error(err: BaseException, permission_error: type[BaseException] | None) -> StoreApiError:
    """判定顺序（spec/03-errors.md）：
    未注入 resolver / 适配层守卫(400/413/501，由 StoreApiError 自带 status) → PermissionError(403)
    → GQL 解析失败(400) → 其余 500 透传。
    """
    if isinstance(err, StoreApiError):
        return err
    if isinstance(err, ParamError):
        return StoreApiError(400, "INVALID_PARAM", str(err) or None)
    if permission_error is not None and isinstance(err, permission_error):
        # 按类型判定，禁按 message 匹配
        return StoreApiError(403, store_code(err), str(err) or None)
    # spec/03 判定顺序第 3 层：GQL 解析失败（core 稳定前缀 ERR_GQL_PARSE:，与
    # ERR_PERM_PREFIX 同构的类型级契约——前缀判定非文案脆弱匹配）→ 400 GQL_PARSE，
    # message 剥前缀取原文（与 go/rust/node 同语义）。
    text = str(err)
    if text.startswith("ERR_GQL_PARSE:"):
        return StoreApiError(400, "GQL_PARSE", text[len("ERR_GQL_PARSE:"):])
    # spec/03 判定顺序第 4 层（core 稳定前缀类）：资源无任何可读副本
    # （py-store resource open 零副本行 → ERR_RESOURCE_NOT_FOUND:）→ 404 NOT_FOUND，
    # message 剥前缀取原文（与 GQL_PARSE 同构；禁按文案匹配）。
    if text.startswith("ERR_RESOURCE_NOT_FOUND:"):
        return StoreApiError(404, "NOT_FOUND", text[len("ERR_RESOURCE_NOT_FOUND:"):] or None)
    return StoreApiError(500, store_code(err), text or None)
