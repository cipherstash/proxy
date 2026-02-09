// Text equality benchmark - baseline matching pgbench encrypted transaction
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

// ID range for text-equality benchmark data (isolated from other tests)
const ID_START = 1000000;
const ID_COUNT = 100;

export const options = getDefaultOptions({
  'iteration_duration': ['p(95)<100'],
});

const db = sql.open(driver, connectionString);

export function setup() {
  // Clean up any leftover data from crashed runs before inserting
  db.exec(`DELETE FROM benchmark_encrypted WHERE id BETWEEN $1 AND $2`, ID_START, ID_START + ID_COUNT - 1);

  // Insert seed data for queries
  for (let i = 0; i < ID_COUNT; i++) {
    const id = ID_START + i;
    const email = `user${i}@example.com`;
    db.exec(
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
  db.exec(`SELECT username FROM benchmark_encrypted WHERE email = $1`, email);
}

export function teardown() {
  // Clean up seed data
  db.exec(`DELETE FROM benchmark_encrypted WHERE id BETWEEN $1 AND $2`, ID_START, ID_START + ID_COUNT - 1);
  db.close();
}

export const handleSummary = createSummaryHandler('text-equality');
