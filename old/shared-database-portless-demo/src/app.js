// Archived proof-of-concept application.
import { randomUUID } from 'node:crypto';

function sendJson(response, status, body) {
  response.writeHead(status, { 'content-type': 'application/json; charset=utf-8' });
  response.end(`${JSON.stringify(body)}\n`);
}

export function createHandler({ database, instanceName }) {
  return async function handler(request, response) {
    try {
      const url = new URL(request.url, 'http://localhost');

      if (request.method === 'GET' && url.pathname === '/health') {
        await database.query('SELECT 1');
        return sendJson(response, 200, { status: 'ok', instance: instanceName });
      }

      if (request.method === 'GET' && url.pathname === '/') {
        const id = randomUUID();
        await database.query(
          'INSERT INTO visits (id, instance_name) VALUES ($1, $2)',
          [id, instanceName],
        );
        const result = await database.query(
          'SELECT id, instance_name, visited_at FROM visits ORDER BY visited_at, id',
        );
        return sendJson(response, 200, {
          message: 'Both service instances use this shared visit history.',
          servedBy: instanceName,
          currentVisitId: id,
          visits: result.rows,
        });
      }

      return sendJson(response, 404, { error: 'not found' });
    } catch (error) {
      console.error(error);
      return sendJson(response, 503, { error: 'database unavailable' });
    }
  };
}
