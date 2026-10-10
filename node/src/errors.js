'use strict';

/**
 * 错误 → HTTP 状态码映射。
 * 规范依据：spec/03-errors.md（判定顺序双端一致，改动必须先改 spec）。
 * 遵循 no-error-masking：message 原样透传、取不到置 null（禁伪造），成功响应不得带 error 字段。
 */

function errorPayload(code, message) {
  return { error: { code, message: message == null ? null : message } };
}

/** store 错误的分类码：优先 err.code，缺省用错误类名，再缺省用通用码 */
function storeCode(err) {
  return (err && err.code) || (err && err.name) || 'STORE_ERROR';
}

function invalidBody(message) {
  const err = new Error(message);
  err.code = 'INVALID_BODY';
  return err;
}

function notFound(message) {
  const err = new Error(message);
  err.code = 'NOT_FOUND';
  return err;
}

// ── 资源字节上传/下载守卫工厂（spec/03-errors.md） ──
function emptyBody(message) {
  const err = new Error(message == null ? '上传请求体为空' : message);
  err.code = 'EMPTY_BODY';
  return err;
}

function tooLarge(message) {
  const err = new Error(message == null ? '上传字节体超过上限' : message);
  err.code = 'UPLOAD_TOO_LARGE';
  return err;
}

function uploadNotConfigured(message) {
  const err = new Error(message == null ? '未注入 uploadResolver' : message);
  err.code = 'UPLOAD_NOT_CONFIGURED';
  return err;
}

function fileNotConfigured(message) {
  const err = new Error(message == null ? '未注入 fileResolver' : message);
  err.code = 'FILE_NOT_CONFIGURED';
  return err;
}

/**
 * 判定顺序第 1、2 层（spec/03-errors.md）：未注入 resolver(501) 与适配层守卫(400/413)。
 * 旧的 INVALID_PARAM/INVALID_BODY/NOT_FOUND 三连 if 整体并入本表，避免双份判断漂移。
 */
const GUARD_STATUS = {
  INVALID_PARAM: 400,
  INVALID_BODY: 400,
  NOT_FOUND: 404,
  EMPTY_BODY: 400,
  UPLOAD_TOO_LARGE: 413,
  UPLOAD_NOT_CONFIGURED: 501,
  FILE_NOT_CONFIGURED: 501,
};

/**
 * 判定顺序（spec/03-errors.md）：
 * 未注入 resolver / 适配层守卫(400/413/501) → CONTEXT_ERROR(401) → 权限类(403)
 * → GQL 解析失败(400) → 其余 500 透传
 * @param {Error} err
 * @param {Function|null} PermissionErrorClass store 的权限错误类（按类型判定，禁按 message 匹配）
 */
function mapError(err, PermissionErrorClass) {
  // parser 阶段 bodyLimit 抢先拒绝（Fastify FST_ERR_CTP_BODY_TOO_LARGE，§4.2 parser bodyLimit）
  // → 413 UPLOAD_TOO_LARGE（spec/03；error.code 与 payload code 不同，单列一支）
  if (err && err.code === 'FST_ERR_CTP_BODY_TOO_LARGE') {
    return { statusCode: 413, body: errorPayload('UPLOAD_TOO_LARGE', err.message) };
  }
  // spec/03 判定顺序第 1、2 层：未注入 resolver(501) 与适配层守卫(400/413)
  if (err && GUARD_STATUS[err.code] !== undefined) {
    return { statusCode: GUARD_STATUS[err.code], body: errorPayload(err.code, err.message) };
  }
  if (err && err.code === 'CONTEXT_ERROR') {
    return { statusCode: 401, body: errorPayload('CONTEXT_ERROR', err.message) };
  }
  if (PermissionErrorClass && err instanceof PermissionErrorClass) {
    return { statusCode: 403, body: errorPayload(storeCode(err), err.message) };
  }
  // spec/03 判定顺序第 3 层：权限类同档 —— NoContext（requireContext 开启且 ctx 缺失）。
  // 宿主（nodejs-store）抛 NoContextError 且带稳定 machine code `no_context`；按 code 判定，
  // 禁按 message 匹配（宿主已剥前缀）→ 403，与 PermissionError 同档。
  if (err && err.code === 'no_context') {
    return { statusCode: 403, body: errorPayload(storeCode(err), err.message) };
  }
  // spec/03 判定顺序第 4 层：GQL 解析失败（core 稳定前缀 ERR_GQL_PARSE:，与
  // ERR_PERM_PREFIX 同构的类型级契约——前缀判定非文案脆弱匹配）→ 400 GQL_PARSE，
  // message 剥前缀取原文（与 go/rust/py 同语义）。
  if (err && typeof err.message === 'string' && err.message.startsWith('ERR_GQL_PARSE:')) {
    return { statusCode: 400, body: errorPayload('GQL_PARSE', err.message.slice('ERR_GQL_PARSE:'.length)) };
  }
  // spec/03 判定顺序第 4 层（core 稳定前缀类）：资源无任何可读副本
  // （core nodejs-store resource open → ERR_RESOURCE_NOT_FOUND:）→ 404 NOT_FOUND，
  // message 剥前缀取原文（与 GQL_PARSE 同构；禁按文案匹配）。
  if (err && typeof err.message === 'string' && err.message.startsWith('ERR_RESOURCE_NOT_FOUND:')) {
    return { statusCode: 404, body: errorPayload('NOT_FOUND', err.message.slice('ERR_RESOURCE_NOT_FOUND:'.length)) };
  }
  return { statusCode: 500, body: errorPayload(storeCode(err), err ? err.message : null) };
}

module.exports = {
  errorPayload, storeCode, mapError,
  invalidBody, notFound,
  emptyBody, tooLarge, uploadNotConfigured, fileNotConfigured,
};
