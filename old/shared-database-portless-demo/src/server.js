// Archived proof-of-concept server.
import { createServer } from 'node:http';
import pg from 'pg';
import { createHandler } from './app.js';

const port = Number.parseInt(process.env.PORT ?? '8080', 10);
const instanceName = process.env.INSTANCE_NAME ?? 'app';
const databaseUrl = process.env.DATABASE_URL;

if (!databaseUrl) {
  throw new Error('DATABASE_URL is required');
}

const pool = new pg.Pool({ connectionString: databaseUrl });

await pool.query(`
  CREATE TABLE IF NOT EXISTS visits (
    id uuid PRIMARY KEY,
    instance_name text NOT NULL,
    visited_at timestamptz NOT NULL DEFAULT now()
  )
`);

const server = createServer(createHandler({ database: pool, instanceName }));
server.listen(port, '0.0.0.0', () => {
  console.log(`${instanceName} listening on port ${port}`);
});

async function shutdown(signal) {
  console.log(`${signal} received; shutting down`);
  server.close(async () => {
    await pool.end();
    process.exit(0);
  });
}

process.on('SIGINT', () => shutdown('SIGINT'));
process.on('SIGTERM', () => shutdown('SIGTERM'));
