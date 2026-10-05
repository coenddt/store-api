'use strict';

/**
 * store-api-node — Fastify 适配器：为已注册 schema 自动生成 RESTful 路由。
 * 路由/参数/错误/上下文语义全部以 spec/*.md 为唯一依据（双端 parity，改动先改 spec）。
 */

const { parseQueryParams, ParamError } = require('./params');
const { mapError, errorPayload, invalidBody, notFound } = require('./errors');

const ARCHIVE_SUFFIX = 'Deleted';

/**
 * 归档表过滤（spec/01-routing.md）：`XxxDeleted` 且 `Xxx` 也在列表中 ⇒ 视为归档表。
 * @param {string[]} names
 */
function filterArchived(names) {
  const set = new Set(names);
  return names.filter((n) => !(n.endsWith(ARCHIVE_SUFFIX) && set.has(n.slice(0, -ARCHIVE_SUFFIX.length))));
}

/** nodejs-store 是 optional peer：仅在未显式传入 errors.PermissionError 时才加载 */
function requirePermissionError() {
  try {
    // eslint-disable-next-line global-require
    return require('nodejs-store').PermissionError;
  } catch (e) {
    const err = new Error(
      'store-api-node 需要 PermissionError 类：安装 nodejs-store，或在插件选项传 errors.PermissionError'
    );
    err.code = 'DEPENDENCY_MISSING';
    throw err;
  }
}

/**
 * @param {import('fastify').FastifyInstance} fastify
 * @param {object} opts
 * @param {object} opts.store nodejs-store 的 store 实例（需已 init + register）
 * @param {string} [opts.prefix] 路由前缀（由 fastify register 的 options.prefix 机制应用，插件内不重复拼接）
 * @param {string} [opts.idField='_id'] 单条路由主键字段名（双端一致，spec/01-routing.md）
 * @param {(req) => any} [opts.contextProvider] 每请求上下文钩子（spec/04-context.md）
 * @param {string[]} [opts.resources] 显式资源名；缺省取 store.list() 并过滤归档表
 * @param {{PermissionError?: Function}} [opts.errors] 显式传入 store 错误类（免依赖 nodejs-store）
 */
async function storeApiPlugin(fastify, opts) {
  const {
    store,
    idField = '_id',
    contextProvider = null,
    resources = null,
    errors = null,
    fileField = 'file',
    fileResolver = null,
  } = opts;
  if (!store) throw new Error('storeApiPlugin 需要 opts.store（nodejs-store 的 store 实例）');
  // 权限错误类来源（双端一致）：显式 errors.PermissionError → store 实例属性 → 加载 nodejs-store
  const PermissionErrorClass = (errors && errors.PermissionError) || store.PermissionError || requirePermissionError();

  if (contextProvider) {
    fastify.addHook('onRequest', async (req) => {
      let ctx;
      try {
        ctx = await contextProvider(req);
      } catch (e) {
        // spec/04-context.md：PermissionError ⇒ 403（RBAC 拒绝）；其余 ⇒ 401 CONTEXT_ERROR，message 原样透传
        if (e instanceof PermissionErrorClass) throw e;
        const err = new Error(e && e.message ? e.message : null);
        err.code = 'CONTEXT_ERROR';
        throw err;
      }
      // spec/04：返回 null/undefined 时同样显式注入空上下文（store.setContext(null) 语义为清除，
      // 有状态持有的运行时禁止残留上一请求上下文，防身份跨请求泄漏）
      await store.setContext(ctx);
    });
  }

  // 单一错误出口：handler / onRequest 钩子抛出的所有错误统一走 mapError 判定
  // （禁 catch 后洗成成功；hook 错误不经过 handler 内 catch，必须由框架错误处理器承接）
  fastify.setErrorHandler((err, req, reply) => {
    const mapped = mapError(err, PermissionErrorClass);
    reply.code(mapped.statusCode);
    return mapped.body;
  });

  // B6：`x-cache` 响应头注记位——取值唯一来源为宿主 store.cacheStatus（未实现时恒 BYPASS）。
  // 覆盖所有响应（含错误响应）。非法值回落 BYPASS（宿主侧已对非法/异常走反馈通道留痕）。
  const CACHE_VALUES = ['HIT', 'MISS', 'BYPASS'];
  fastify.addHook('onSend', async (req, reply, payload) => {
    const v = typeof store.cacheStatus === 'function' ? store.cacheStatus() : 'BYPASS';
    reply.header('x-cache', CACHE_VALUES.includes(v) ? v : 'BYPASS');
    return payload;
  });

  function requireBody(body) {
    if (body == null || typeof body !== 'object' || Array.isArray(body)) {
      throw invalidBody('请求体必须是 JSON 对象');
    }
    return body;
  }

  /**
   * 从 store 元数据生成显式投影串（` { f1, f2 }`）。
   * store.get(name) 缺失 / fields 为空 → 空串（无投影，data 仅 _id——上游 schema 定义不完整的显式后果）。
   */
  function schemaProjection(store, name) {
    const meta = typeof store.get === 'function' ? store.get(name) : null;
    const fields = meta && meta.fields ? Object.keys(meta.fields) : [];
    return fields.length ? ` { ${fields.join(', ')} }` : '';
  }

  const names = resources || filterArchived(store.list());
  for (const name of names) {
    const base = `/${name}`;
    const oneParams = (id) => ({ c0: { [idField]: id } });
    // 显式投影（spec/01+02）：GQL 省略投影段 = 只返回 _id（三端 core 契约）。
    // 列表路由仅在 q 缺失时用它；q 存在时投影完全由 q 决定，适配层不追加。
    const proj = schemaProjection(store, name);

    fastify.get(base, async (req) => {
      const params = parseQueryParams(req.query);
      const gql = name + (req.query.q || proj);
      return { data: await store.query(gql, params) };
    });

    fastify.get(`${base}/:id`, async (req) => {
      const data = await store.queryOne(`${name}($condition: @c0)${proj}`, oneParams(req.params.id));
      if (data == null) throw notFound(`记录不存在: ${idField}=${req.params.id}`);
      return { data };
    });

    fastify.get(`${base}/:id/file`, async (req, reply) => {
      const rec = await store.queryOne(
        `${name}($condition: @c0)${proj}`,
        { c0: { [idField]: req.params.id } },
      );
      if (rec == null) throw notFound(`记录不存在: ${idField}=${req.params.id}`);
      const raw = rec[fileField];
      if (raw == null) throw notFound(`文件不存在: ${fileField}=${req.params.id}`);
      const out = fileResolver
        ? await fileResolver(req, rec, req.params.id)
        : { body: String(raw), contentType: 'text/plain; charset=utf-8', fileName: `${name}-${req.params.id}` };
      reply.header('content-type', out.contentType || 'application/octet-stream');
      reply.header('content-disposition', `attachment; filename="${out.fileName || 'file'}"`);
      return out.body;
    });

    fastify.post(base, async (req, reply) => {
      const body = requireBody(req.body);
      reply.code(201);
      return { data: await store.insert(name, body) };
    });

    fastify.patch(`${base}/:id`, async (req) => {
      const body = requireBody(req.body);
      return { data: await store.update(name, { [idField]: req.params.id }, body) };
    });

    fastify.delete(`${base}/:id`, async (req) => (
      { data: await store.remove(name, { [idField]: req.params.id }) }
    ));
  }
}

module.exports = { storeApiPlugin, filterArchived, parseQueryParams, errorPayload, ParamError };
