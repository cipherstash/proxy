# PgBouncer for the connection pooler compatibility matrix (`mise run test:poolers`).
#
# PgBouncer is installed from the Alpine release's package repository, so the
# Alpine release selects the PgBouncer version. Alpine can update a package
# within a release, so the build fails unless `pgbouncer --version` reports
# exactly PGBOUNCER_VERSION:
#
#   ALPINE_VERSION=3.18 PGBOUNCER_VERSION=1.19.0 (before protocol-level prepared statement support)
#   ALPINE_VERSION=3.22 PGBOUNCER_VERSION=1.24.0 (supports `max_prepared_statements`)
ARG ALPINE_VERSION=3.22
FROM alpine:${ALPINE_VERSION}
ARG PGBOUNCER_VERSION

RUN apk add --no-cache pgbouncer \
  && (id pgbouncer >/dev/null 2>&1 || adduser -D -S pgbouncer) \
  && installed="$(pgbouncer --version | head -n 1)" \
  && if [ "${installed}" != "PgBouncer ${PGBOUNCER_VERSION:?PGBOUNCER_VERSION build arg is required}" ]; then \
       echo "Expected PgBouncer ${PGBOUNCER_VERSION}, installed: ${installed}" >&2; exit 1; \
     fi

COPY pgbouncer.ini /etc/pgbouncer/pgbouncer.ini.base
COPY userlist.txt /etc/pgbouncer/userlist.txt
COPY entrypoint.sh /usr/local/bin/pgbouncer-entrypoint.sh

RUN chmod 0755 /usr/local/bin/pgbouncer-entrypoint.sh \
  && chown -R pgbouncer /etc/pgbouncer

USER pgbouncer
EXPOSE 6432
ENTRYPOINT ["/usr/local/bin/pgbouncer-entrypoint.sh"]
