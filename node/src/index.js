'use strict';

/**
 * store-api-node — 为 nodejs-store schema 自动生成 RESTful API 的 Fastify 适配器。
 *
 *   const { storeApiPlugin } = require('store-api-node');
 *   await fastify.register(storeApiPlugin, { store, prefix: '/api' });
 */

const { storeApiPlugin, filterArchived, parseQueryParams, errorPayload, ParamError } = require('./plugin');

module.exports = { storeApiPlugin, filterArchived, parseQueryParams, errorPayload, ParamError };
