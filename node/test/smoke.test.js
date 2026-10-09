'use strict';

const { test } = require('node:test');
const assert = require('node:assert');
const Fastify = require('fastify');

const { storeApiPlugin, filterArchived, parseQueryParams } = require('../src/plugin');
const { convertValue } = require('../src/params');

/** 模拟 store 的 $set 点路径写入语义：'images.full' → row.images.full（Mongo $set {'a.b': v} 即嵌套写） */
function applySet(row, path, value) {
  if (path.startsWith('$')) return; // 原生操作符（$inc/$unset 等）本 mock 不模拟
  const keys = path.split('.');
  let cur = row;
  for (let i = 0; i < keys.length - 1; i += 1) {
    if (cur[keys[i]] == null || typeof cur[keys[i]] !== 'object') cur[keys[i]] = {};
    cur = cur[keys[i]];
  }
  cur[keys[keys.length - 1]] = value;
}

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
    if (row) {
      // 模拟 store 的 $set 语义：非操作符键自动包 $set（nodejs-store/src/crud/write.js），
      // 键内点路径由后端寻址（Mongo $set {'a.b': v} 即嵌套写入）。
      for (const [k, v] of Object.entries(data)) applySet(row, k, v);
    }
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

test('fail-secure 装配守卫：requireContext=true 且无 contextProvider ⇒ 装配期 ERR_SECURE_CONFIG', async () => {
  const store = new MockStore();
  store.requireContext = () => true;
  // 无 contextProvider：装配期拒绝（fail-fast，错误不推迟到运行期）
  await assert.rejects(() => buildApp(store), /ERR_SECURE_CONFIG/);
  // 补配 contextProvider：正常装配
  const app = await buildApp(store, { contextProvider: () => ({ uid: 'u1' }) });
  assert.ok(app);
  // requireContext=false（默认姿态）时无守卫，照常装配
  const open = new MockStore();
  open.requireContext = () => false;
  assert.ok(await buildApp(open));
});

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

test('文件下载路由：注入 resolver → 200 + 头；未注入 → 501；记录/字段缺失 → 404', async () => {
  const store = new MockStore();

  // 未注入 fileResolver → 501 FILE_NOT_CONFIGURED（决策 D，原 text/plain 兜底已废止）
  const bare = await buildApp(store);
  const rec = await bare.inject({ method: 'POST', url: '/api/user', payload: { name: 'a', age: 1 } });
  const id = rec.json().data._id;
  store.rows.get(id).file = 'hello';
  const notConfigured = await bare.inject({ method: 'GET', url: `/api/user/${id}/file` });
  assert.equal(notConfigured.statusCode, 501);
  assert.equal(notConfigured.json().error.code, 'FILE_NOT_CONFIGURED');

  // 注入 resolver → 200
  const app = await buildApp(store, {
    fileResolver: async () => ({ body: Buffer.from('hello'), contentType: 'text/plain; charset=utf-8', fileName: `user-${id}` }),
  });
  const file = await app.inject({ method: 'GET', url: `/api/user/${id}/file` });
  assert.equal(file.statusCode, 200);
  assert.equal(file.headers['content-type'], 'text/plain; charset=utf-8');
  assert.equal(file.headers['content-disposition'], `attachment; filename="user-${id}"`);
  assert.equal(file.body, 'hello');

  const missing = await app.inject({ method: 'GET', url: '/api/user/nope/file' });
  assert.equal(missing.statusCode, 404);
  assert.equal(missing.json().error.code, 'NOT_FOUND');
});

test('文件下载路由：资源无任何可读副本 → 404 NOT_FOUND（core 前缀 ERR_RESOURCE_NOT_FOUND:）', async () => {
  const store = new MockStore();
  // core 的 resource open 在「零副本行」时抛带稳定前缀的错误（spec/03 判定顺序第 4 层）
  const app = await buildApp(store, {
    fileResolver: async () => { throw new Error('ERR_RESOURCE_NOT_FOUND:资源不存在或无可读副本: sha1x'); },
  });
  const rec = await app.inject({ method: 'POST', url: '/api/user', payload: { name: 'a', age: 1 } });
  const id = rec.json().data._id;
  store.rows.get(id).file = 'sha1x'; // 字段非空才会走到 resolver（字段空是适配层判定的另一条 404）

  const res = await app.inject({ method: 'GET', url: `/api/user/${id}/file` });
  assert.equal(res.statusCode, 404);
  assert.equal(res.json().error.code, 'NOT_FOUND');
  assert.equal(res.json().error.message, '资源不存在或无可读副本: sha1x'); // 前缀已被剥离
});

test('文件下载路由：provider 读取失败（非前缀错误）→ 500 透传，不得伪装成 404', async () => {
  const store = new MockStore();
  const app = await buildApp(store, {
    fileResolver: async () => { throw new Error('io down'); },
  });
  const rec = await app.inject({ method: 'POST', url: '/api/user', payload: { name: 'a', age: 1 } });
  const id = rec.json().data._id;
  store.rows.get(id).file = 'sha1x';

  const res = await app.inject({ method: 'GET', url: `/api/user/${id}/file` });
  assert.equal(res.statusCode, 500);
  assert.equal(res.json().error.message, 'io down');
});

test('文件上传路由：注入 uploadResolver → 200 + 回写；未注入 → 501；空体 → 400（spec/01+02+03）', async () => {
  const store = new MockStore();

  // 未注入 uploadResolver → 501 UPLOAD_NOT_CONFIGURED
  const bare = await buildApp(store);
  const created = await bare.inject({ method: 'POST', url: '/api/user', payload: { name: 'a', age: 1 } });
  const id = created.json().data._id;
  const notConfigured = await bare.inject({
    method: 'POST', url: `/api/user/${id}/file`,
    headers: { 'content-type': 'application/octet-stream' }, payload: Buffer.from('bytes'),
  });
  assert.equal(notConfigured.statusCode, 501);
  assert.equal(notConfigured.json().error.code, 'UPLOAD_NOT_CONFIGURED');

  // 注入 uploadResolver → 200 且字段回写
  const app = await buildApp(store, { uploadResolver: async () => ({ ref: 'sha1abc' }) });
  const ok = await app.inject({
    method: 'POST', url: `/api/user/${id}/file`,
    headers: { 'content-type': 'application/octet-stream' }, payload: Buffer.from('bytes'),
  });
  assert.equal(ok.statusCode, 200);
  assert.equal(ok.json().data.file, 'sha1abc');
  assert.equal(store.rows.get(id).file, 'sha1abc');

  // 空体 → 400 EMPTY_BODY
  const empty = await app.inject({
    method: 'POST', url: `/api/user/${id}/file`,
    headers: { 'content-type': 'application/octet-stream' }, payload: Buffer.alloc(0),
  });
  assert.equal(empty.statusCode, 400);
  assert.equal(empty.json().error.code, 'EMPTY_BODY');

  // 超限 → 413 UPLOAD_TOO_LARGE（A4；用小 uploadLimit 触发，避免构造 32MB 大包）
  const small = await buildApp(store, { uploadResolver: async () => ({ ref: 'x' }), uploadLimit: 4 });
  const tooLarge = await small.inject({
    method: 'POST', url: `/api/user/${id}/file`,
    headers: { 'content-type': 'application/octet-stream' }, payload: Buffer.from('bytes'),
  });
  assert.equal(tooLarge.statusCode, 413);
  assert.equal(tooLarge.json().error.code, 'UPLOAD_TOO_LARGE');

  // 记录不存在 → 404
  const missing = await app.inject({
    method: 'POST', url: '/api/user/nope/file',
    headers: { 'content-type': 'application/octet-stream' }, payload: Buffer.from('bytes'),
  });
  assert.equal(missing.statusCode, 404);
  assert.equal(missing.json().error.code, 'NOT_FOUND');
});

test('上传/下载 field 点路径寻址（spec/02）', async () => {
  const store = new MockStore();
  const created = await buildApp(store).then((a) => a.inject({ method: 'POST', url: '/api/user', payload: { name: 'a', age: 1 } }));
  const id = created.json().data._id;
  store.rows.get(id).images = { full: 'old-ref' };
  const app = await buildApp(store, { uploadResolver: async () => ({ ref: 'new-ref' }) });
  const ok = await app.inject({
    method: 'POST', url: `/api/user/${id}/file?field=images.full`,
    headers: { 'content-type': 'application/octet-stream' }, payload: Buffer.from('bytes'),
  });
  assert.equal(ok.statusCode, 200);
  assert.equal(ok.json().data.images.full, 'new-ref');
});
