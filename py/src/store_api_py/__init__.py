"""store-api-py — 为 py-store schema 自动生成 RESTful API 的 FastAPI 适配器。

    from store_api_py import create_app
    app = create_app(store, prefix='/api')  # uvicorn main:app
"""

from .app import create_app, filter_archived
from .params import ParamError, convert_value, parse_query_params

__all__ = ["create_app", "filter_archived", "parse_query_params", "convert_value", "ParamError"]
