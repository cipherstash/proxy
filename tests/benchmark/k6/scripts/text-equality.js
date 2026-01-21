// Text equality benchmark - baseline matching pgbench encrypted transaction
//
// Uses xk6-pgxpool API:
//   pgxpool.open(connString, minConns, maxConns)
//   pgxpool.exec(pool, sql, ...args)

import pgxpool from 'k6/x/pgxpool';
import { getConnectionString, getPoolConfig, getDefaultOptions } from './lib/config.js';
import { createSummaryHandler } from './lib/summary.js';

const target = __ENV.K6_TARGET || 'proxy';
const connectionString = getConnectionString(target);
const poolConfig = getPoolConfig();

// ID range for text-equality benchmark data (isolated from other tests)
const ID_START = 1000000;
const ID_COUNT = 100;

export const options = getDefaultOptions({
  'iteration_duration': ['p(95)<100'],
});

const pool = pgxpool.open(connectionString, poolConfig.minConns, poolConfig.maxConns);

export function setup() {
  // Clean up any leftover data from crashed runs before inserting
  pgxpool.exec(pool, `DELETE FROM benchmark_encrypted WHERE id BETWEEN $1 AND $2`, ID_START, ID_START + ID_COUNT - 1);

  // Insert seed data for queries
  for (let i = 0; i < ID_COUNT; i++) {
    const id = ID_START + i;
    const email = `user${i}@example.com`;
    pgxpool.exec(
      pool,
      `INSERT INTO benchmark_encrypted (id, username, email) VALUES ($1, $2, $3)`,
      id,
      `user${i}`,
      email
    );
  }
}

export default function() {
  const i = Math.floor(Math.random() * ID_COUNT);
  const email = `user${i}@example.com`;

  // Use exec instead of query - we only need to verify the query runs, not inspect results
  // This avoids potential resource leaks if pgxpool.query returns a cursor
  pgxpool.exec(pool, `SELECT username FROM benchmark_encrypted WHERE email = $1`, email);
}

export function teardown() {
  // Clean up seed data
  pgxpool.exec(pool, `DELETE FROM benchmark_encrypted WHERE id BETWEEN $1 AND $2`, ID_START, ID_START + ID_COUNT - 1);
  pgxpool.close(pool);
}

export const handleSummary = createSummaryHandler('text-equality');
