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

/**
 * 判定顺序（spec/03-errors.md）：
 * 适配层守卫(400) → PermissionError(403) → queryOne 空结果(404) → 其余 500 透传
 * @param {Error} err
 * @param {Function|null} PermissionErrorClass store 的权限错误类（按类型判定，禁按 message 匹配）
 */
function mapError(err, PermissionErrorClass) {
  if (err && (err.code === 'INVALID_PARAM' || err.code === 'INVALID_BODY' || err.code === 'NOT_FOUND')) {
    return { statusCode: Number(err.code === 'NOT_FOUND' ? 404 : 400), body: errorPayload(err.code, err.message) };
  }
  if (err && err.code === 'CONTEXT_ERROR') {
    return { statusCode: 401, body: errorPayload('CONTEXT_ERROR', err.message) };
  }
  if (PermissionErrorClass && err instanceof PermissionErrorClass) {
    return { statusCode: 403, body: errorPayload(storeCode(err), err.message) };
  }
  // spec/03 判定顺序第 3 层：GQL 解析失败（core 稳定前缀 ERR_GQL_PARSE:，与
  // ERR_PERM_PREFIX 同构的类型级契约——前缀判定非文案脆弱匹配）→ 400 GQL_PARSE，
  // message 剥前缀取原文（与 go/rust/py 同语义）。
  if (err && typeof err.message === 'string' && err.message.startsWith('ERR_GQL_PARSE:')) {
    return { statusCode: 400, body: errorPayload('GQL_PARSE', err.message.slice('ERR_GQL_PARSE:'.length)) };
  }
  return { statusCode: 500, body: errorPayload(storeCode(err), err ? err.message : null) };
}

module.exports = { errorPayload, storeCode, mapError, invalidBody, notFound };
