// Large payload (500KB+) INSERT benchmark for memory/performance investigation
//
// Uses xk6-sql API:
//   sql.open(driver, connString)
//   db.exec(sql, ...args)

import sql from 'k6/x/sql';
import driver from 'k6/x/sql/driver/postgres';
import { getConnectionString, getDefaultOptions } from './lib/config.js';
import { randomId, generateLargeJsonb } from './lib/data.js';
import { createSummaryHandler } from './lib/summary.js';

const target = __ENV.K6_TARGET || 'proxy';
const connectionString = getConnectionString(target);

export const options = getDefaultOptions({
  'iteration_duration': ['p(95)<30000'],  // 30s for large payloads
});

const db = sql.open(driver, connectionString);

export default function() {
  const id = randomId();
  const jsonb = generateLargeJsonb(id);

  db.exec(
    `INSERT INTO benchmark_encrypted (id, encrypted_jsonb_with_ste_vec) VALUES ($1, $2)`,
    id,
    JSON.stringify(jsonb)
  );
}

export function teardown() {
  db.close();
}

export const handleSummary = createSummaryHandler('jsonb-ste-vec-large-payload');
