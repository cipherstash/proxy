FROM debian:bookworm-slim AS runtime

# Install TLS/SSL certs for https support, and PostgreSQL client (psql)
RUN apt update && apt install -y ca-certificates postgresql-client

# Copy binary
COPY cipherstash-proxy /usr/local/bin/cipherstash-proxy

# Copy EQL install scripts
COPY cipherstash-eql.sql /opt/cipherstash-eql.sql

ENV CS_CONFIG_FILE=/etc/cipherstash-proxy.toml
ENTRYPOINT ["cipherstash-proxy"]
