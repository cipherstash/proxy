// Connection configuration for k6 benchmarks
// Ports: postgres=5532, proxy=6432
//
// xk6-sql API:
//   import sql from 'k6/x/sql';
//   import driver from 'k6/x/sql/driver/postgres';
//   const db = sql.open(driver, connString);
//   db.exec(sql, ...args);
//   const rows = db.query(sql, ...args);
//   db.close();

export const POSTGRES_PORT = 5532;
export const PROXY_PORT = 6432;

export function getConnectionString(target) {
  // Default to host.docker.internal (works on macOS and Windows Docker)
  // For Linux CI with --network=host, set K6_DB_HOST=127.0.0.1
  const host = __ENV.K6_DB_HOST || 'host.docker.internal';
  const port = target === 'proxy' ? PROXY_PORT : POSTGRES_PORT;
  const user = __ENV.K6_DB_USER || 'cipherstash';
  const password = __ENV.K6_DB_PASSWORD || 'p@ssword';
  const database = __ENV.K6_DB_NAME || 'cipherstash';
  // Default sslmode=disable for local/CI; override via K6_DB_SSLMODE if needed
  const sslmode = __ENV.K6_DB_SSLMODE || 'disable';

  return `postgres://${user}:${password}@${host}:${port}/${database}?sslmode=${sslmode}`;
}

export function getDefaultOptions(thresholds = {}) {
  return {
    scenarios: {
      default: {
        executor: 'constant-vus',
        vus: parseInt(__ENV.K6_VUS || '10'),
        duration: __ENV.K6_DURATION || '30s',
      },
    },
    summaryTrendStats: ['min', 'avg', 'med', 'p(90)', 'p(95)', 'p(99)', 'max'],
    thresholds: {
      'iteration_duration': ['p(95)<500'],
      ...thresholds,
    },
  };
}
