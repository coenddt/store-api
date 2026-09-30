'use strict';

/**
 * 查询参数 → GQL params 绑定。
 * 规范依据：spec/02-params.md（双端逐字一致，改动必须先改 spec）。
 */

class ParamError extends Error {
  constructor(message) {
    super(message);
    this.code = 'INVALID_PARAM';
  }
}

const INT_RE = /^-?\d+$/;
const FLOAT_RE = /^-?\d+\.\d+$/;

/**
 * p.<name> 值的类型转换：true/false/null → 字面量；整数/浮点正则全匹配 → 数值；
 * `{`/`[` 开头 → JSON 解析（失败抛 ParamError，禁静默当字符串）；其余 → 字符串原样。
 */
function convertValue(raw) {
  if (raw === 'true') return true;
  if (raw === 'false') return false;
  if (raw === 'null') return null;
  if (INT_RE.test(raw)) return Number.parseInt(raw, 10);
  if (FLOAT_RE.test(raw)) return Number.parseFloat(raw);
  if (raw.startsWith('{') || raw.startsWith('[')) {
    try {
      return JSON.parse(raw);
    } catch (e) {
      throw new ParamError(`参数 JSON 解析失败: ${e.message}`);
    }
  }
  return raw;
}

/**
 * 从 query 对象收集 p.* 参数。同名重复出现 → 数组（每元素各自转换）；单次 → 标量。
 * @param {Record<string, string|string[]>} query
 */
function parseQueryParams(query) {
  const params = {};
  for (const [key, value] of Object.entries(query || {})) {
    if (!key.startsWith('p.')) continue;
    params[key.slice(2)] = Array.isArray(value) ? value.map(convertValue) : convertValue(value);
  }
  return params;
}

module.exports = { parseQueryParams, convertValue, ParamError };
