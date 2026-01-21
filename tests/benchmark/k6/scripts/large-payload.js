// Large payload (500KB+) INSERT benchmark for memory/performance investigation
//
// Uses xk6-pgxpool API:
//   pgxpool.open(connString, minConns, maxConns)
//   pgxpool.exec(pool, sql, ...args)

import pgxpool from 'k6/x/pgxpool';
import { getConnectionString, getPoolConfig, getDefaultOptions } from './lib/config.js';
import { randomId, generateLargeJsonb } from './lib/data.js';
import { createSummaryHandler } from './lib/summary.js';

const target = __ENV.K6_TARGET || 'proxy';
const connectionString = getConnectionString(target);
const poolConfig = getPoolConfig();

export const options = getDefaultOptions({
  'iteration_duration': ['p(95)<30000'],  // 30s for large payloads
});

const pool = pgxpool.open(connectionString, poolConfig.minConns, poolConfig.maxConns);

export default function() {
  const id = randomId();
  const jsonb = generateLargeJsonb(id);

  pgxpool.exec(
    pool,
    `INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)`,
    id,
    JSON.stringify(jsonb)
  );
}

export function teardown() {
  pgxpool.close(pool);
}

export const handleSummary = createSummaryHandler('large-payload');
