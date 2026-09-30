"""查询参数 → GQL params 绑定。

规范依据：spec/02-params.md（双端逐字一致，改动必须先改 spec）。
"""

from __future__ import annotations

import json
import re
from typing import Any

_INT_RE = re.compile(r"^-?\d+$")
_FLOAT_RE = re.compile(r"^-?\d+\.\d+$")


class ParamError(Exception):
    code = "INVALID_PARAM"


def convert_value(raw: str) -> Any:
    """p.<name> 值的类型转换：true/false/null → 字面量；整数/浮点正则全匹配 → 数值；
    ``{``/``[`` 开头 → JSON 解析（失败抛 ParamError，禁静默当字符串）；其余 → 字符串原样。
    """
    if raw == "true":
        return True
    if raw == "false":
        return False
    if raw == "null":
        return None
    if _INT_RE.match(raw):
        return int(raw)
    if _FLOAT_RE.match(raw):
        return float(raw)
    if raw.startswith("{") or raw.startswith("["):
        try:
            return json.loads(raw)
        except json.JSONDecodeError as e:
            raise ParamError(f"参数 JSON 解析失败: {e}") from e
    return raw


def parse_query_params(query_params) -> dict[str, Any]:
    """从 QueryParams 收集 p.* 参数。同名重复出现 → 列表（每元素各自转换）；单次 → 标量。"""
    params: dict[str, Any] = {}
    for key in query_params.keys():
        if not key.startswith("p."):
            continue
        converted = [convert_value(v) for v in query_params.getlist(key)]
        params[key[2:]] = converted if len(converted) > 1 else converted[0]
    return params
