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