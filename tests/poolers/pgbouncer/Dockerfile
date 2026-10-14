# PgBouncer for the connection pooler compatibility matrix (`mise run test:poolers`).
#
# The PgBouncer version is selected through the Alpine release, so the matrix
# pins exact PgBouncer versions without depending on a third-party image:
#
#   ALPINE_VERSION=3.18 -> PgBouncer 1.19.0 (before protocol-level prepared statement support)
#   ALPINE_VERSION=3.22 -> PgBouncer 1.24.0 (supports `max_prepared_statements`)
ARG ALPINE_VERSION=3.22
FROM alpine:${ALPINE_VERSION}

RUN apk add --no-cache pgbouncer \
  && (id pgbouncer >/dev/null 2>&1 || adduser -D -S pgbouncer)

COPY pgbouncer.ini /etc/pgbouncer/pgbouncer.ini.base
COPY userlist.txt /etc/pgbouncer/userlist.txt
COPY entrypoint.sh /usr/local/bin/pgbouncer-entrypoint.sh

RUN chmod 0755 /usr/local/bin/pgbouncer-entrypoint.sh \
  && chown -R pgbouncer /etc/pgbouncer

USER pgbouncer
EXPOSE 6432
ENTRYPOINT ["/usr/local/bin/pgbouncer-entrypoint.sh"]
