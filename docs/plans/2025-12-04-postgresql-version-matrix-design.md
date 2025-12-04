# PostgreSQL Version Matrix for CI

## Problem

PostgreSQL 18 was released and the `postgres:latest` Docker image broke volume mounting (commit `3e4fff24`). CI was pinned to PostgreSQL 17. We need to:
1. Support PostgreSQL 18 (assuming upstream fix)
2. Expand test coverage across multiple PostgreSQL versions
3. Catch future breaking changes early

## Decision

Run a full parallel matrix in CI testing PostgreSQL versions 14, 15, 16, 17, and 18.

## Design

### GitHub Actions Matrix Strategy

Update `.github/workflows/test.yml`:

```yaml
jobs:
  test:
    name: Test (PostgreSQL ${{ matrix.pg_version }})
    runs-on: blacksmith-16vcpu-ubuntu-2204
    strategy:
      fail-fast: false
      matrix:
        pg_version: [14, 15, 16, 17, 18]
    steps:
      - uses: actions/checkout@v4
      - uses: ./.github/actions/setup-test
      - run: |
          mise run postgres:up --extra-args "--detach --wait"
        env:
          PG_VERSION: ${{ matrix.pg_version }}
      # ... rest of steps with PG_VERSION in env
```

- `fail-fast: false` ensures all versions complete even if one fails
- Version passed via `PG_VERSION` environment variable
- Job names include version for clear identification in GitHub UI

### Docker Compose Changes

Update `tests/docker-compose.yml`:

```yaml
services:
  postgres: &postgres
    image: postgres:${PG_VERSION:-17}
    pull_policy: always
    # ... rest unchanged

  postgres-tls:  # renamed from postgres-17-tls
    <<: *postgres
    image: postgres:${PG_VERSION:-17}
    container_name: postgres-tls
    environment:
      PGPORT: 5617
    volumes:
      - ./pg/pg_hba-tls.conf:/etc/postgresql/pg_hba.conf
      - ./tls/localhost-key.pem:/etc/postgresql/localhost-key.pem
      - ./tls/localhost.pem:/etc/postgresql/localhost.pem
      - ./pg/data-tls:/var/lib/postgresql/data  # simplified from data-17
```

- Both containers use `postgres:${PG_VERSION:-17}` - defaults to 17 for local dev
- Rename `postgres-17-tls` → `postgres-tls` (version-agnostic naming)
- Data directory `data-17` → `data-tls` (version-agnostic)

### Mise Configuration Updates

Update `mise.toml` task `postgres:up` default services:
- Change `postgres postgres-17-tls` → `postgres postgres-tls`

Update `tests/mise.tls.toml`:
- Update any `postgres-17-tls` references to `postgres-tls`

No explicit `PG_VERSION` passthrough needed - docker-compose reads from environment.

### CI Test Execution

Each matrix job runs the full integration suite against one PostgreSQL version:
- All jobs use same ports (5532, 5617, 6432) - no conflicts as each runs on separate runner
- Tests run against TLS-enabled PostgreSQL container
- Each version shows as separate check in GitHub PR

## Files to Modify

| File | Change |
|------|--------|
| `.github/workflows/test.yml` | Add matrix strategy with `pg_version: [14, 15, 16, 17, 18]` |
| `tests/docker-compose.yml` | Use `postgres:${PG_VERSION:-17}`, rename `postgres-17-tls` → `postgres-tls` |
| `mise.toml` | Update default services in `postgres:up` task |
| `tests/mise.tls.toml` | Update container name reference if present |

## Local Development

No changes to local development workflow. Defaults to PostgreSQL 17.

## Validation

1. Create branch with changes
2. Push to trigger CI matrix
3. Verify all 5 versions pass (especially v18 to confirm volume issue resolved)
4. If v18 fails, investigate the specific volume issue before merge

## Rollback

- Remove strategy block to revert to single version
- `PG_VERSION` defaults to 17, so removing the env var restores current behavior
