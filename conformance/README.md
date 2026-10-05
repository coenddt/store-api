# conformance — 双端一致性用例

同一份场景 JSON（`cases/*.json`），node / py 两端各自的 runner 加载执行，断言 HTTP 行为逐项一致。语义变更必须先改 `spec/`，再双端实现，再补这里的用例——三者不一致即为缺陷。

## 用例文件格式

```json
{
  "name": "users-crud",
  "resource": "user",
  "steps": [
    { "method": "POST", "path": "/api/user", "body": {"name": "a", "age": 1}, "expect": {"status": 201, "dataKeys": ["name", "age"]} }
  ]
}
```

`expect` 支持键：`status`（状态码）、`errorCode`（错误码）、`dataKeys`（data 中必须出现的键，用于规避 mock/真实 store 的字段差异）。

`opts` 为用例级适配器选项提示（runner 据此装配 mock 适配器）：

- `fileResolver: true` / `uploadResolver: true`：注入最小 mock resolver（下载回固定字节、上传回固定 `ref`）；
- `noFileResolver: true` / `noUploadResolver: true`：显式不注入，用于验证 501。

> 超限（413 `UPLOAD_TOO_LARGE`）不落在 JSON 用例：32MB 字节体塞进 JSON 不现实，该场景由双端 smoke 测试通过临时调小 `uploadLimit`（如 `uploadLimit: 16`）覆盖。

## 现状

- v0：用例文件已定格式并落第一份 `cases/users-crud.json`；两端 smoke 测试（`node/test/smoke.test.js`、`py/tests/test_smoke.py`）的用例矩阵与本目录语义对齐（mock store，不连真实库）。
- 待实现（v1）：双端 runner 直接消费 `cases/*.json` 执行并比对；CI 中双端互验。

## 用例清单

- `users-crud.json`：CRUD 全链路 + GQL 查询透传 + 错误矩阵（400/401/403/404/500）
- `users-file.json`：下载路由（注入 `fileResolver`：200；记录缺失 404）
- `users-file-upload.json`：上传路由（注入 `uploadResolver`：200 回写 / 空体 400 / 超限 413 / 记录缺失 404 / `field` 点路径）
- `users-file-unconfigured.json`：未注入 resolver（下载 501 `FILE_NOT_CONFIGURED` / 上传 501 `UPLOAD_NOT_CONFIGURED`）
