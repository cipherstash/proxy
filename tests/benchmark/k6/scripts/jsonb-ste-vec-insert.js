// JSONB INSERT benchmark - primary CI benchmark for encrypted JSONB performance
//
// Uses xk6-sql API:
//   sql.open(driver, connString)
//   db.exec(sql, ...args)

import sql from 'k6/x/sql';
import driver from 'k6/x/sql/driver/postgres';
import { getConnectionString, getDefaultOptions } from './lib/config.js';
import { randomId, generateStandardJsonb } from './lib/data.js';
import { createSummaryHandler } from './lib/summary.js';

const target = __ENV.K6_TARGET || 'proxy';
const connectionString = getConnectionString(target);

export const options = getDefaultOptions({
  'iteration_duration': ['p(95)<500'],
});

const db = sql.open(driver, connectionString);

export default function() {
  const id = randomId();
  const jsonb = generateStandardJsonb(id);

  db.exec(
    `INSERT INTO benchmark_encrypted (id, encrypted_jsonb_with_ste_vec) VALUES ($1, $2)`,
    id,
    JSON.stringify(jsonb)
  );
}

export function teardown() {
  db.close();
}

export const handleSummary = createSummaryHandler('jsonb-ste-vec-insert');
