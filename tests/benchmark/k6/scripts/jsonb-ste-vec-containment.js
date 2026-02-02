// JSONB containment (@>) benchmark
//
// The @> operator on eql_v2_encrypted is confirmed working via integration tests:
//   packages/cipherstash-proxy-integration/src/select/jsonb_contains.rs
//
// Uses xk6-sql API:
//   sql.open(driver, connString)
//   db.exec(sql, ...args)

import sql from 'k6/x/sql';
import driver from 'k6/x/sql/driver/postgres';
import { getConnectionString, getDefaultOptions } from './lib/config.js';
import { createSummaryHandler } from './lib/summary.js';

const target = __ENV.K6_TARGET || 'proxy';
const connectionString = getConnectionString(target);

// ID range for containment benchmark data (isolated from other tests)
const ID_START = 2000000;
const ID_COUNT = 100;

export const options = getDefaultOptions({
  'iteration_duration': ['p(95)<200'],
});

const db = sql.open(driver, connectionString);

export function setup() {
  // Clean up any leftover data from crashed runs before inserting
  db.exec(`DELETE FROM benchmark_encrypted WHERE id BETWEEN $1 AND $2`, ID_START, ID_START + ID_COUNT - 1);

  // Insert seed data with known values for containment queries
  for (let i = 0; i < ID_COUNT; i++) {
    const id = ID_START + i;
    const jsonb = {
      id: id,
      string: `value${i % 10}`,
      number: i % 10,
      nested: { string: 'world', number: i },
    };
    db.exec(
      `INSERT INTO benchmark_encrypted (id, encrypted_jsonb_with_ste_vec) VALUES ($1, $2)`,
      id,
      JSON.stringify(jsonb)
    );
  }
}

export default function() {
  const i = Math.floor(Math.random() * 10);
  const pattern = JSON.stringify({ string: `value${i}` });

  // Use exec instead of query - we only need to verify the query runs, not inspect results
  // Query uses @> containment operator on encrypted JSONB
  db.exec(
    `SELECT id FROM benchmark_encrypted WHERE encrypted_jsonb_with_ste_vec @> $1 AND id BETWEEN $2 AND $3`,
    pattern,
    ID_START,
    ID_START + ID_COUNT - 1
  );
}

export function teardown() {
  // Clean up seed data
  db.exec(`DELETE FROM benchmark_encrypted WHERE id BETWEEN $1 AND $2`, ID_START, ID_START + ID_COUNT - 1);
  db.close();
}

export const handleSummary = createSummaryHandler('jsonb-ste-vec-containment');
