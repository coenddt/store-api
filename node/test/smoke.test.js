'use strict';

const { test } = require('node:test');
const assert = require('node:assert');
const Fastify = require('fastify');

const { storeApiPlugin, filterArchived, parseQueryParams } = require('../src/plugin');
const { convertValue } = require('../src/params');

/** mock store：只实现适配器调用面（query/queryOne/insert/update/remove/list/setContext） */
class MockStore {
  constructor() {
    this.rows = new Map();
    this.lastQuery = null;
    this.lastContext = undefined;
    const PermissionError = class PermissionError extends Error { };
    this.PermissionError = PermissionError;
  }

  list() { return ['user', 'userDeleted']; }

  get(name) {
    return name === 'user' ? { name: 'user', fields: { name: { type: 'string' }, age: { type: 'number' } } } : null;
  }

  async query(gql, params) {
    this.lastQuery = { gql, params };
    return [...this.rows.values()];
  }

  async queryOne(gql, params) {
    this.lastQuery = { gql, params };
    const id = params && params.c0 && params.c0._id;
    return this.rows.get(id) || null;
  }

  async insert(name, data) {
    const id = data._id || `u${this.rows.size + 1}`;
    const row = { _id: id, ...data };
    this.rows.set(id, row);
    return row;
  }

  async update(name, cond, data) {
    const row = this.rows.get(cond._id);
    if (row) Object.assign(row, data);
    return row || null;
  }

  async remove(name, cond) {
    const row = this.rows.get(cond._id) || null;
    this.rows.delete(cond._id);
    return row;
  }

  async setContext(ctx) { this.lastContext = ctx; }
}

async function buildApp(store, extraOpts = {}) {
  const app = Fastify();
  await app.register(storeApiPlugin, { store, prefix: '/api', ...extraOpts });
  await app.ready();
  return app;
}

test('归档表过滤：XxxDeleted 且 Xxx 存在时被排除', () => {
  assert.deepEqual(filterArchived(['user', 'userDeleted', 'post', 'logDeleted']), ['user', 'post', 'logDeleted']);
});

test('params 转换矩阵（spec/02-params.md）', () => {
  assert.deepEqual(
    parseQueryParams({ 'p.a': 'true', 'p.b': 'false', 'p.c': 'null', 'p.d': '42', 'p.e': '-1.5', 'p.f': 'abc' }),
    { a: true, b: false, c: null, d: 42, e: -1.5, f: 'abc' }
  );
  assert.deepEqual(parseQueryParams({ 'p.o': '{"age":{"$gte":18}}' }), { o: { age: { $gte: 18 } } });
  // 同名重复出现 → 数组（每元素各自转换）
  assert.deepEqual(parseQueryParams({ 'p.tag': ['a', 'b'] }), { tag: ['a', 'b'] });
});

test('params JSON 非法 → ParamError', () => {
  const { ParamError } = require('../src/params');
  assert.throws(() => parseQueryParams({ 'p.o': '{"age":' }), ParamError);
});

test('CRUD 全链路 + GQL 透传', async (t) => {
  const store = new MockStore();
  const app = await buildApp(store);

  const created = await app.inject({ method: 'POST', url: '/api/user', payload: { name: 'a', age: 1 } });
  assert.equal(created.statusCode, 201);
  assert.equal(created.json().data.name, 'a');
  assert.ok(created.json().data._id);

  const listed = await app.inject({ method: 'GET', url: '/api/user?q=($condition: @c0)&p.c0={"age":{"$gte":18}}' });
  assert.equal(listed.statusCode, 200);
  assert.ok(Array.isArray(listed.json().data));
  // 拼接结果 = 资源名 + q 原样（spec/02-params.md）；带 q 时投影完全由 q 决定
  assert.equal(store.lastQuery.gql, 'user($condition: @c0)');
  assert.deepEqual(store.lastQuery.params, { c0: { age: { $gte: 18 } } });

  const id = created.json().data._id;
  const listedNoQ = await app.inject({ method: 'GET', url: '/api/user' });
  assert.equal(listedNoQ.statusCode, 200);
  assert.equal(store.lastQuery.gql, 'user { name, age }'); // 无 q → 适配器生成全字段投影（spec/02）

  const got = await app.inject({ method: 'GET', url: `/api/user/${id}` });
  assert.equal(got.statusCode, 200);
  assert.equal(store.lastQuery.gql, 'user($condition: @c0) { name, age }');
  assert.deepEqual(store.lastQuery.params, { c0: { _id: id } });

  const patched = await app.inject({ method: 'PATCH', url: `/api/user/${id}`, payload: { age: 2 } });
  assert.equal(patched.statusCode, 200);
  assert.equal(patched.json().data.age, 2);

  const removed = await app.inject({ method: 'DELETE', url: `/api/user/${id}` });
  assert.equal(removed.statusCode, 200);

  const gone = await app.inject({ method: 'GET', url: `/api/user/${id}` });
  assert.equal(gone.statusCode, 404);
  assert.equal(gone.json().error.code, 'NOT_FOUND');
});

test('错误映射：400/403/401（spec/03-errors.md）', async () => {
  const store = new MockStore();
  const app = await buildApp(store, {
    contextProvider: (req) => {
      if (req.headers['x-user'] === 'denied') {
        const e = new store.PermissionError('权限不足');
        e.code = 'PERMISSION_DENIED';
        throw e;
      }
      if (req.headers['x-user'] === 'broken') throw new Error('token 解析失败');
      return { uid: req.headers['x-user'] || null };
    },
  });

  assert.equal(store.lastContext, undefined);

  const badBody = await app.inject({ method: 'POST', url: '/api/user', payload: [1, 2] });
  assert.equal(badBody.statusCode, 400);
  assert.equal(badBody.json().error.code, 'INVALID_BODY');

  const badParam = await app.inject({ method: 'GET', url: '/api/user?p.c0={"age":' });
  assert.equal(badParam.statusCode, 400);
  assert.equal(badParam.json().error.code, 'INVALID_PARAM');
  assert.ok(badParam.json().error.message.includes('JSON'));

  const denied = await app.inject({ method: 'GET', url: '/api/user', headers: { 'x-user': 'denied' } });
  assert.equal(denied.statusCode, 403);
  assert.equal(denied.json().error.code, 'PERMISSION_DENIED');
  assert.equal(denied.json().error.message, '权限不足');

  const broken = await app.inject({ method: 'GET', url: '/api/user', headers: { 'x-user': 'broken' } });
  assert.equal(broken.statusCode, 401);
  assert.equal(broken.json().error.code, 'CONTEXT_ERROR');
  assert.equal(broken.json().error.message, 'token 解析失败');

  const ok = await app.inject({ method: 'GET', url: '/api/user', headers: { 'x-user': 'u1' } });
  assert.equal(ok.statusCode, 200);
  assert.equal(ok.json().error, undefined); // 成功响应不得带 error 字段
  assert.deepEqual(store.lastContext, { uid: 'u1' });
});

test('store 异常透传为 500 且保留 code/message', async () => {
  const store = new MockStore();
  store.query = async () => {
    const e = new Error('连接超时');
    e.code = 'CONN_TIMEOUT';
    throw e;
  };
  const app = await buildApp(store);
  const res = await app.inject({ method: 'GET', url: '/api/user' });
  assert.equal(res.statusCode, 500);
  assert.equal(res.json().error.code, 'CONN_TIMEOUT');
  assert.equal(res.json().error.message, '连接超时');
});

test('x-cache 注记位（B6）：无 provider 恒 BYPASS（含错误响应）；provider=HIT 透传', async () => {
  const store = new MockStore();
  const app = await buildApp(store);

  const ok = await app.inject({ method: 'GET', url: '/api/user' });
  assert.equal(ok.headers['x-cache'], 'BYPASS');

  const missing = await app.inject({ method: 'GET', url: '/api/user/nope' });
  assert.equal(missing.statusCode, 404);
  assert.equal(missing.headers['x-cache'], 'BYPASS'); // 错误响应同样带注记

  store.cacheStatus = () => 'HIT';
  const hit = await app.inject({ method: 'GET', url: '/api/user' });
  assert.equal(hit.headers['x-cache'], 'HIT');
});
