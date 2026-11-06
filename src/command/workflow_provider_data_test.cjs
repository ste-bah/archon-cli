const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const read = (file) => JSON.parse(fs.readFileSync(path.join(__dirname, file), 'utf8'));
const optionalString = (value) => value === null || (typeof value === 'string' && value.trim().length > 0);

test('embedded provider defaults conform to the registry data schema', () => {
  const rows = read('workflow_provider_defaults.json');
  assert.ok(Array.isArray(rows) && rows.length > 0);
  const ids = new Set();
  for (const row of rows) {
    assert.deepEqual(Object.keys(row).sort(), ['base_url_config', 'environment', 'id']);
    assert.equal(typeof row.id, 'string');
    assert.ok(row.id.length > 0 && !ids.has(row.id));
    ids.add(row.id);
    assert.ok(optionalString(row.base_url_config));
    if (row.base_url_config !== null) assert.ok(row.base_url_config.startsWith('/'));
    const env = row.environment;
    assert.deepEqual(Object.keys(env).sort(), ['api_key_env', 'base_url', 'base_url_env', 'credential_env']);
    for (const key of ['base_url', 'base_url_env', 'api_key_env']) assert.ok(optionalString(env[key]));
    assert.ok(Array.isArray(env.credential_env));
    for (const name of [env.api_key_env, env.base_url_env, ...env.credential_env].filter(x => x !== null)) {
      assert.match(name, /^[A-Za-z_][A-Za-z0-9_]*$/);
    }
    assert.equal(new Set(env.credential_env).size, env.credential_env.length);
  }
});
