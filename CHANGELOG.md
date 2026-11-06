# Changelog — store-api（v0.6.x 维护线）

本文件只记录 v0.6.x 维护分支的补丁级变更；主干（含破坏性契约变更）见 `main` 的 CHANGELOG.md。

## [Unreleased] — 0.6.4

### Fixed

- **下载 `Content-Disposition` 清洗 `fileName` 的 CR/LF**：py 实测可把 `\r\n` 原样写入响应头
  （可拆出额外响应头），node 实测会 500 `ERR_INVALID_CHAR`；两实现统一把 CR/LF 替换为空格，
  文件名其余内容保持不变。正常文件名（不含 CR/LF）的响应头逐字节不变，对现成调用方无影响。
- **py 上传体改为边读边限流**：`_read_upload_body` 改用 `request.stream()` 分块累计，超限立即中断
  并释放已缓冲字节，不再先 `await request.body()` 把超大请求整读入内存；对外状态码不变（仍 413
  `UPLOAD_TOO_LARGE`），仅内存占用与中断时机变化。

### Tests

- 锁定下载 `fileName` 的 CR/LF 清洗（node / py）。
