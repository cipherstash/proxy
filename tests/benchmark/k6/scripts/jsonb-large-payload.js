// Large payload INSERT benchmark for memory/performance investigation
// Uses encrypted_jsonb_extract (~250KB) and encrypted_jsonb_full (~500KB) columns
// Replicates customer scenario with realistic credit report structures
//
// Uses xk6-sql API:
//   sql.open(driver, connString)
//   db.exec(sql, ...args)

import sql from 'k6/x/sql';
import driver from 'k6/x/sql/driver/postgres';
import { getConnectionString, getDefaultOptions } from './lib/config.js';
import { randomId, generateExtractPayload, generateFullPayload } from './lib/data.js';
import { createSummaryHandler } from './lib/summary.js';

const target = __ENV.K6_TARGET || 'proxy';
const connectionString = getConnectionString(target);

// Payload mode: 'extract' (~250KB), 'full' (~500KB), or 'dual' (both columns)
const payloadMode = __ENV.K6_PAYLOAD_MODE || 'dual';

export const options = getDefaultOptions({
  'iteration_duration': ['p(95)<30000'],  // 30s for large payloads
});

const db = sql.open(driver, connectionString);

export default function() {
  const id = randomId();

  if (payloadMode === 'extract') {
    // ~250KB payload only
    const extractPayload = generateExtractPayload(id);
    db.exec(
      `INSERT INTO benchmark_encrypted (id, encrypted_jsonb_extract) VALUES ($1, $2)`,
      id,
      JSON.stringify(extractPayload)
    );
  } else if (payloadMode === 'full') {
    // ~500KB payload only
    const fullPayload = generateFullPayload(id);
    db.exec(
      `INSERT INTO benchmark_encrypted (id, encrypted_jsonb_full) VALUES ($1, $2)`,
      id,
      JSON.stringify(fullPayload)
    );
  } else {
    // Dual mode: insert both columns simultaneously (default)
    // This replicates the customer scenario that caused 25s+ timeouts
    const extractPayload = generateExtractPayload(id);
    const fullPayload = generateFullPayload(id);
    db.exec(
      `INSERT INTO benchmark_encrypted (id, encrypted_jsonb_extract, encrypted_jsonb_full) VALUES ($1, $2, $3)`,
      id,
      JSON.stringify(extractPayload),
      JSON.stringify(fullPayload)
    );
  }
}

export function teardown() {
  db.close();
}

export const handleSummary = createSummaryHandler('jsonb-large-payload');
