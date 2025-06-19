FROM ubuntu:25.10

# Install TLS/SSL certs for https support, PostgreSQL client (psql), and curl
# for retrieving the certificate bundle.
RUN apt update && apt install -y ca-certificates postgresql-client curl

# Copy binary
COPY cipherstash-proxy /usr/local/bin/cipherstash-proxy
# Copy entrypoint, for handling Proxy startup
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh

# Copy EQL install scripts
COPY cipherstash-encrypt.sql /opt/cipherstash-eql.sql
# Copy example schema
COPY docs/getting-started/schema-example.sql /opt/schema-example.sql

# Make the AWS global bundle available for use in the docker-entrypoint.sh script.
ENV CS_DATABASE__AWS_BUNDLE_PATH="./aws-rds-global-bundle.pem"
RUN curl -ks "https://truststore.pki.rds.amazonaws.com/global/global-bundle.pem" -o "$CS_DATABASE__AWS_BUNDLE_PATH"

ENTRYPOINT ["docker-entrypoint.sh"]
