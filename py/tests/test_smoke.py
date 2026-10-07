"""smoke 测试：mock store（不连真实库），镜像 node/test/smoke.test.js 的用例矩阵。

conformance 纪律：两端断言逐项对应，任何一端改动必须同步另一端。
"""

import sys
from pathlib import Path

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from store_api_py import ParamError, create_app, filter_archived, parse_query_params  # noqa: E402
from store_api_py.params import convert_value  # noqa: E402


class MockPermissionError(Exception):
    pass


def _apply_set(row: dict, path: str, value):
    """模拟 store $set 的点路径写入语义：'images.full' → row['images']['full']（Mongo $set {'a.b': v} 即嵌套写）。"""
    if path.startswith("$"):
        return  # 原生操作符（$inc/$unset 等）本 mock 不模拟
    segs = path.split(".")
    cur = row
    for s in segs[:-1]:
        if not isinstance(cur.get(s), dict):
            cur[s] = {}
        cur = cur[s]
    cur[segs[-1]] = value


class MockStore:
    """mock store：只实现适配器调用面（query/query_one/insert/update/remove/list/set_context）。"""

    PermissionError = MockPermissionError

    def __init__(self):
        self.rows: dict[str, dict] = {}
        self.last_query: tuple | None = None
        self.last_context = None

    def list(self):
        return ["user", "userDeleted"]

    def get(self, name):
        if name == "user":
            return {"name": "user", "fields": {"name": {"type": "string"}, "age": {"type": "number"}}}
        raise KeyError(name)

    def set_context(self, ctx):
        self.last_context = ctx

    async def query(self, gql, params=None):
        self.last_query = (gql, params)
        return list(self.rows.values())

    async def query_one(self, gql, params=None):
        self.last_query = (gql, params)
        rid = (params or {}).get("c0", {}).get("_id")
        return self.rows.get(rid)

    async def insert(self, name, data):
        rid = data.get("_id") or f"u{len(self.rows) + 1}"
        row = {"_id": rid, **data}
        self.rows[rid] = row
        return row

    async def update(self, name, cond, data):
        row = self.rows.get(cond.get("_id"))
        if row:
            # 模拟 store 的 $set 语义：非 `$` 前缀键自动包 $set（py-store/src/py_store/crud/write.py），
            # 键内点路径由后端寻址（Mongo $set {'a.b': v} 即嵌套写入）。
            for k, v in data.items():
                _apply_set(row, k, v)
        return row

    async def remove(self, name, cond):
        return self.rows.pop(cond.get("_id"), None)


def build_app(store, **extra):
    return create_app(store, prefix="/api", **extra)


def test_filter_archived():
    assert filter_archived(["user", "userDeleted", "post", "logDeleted"]) == ["user", "post", "logDeleted"]


def test_params_convert_matrix():
    qp = parse_query_params(
        _QP({"p.a": "true", "p.b": "false", "p.c": "null", "p.d": "42", "p.e": "-1.5", "p.f": "abc"})
    )
    assert qp == {"a": True, "b": False, "c": None, "d": 42, "e": -1.5, "f": "abc"}
    assert parse_query_params(_QP({"p.o": '{"age":{"$gte":18}}'})) == {"o": {"age": {"$gte": 18}}}
    # 同名重复出现 → 列表（每元素各自转换）
    assert parse_query_params(_QP([("p.tag", "a"), ("p.tag", "b")])) == {"tag": ["a", "b"]}


def test_params_convert_value_scalars():
    assert convert_value("true") is True
    assert convert_value("null") is None
    assert convert_value("42") == 42
    assert convert_value("abc") == "abc"


def test_params_invalid_json():
    with pytest.raises(ParamError):
        parse_query_params(_QP({"p.o": '{"age":'}))


class _QP:
    """starlette QueryParams 的最小替身"""

    def __init__(self, items):
        self._items = [(k, v) for k, v in (items.items() if isinstance(items, dict) else items)]

    def keys(self):
        return list(dict.fromkeys(k for k, _ in self._items))

    def getlist(self, key):
        return [v for k, v in self._items if k == key]

    def get(self, key, default=None):
        vs = self.getlist(key)
        return vs[0] if vs else default


def test_crud_full_chain():
    store = MockStore()
    client = TestClient(build_app(store))

    created = client.post("/api/user", json={"name": "a", "age": 1})
    assert created.status_code == 201
    assert created.json()["data"]["name"] == "a"
    rid = created.json()["data"]["_id"]

    listed = client.get("/api/user", params={"q": "($condition: @c0)", "p.c0": '{"age":{"$gte":18}}'})
    assert listed.status_code == 200
    assert isinstance(listed.json()["data"], list)
    # 拼接结果 = 资源名 + q 原样（spec/02-params.md）
    assert store.last_query == ("user($condition: @c0)", {"c0": {"age": {"$gte": 18}}})

    # 无 q → 适配器生成全字段投影（spec/02）
    listed_no_q = client.get("/api/user")
    assert listed_no_q.status_code == 200
    assert store.last_query == ("user { name, age }", {})

    got = client.get(f"/api/user/{rid}")
    assert got.status_code == 200
    assert store.last_query == ("user($condition: @c0) { name, age }", {"c0": {"_id": rid}})

    patched = client.patch(f"/api/user/{rid}", json={"age": 2})
    assert patched.status_code == 200
    assert patched.json()["data"]["age"] == 2

    removed = client.delete(f"/api/user/{rid}")
    assert removed.status_code == 200

    gone = client.get(f"/api/user/{rid}")
    assert gone.status_code == 404
    assert gone.json()["error"]["code"] == "NOT_FOUND"


def test_error_mapping_400_403_401():
    store = MockStore()

    def provider(request):
        who = request.headers.get("x-user")
        if who == "denied":
            raise MockPermissionError("权限不足")
        if who == "broken":
            raise ValueError("token 解析失败")
        return {"uid": who}

    app = build_app(store, context_provider=provider)
    client = TestClient(app)

    bad_body = client.post("/api/user", json=[1, 2])
    assert bad_body.status_code == 400
    assert bad_body.json()["error"]["code"] == "INVALID_BODY"

    bad_param = client.get("/api/user", params={"p.c0": '{"age":'})
    assert bad_param.status_code == 400
    assert bad_param.json()["error"]["code"] == "INVALID_PARAM"
    assert "JSON" in bad_param.json()["error"]["message"]

    denied = client.get("/api/user", headers={"x-user": "denied"})
    assert denied.status_code == 403
    assert denied.json()["error"]["code"] == "MockPermissionError"
    assert denied.json()["error"]["message"] == "权限不足"

    broken = client.get("/api/user", headers={"x-user": "broken"})
    assert broken.status_code == 401
    assert broken.json()["error"]["code"] == "CONTEXT_ERROR"
    assert broken.json()["error"]["message"] == "token 解析失败"

    ok = client.get("/api/user", headers={"x-user": "u1"})
    assert ok.status_code == 200
    assert "error" not in ok.json()  # 成功响应不得带 error 字段
    assert store.last_context == {"uid": "u1"}


def test_store_error_passthrough_500():
    store = MockStore()

    async def boom(gql, params=None):
        e = RuntimeError("连接超时")
        e.code = "CONN_TIMEOUT"
        raise e

    store.query = boom
    client = TestClient(build_app(store))
    res = client.get("/api/user")
    assert res.status_code == 500
    assert res.json()["error"]["code"] == "CONN_TIMEOUT"
    assert res.json()["error"]["message"] == "连接超时"


def test_fastapi_instance():
    assert isinstance(build_app(MockStore()), FastAPI)


def test_x_cache_header():
    """x-cache 注记位（B6）：无 provider 恒 BYPASS（含错误响应）；provider=HIT 透传"""
    store = MockStore()
    client = TestClient(build_app(store))

    ok = client.get("/api/user")
    assert ok.headers["x-cache"] == "BYPASS"

    missing = client.get("/api/user/nope")
    assert missing.status_code == 404
    assert missing.headers["x-cache"] == "BYPASS"  # 错误响应同样带注记

    store.cache_status = lambda: "HIT"
    hit = client.get("/api/user")
    assert hit.headers["x-cache"] == "HIT"


def test_file_download_route():
    """文件下载路由：注入 resolver → 200 + 头；未注入 → 501；缺失 → 404（spec/01+03+05）"""
    store = MockStore()

    # 未注入 file_resolver → 501 FILE_NOT_CONFIGURED（决策 D，原 text/plain 兜底已废止）
    bare = TestClient(build_app(store))
    rid = bare.post("/api/user", json={"name": "a", "age": 1}).json()["data"]["_id"]
    store.rows[rid]["file"] = "hello"
    not_configured = bare.get(f"/api/user/{rid}/file")
    assert not_configured.status_code == 501
    assert not_configured.json()["error"]["code"] == "FILE_NOT_CONFIGURED"

    # 注入 resolver → 200
    async def _fr(request, rec, i):
        return {"body": b"hello", "contentType": "text/plain; charset=utf-8", "fileName": f"user-{i}"}

    client = TestClient(build_app(store, file_resolver=_fr))
    file = client.get(f"/api/user/{rid}/file")
    assert file.status_code == 200
    assert file.headers["content-type"] == "text/plain; charset=utf-8"
    assert file.headers["content-disposition"] == f'attachment; filename="user-{rid}"'
    assert file.content == b"hello"

    missing = client.get("/api/user/nope/file")
    assert missing.status_code == 404
    assert missing.json()["error"]["code"] == "NOT_FOUND"


def test_file_download_resource_not_found_maps_404():
    """下载路由：资源无任何可读副本（core 前缀 ERR_RESOURCE_NOT_FOUND:）→ 404 NOT_FOUND（spec/03）。"""
    store = MockStore()
    rid = TestClient(build_app(store)).post("/api/user", json={"name": "a", "age": 1}).json()["data"]["_id"]
    store.rows[rid]["file"] = "sha1x"  # 字段非空才会走到 resolver

    async def _fr(request, rec, i):
        raise FileNotFoundError("ERR_RESOURCE_NOT_FOUND:资源不存在或无可读副本: sha1x")

    res = TestClient(build_app(store, file_resolver=_fr)).get(f"/api/user/{rid}/file")
    assert res.status_code == 404
    assert res.json()["error"]["code"] == "NOT_FOUND"
    assert res.json()["error"]["message"] == "资源不存在或无可读副本: sha1x"  # 前缀已剥离


def test_file_download_provider_failure_stays_500():
    """下载路由：provider 读取失败（非前缀错误）→ 500 透传，不得伪装成 404。"""
    store = MockStore()
    rid = TestClient(build_app(store)).post("/api/user", json={"name": "a", "age": 1}).json()["data"]["_id"]
    store.rows[rid]["file"] = "sha1x"

    async def _fr(request, rec, i):
        raise RuntimeError("io down")

    res = TestClient(build_app(store, file_resolver=_fr)).get(f"/api/user/{rid}/file")
    assert res.status_code == 500
    assert res.json()["error"]["message"] == "io down"


def test_upload_route_and_field():
    """上传路由：未注入 → 501；注入 → 200 回写；空体 → 400；超限 → 413；field 点路径（spec/01+02+03）"""
    store = MockStore()
    bare = TestClient(build_app(store))
    rid = bare.post("/api/user", json={"name": "a", "age": 1}).json()["data"]["_id"]

    nc = bare.post(f"/api/user/{rid}/file", content=b"bytes",
                   headers={"content-type": "application/octet-stream"})
    assert nc.status_code == 501
    assert nc.json()["error"]["code"] == "UPLOAD_NOT_CONFIGURED"

    async def _ur(request, rec, i):
        return {"ref": "sha1abc"}

    client = TestClient(build_app(store, upload_resolver=_ur))
    ok = client.post(f"/api/user/{rid}/file", content=b"bytes",
                     headers={"content-type": "application/octet-stream"})
    assert ok.status_code == 200
    assert ok.json()["data"]["file"] == "sha1abc"

    empty = client.post(f"/api/user/{rid}/file", content=b"",
                        headers={"content-type": "application/octet-stream"})
    assert empty.status_code == 400
    assert empty.json()["error"]["code"] == "EMPTY_BODY"

    # 超限 → 413 UPLOAD_TOO_LARGE（A4 parity；用小 upload_limit 触发，避免构造 32MB 大包）
    small = TestClient(build_app(store, upload_resolver=_ur, upload_limit=4))
    too_large = small.post(f"/api/user/{rid}/file", content=b"bytes",
                           headers={"content-type": "application/octet-stream"})
    assert too_large.status_code == 413
    assert too_large.json()["error"]["code"] == "UPLOAD_TOO_LARGE"

    store.rows[rid]["images"] = {"full": "old-ref"}
    ok2 = client.post(f"/api/user/{rid}/file?field=images.full", content=b"bytes",
                      headers={"content-type": "application/octet-stream"})
    assert ok2.status_code == 200
    assert ok2.json()["data"]["images"]["full"] == "sha1abc"

    missing = client.post("/api/user/nope/file", content=b"bytes",
                          headers={"content-type": "application/octet-stream"})
    assert missing.status_code == 404
    assert missing.json()["error"]["code"] == "NOT_FOUND"
