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


def context_error(message: str | None) -> StoreApiError:
    return StoreApiError(401, "CONTEXT_ERROR", message)


def map_error(err: BaseException, permission_error: type[BaseException] | None) -> StoreApiError:
    """判定顺序（spec/03-errors.md）：
    适配层守卫(400) → PermissionError(403) → queryOne 空结果(404) → 其余 500 透传
    """
    if isinstance(err, StoreApiError):
        return err
    if isinstance(err, ParamError):
        return StoreApiError(400, "INVALID_PARAM", str(err) or None)
    if permission_error is not None and isinstance(err, permission_error):
        # 按类型判定，禁按 message 匹配
        return StoreApiError(403, store_code(err), str(err) or None)
    return StoreApiError(500, store_code(err), str(err) or None)
