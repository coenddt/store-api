"""映射矩阵：每档错误语义 → REST 状态码（A3「可程序化区分」验收）。

直接调纯函数 map_error（无需真实 store/库），用例与 node/test/error-matrix.test.js 同构。
规范依据：spec/03-errors.md 判定顺序 + core machine code（`no_context` / `permission_denied`）。
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from store_api_py.errors import map_error  # noqa: E402


class PermissionError(Exception):
    """store 权限错误类（按类型判定，禁按 message 匹配）。"""


def _err(message: str, code: str) -> Exception:
    e = Exception(message)
    e.code = code
    return e


def test_matrix_permission_to_403():
    err = PermissionError("无访问权限")
    err.code = "permission_denied"
    r = map_error(err, PermissionError)
    assert r.status_code == 403
    assert r.code == "permission_denied"
    assert r.message == "无访问权限"


def test_matrix_no_context_to_403():
    r = map_error(_err("上下文缺失", "no_context"), PermissionError)
    assert r.status_code == 403
    assert r.code == "no_context"
    assert r.message == "上下文缺失"


def test_matrix_gql_parse_to_400():
    r = map_error(Exception("ERR_GQL_PARSE:意外的 token"), PermissionError)
    assert r.status_code == 400
    assert r.code == "GQL_PARSE"
    assert r.message == "意外的 token"


def test_matrix_other_to_500():
    r = map_error(_err("连接超时", "CONN_TIMEOUT"), PermissionError)
    assert r.status_code == 500
    assert r.code == "CONN_TIMEOUT"
    assert r.message == "连接超时"