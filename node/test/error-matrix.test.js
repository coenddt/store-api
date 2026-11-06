'use strict';

/**
 * 映射矩阵：每档错误语义 → REST 状态码（A3「可程序化区分」验收）。
 * 直接调纯函数 mapError（无需真实 store/库），用例与 py/tests/test_error_matrix.py 同构。
 * 规范依据：spec/03-errors.md 判定顺序 + core machine code（`no_context` / `permission_denied`）。
 */

const { test } = require('node:test');
const assert = require('node:assert');

const { mapError } = require('../src/errors');

// store 权限错误类（按类型判定，禁按 message 匹配）
class PermissionError extends Error {}

test('映射矩阵：Permission 档 → 403', () => {
  const err = new PermissionError('无访问权限');
  err.code = 'permission_denied';
  const r = mapError(err, PermissionError);
  assert.equal(r.statusCode, 403);
  assert.equal(r.body.error.code, 'permission_denied');
  assert.equal(r.body.error.message, '无访问权限');
});

test('映射矩阵：NoContext 档 → 403（code=no_context）', () => {
  const err = new Error('上下文缺失');
  err.code = 'no_context';
  const r = mapError(err, PermissionError);
  assert.equal(r.statusCode, 403);
  assert.equal(r.body.error.code, 'no_context');
  assert.equal(r.body.error.message, '上下文缺失');
});

test('映射矩阵：ERR_GQL_PARSE 档 → 400（剥离前缀）', () => {
  const r = mapError(new Error('ERR_GQL_PARSE:意外的 token'), PermissionError);
  assert.equal(r.statusCode, 400);
  assert.equal(r.body.error.code, 'GQL_PARSE');
  assert.equal(r.body.error.message, '意外的 token');
});

test('映射矩阵：Other 档 → 500 透传原文（禁静默）', () => {
  const err = new Error('连接超时');
  err.code = 'CONN_TIMEOUT';
  const r = mapError(err, PermissionError);
  assert.equal(r.statusCode, 500);
  assert.equal(r.body.error.code, 'CONN_TIMEOUT');
  assert.equal(r.body.error.message, '连接超时');
});

test('映射矩阵：ERR_NOT_FOUND 写探针未命中 → 404（code=not_found，message 原样）', () => {
  const message = 'ERR_NOT_FOUND:目标记录不存在（写条件未命中任何行，_id 不存在或已被删除）';
  const r = mapError(new Error(message), PermissionError);
  assert.equal(r.statusCode, 404);
  assert.equal(r.body.error.code, 'not_found');
  assert.equal(r.body.error.message, message);
  // 宿主只给 machine code（无前缀 message）同样 404
  const byCode = new Error('not found');
  byCode.code = 'not_found';
  assert.equal(mapError(byCode, PermissionError).statusCode, 404);
});

test('映射矩阵：非法/空 JSON 请求体（Fastify parser 错误）→ 400 INVALID_BODY', () => {
  for (const code of ['FST_ERR_CTP_INVALID_JSON_BODY', 'FST_ERR_CTP_EMPTY_JSON_BODY']) {
    const err = new Error('body parse failed');
    err.code = code;
    const r = mapError(err, PermissionError);
    assert.equal(r.statusCode, 400, code);
    assert.equal(r.body.error.code, 'INVALID_BODY', code);
    assert.equal(r.body.error.message, 'body parse failed');
  }
});

test('跨端 code 口径：ERR_* 前缀与 rust store_code 同输入同 code', () => {
  const cases = [
    ['ERR_NOT_FOUND:目标记录不存在（写条件未命中任何行，_id 不存在或已被删除）', 'not_found'],
    ['ERR_NOT_FOUND', 'not_found'],
    ['ERR_NO_CONTEXT:缺少请求上下文', 'no_context'],
    ['ERR_NO_CONTEXT', 'no_context'],
    ['ERR_PERMISSION:RBAC:无权写入', 'RBAC'],
    ['ERR_TEXT2QUERY:档位不支持', 'ERR_TEXT2QUERY'],
    ['ERR_GQL_PARSE:意外的 token', 'GQL_PARSE'],
  ];
  for (const [message, code] of cases) {
    const r = mapError(new Error(message), PermissionError);
    assert.equal(r.body.error.code, code, `${message} ⇒ ${code}`);
    assert.equal(r.body.error.message === null, false);
  }
  // 未知错误保持原样，不吞成通用错误
  const unknown = new Error('连接超时');
  unknown.code = 'CONN_TIMEOUT';
  assert.equal(mapError(unknown, PermissionError).body.error.code, 'CONN_TIMEOUT');
});