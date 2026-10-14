#!/bin/sh
# Renders the PgBouncer configuration for this matrix instance and starts PgBouncer.
#
# PGBOUNCER_MAX_PREPARED_STATEMENTS, when set, adds `max_prepared_statements` to
# the [pgbouncer] section. Leave it unset for PgBouncer < 1.21, which does not
# know the setting.
set -eu

config=/tmp/pgbouncer.ini
cp /etc/pgbouncer/pgbouncer.ini.base "${config}"

if [ -n "${PGBOUNCER_MAX_PREPARED_STATEMENTS:-}" ]; then
  printf '\nmax_prepared_statements = %s\n' "${PGBOUNCER_MAX_PREPARED_STATEMENTS}" >> "${config}"
fi

pgbouncer --version
exec pgbouncer "${config}"
