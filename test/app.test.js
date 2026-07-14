import assert from 'node:assert/strict';
import { once } from 'node:events';
import { createServer } from 'node:http';
import test from 'node:test';
import { createHandler } from '../src/app.js';

async function withServer(database, callback) {
  const server = createServer(createHandler({ database, instanceName: 'test-one' }));
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const { port } = server.address();
  try {
    await callback(`http://127.0.0.1:${port}`);
  } finally {
    server.close();
    await once(server, 'close');
  }
}

test('health reports the serving instance', async () => {
  const database = { query: async () => ({ rows: [] }) };
  await withServer(database, async (baseUrl) => {
    const response = await fetch(`${baseUrl}/health`);
    assert.equal(response.status, 200);
    assert.deepEqual(await response.json(), { status: 'ok', instance: 'test-one' });
  });
});
test('a visit is written and shared history is returned', async () => {
  const calls = [];
  const database = {
    async query(sql, values) {
      calls.push({ sql, values });
      return sql.startsWith('SELECT id')
        ? { rows: [{ id: 'existing', instance_name: 'test-two' }] }
        : { rows: [] };
    },
  };

  await withServer(database, async (baseUrl) => {
    const response = await fetch(baseUrl);
    const body = await response.json();
    assert.equal(response.status, 200);
    assert.equal(body.servedBy, 'test-one');
    assert.equal(body.visits[0].instance_name, 'test-two');
    assert.equal(calls[0].values[1], 'test-one');
  });
});

test('unknown routes return 404', async () => {
  const database = { query: async () => ({ rows: [] }) };
  await withServer(database, async (baseUrl) => {
    const response = await fetch(`${baseUrl}/missing`);
    assert.equal(response.status, 404);
  });
});
