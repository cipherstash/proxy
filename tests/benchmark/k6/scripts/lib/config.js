// Connection configuration for k6 benchmarks
// Ports: postgres=5532, proxy=6432
//
// xk6-pgxpool API:
//   import pgxpool from 'k6/x/pgxpool'
//   const pool = pgxpool.open(connString, minConns, maxConns)
//   pgxpool.query(pool, sql, ...args)
//   pgxpool.exec(pool, sql, ...args)

export const POSTGRES_PORT = 5532;
export const PROXY_PORT = 6432;

export function getConnectionString(target) {
  // Default to 127.0.0.1 (works on Linux CI and macOS with --network=host)
  const host = __ENV.K6_DB_HOST || '127.0.0.1';
  const port = target === 'proxy' ? PROXY_PORT : POSTGRES_PORT;
  const user = __ENV.K6_DB_USER || 'cipherstash';
  const password = __ENV.K6_DB_PASSWORD || 'p@ssword';
  const database = __ENV.K6_DB_NAME || 'cipherstash';
  // Default sslmode=disable for local/CI; override via K6_DB_SSLMODE if needed
  const sslmode = __ENV.K6_DB_SSLMODE || 'disable';

  return `postgres://${user}:${password}@${host}:${port}/${database}?sslmode=${sslmode}`;
}

export function getPoolConfig() {
  return {
    minConns: parseInt(__ENV.K6_POOL_MIN || '2'),
    maxConns: parseInt(__ENV.K6_POOL_MAX || '10'),
  };
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
    thresholds: {
      'iteration_duration': ['p(95)<500'],
      ...thresholds,
    },
  };
}
