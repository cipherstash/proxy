// JSONB containment (@>) benchmark
//
// The @> operator on eql_v2_encrypted is confirmed working via integration tests:
//   packages/cipherstash-proxy-integration/src/select/jsonb_contains.rs:13-14
//   "SELECT encrypted_jsonb @> $1 FROM encrypted LIMIT 1"
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

// ID range for containment benchmark data (isolated from other tests)
const ID_START = 2000000;
const ID_COUNT = 100;

export const options = getDefaultOptions({
  'iteration_duration': ['p(95)<200'],
});

const pool = pgxpool.open(connectionString, poolConfig.minConns, poolConfig.maxConns);

export function setup() {
  // Clean up any leftover data from crashed runs before inserting
  pgxpool.exec(pool, `DELETE FROM encrypted WHERE id BETWEEN $1 AND $2`, ID_START, ID_START + ID_COUNT - 1);

  // Insert seed data with known values for containment queries
  for (let i = 0; i < ID_COUNT; i++) {
    const id = ID_START + i;
    const jsonb = {
      id: id,
      string: `value${i % 10}`,
      number: i % 10,
      nested: { string: 'world', number: i },
    };
    pgxpool.exec(
      pool,
      `INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)`,
      id,
      JSON.stringify(jsonb)
    );
  }
}

export default function() {
  const i = Math.floor(Math.random() * 10);
  const pattern = JSON.stringify({ string: `value${i}` });

  // Use exec instead of query - we only need to verify the query runs, not inspect results
  // This avoids potential resource leaks if pgxpool.query returns a cursor
  // Query uses @> containment operator on encrypted JSONB
  pgxpool.exec(
    pool,
    `SELECT id FROM encrypted WHERE encrypted_jsonb @> $1 AND id BETWEEN $2 AND $3`,
    pattern,
    ID_START,
    ID_START + ID_COUNT - 1
  );
}

export function teardown() {
  // Clean up seed data
  pgxpool.exec(pool, `DELETE FROM encrypted WHERE id BETWEEN $1 AND $2`, ID_START, ID_START + ID_COUNT - 1);
  pgxpool.close(pool);
}

export const handleSummary = createSummaryHandler('jsonb-containment');
