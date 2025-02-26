FROM ubuntu:latest

# Install TLS/SSL certs for https support, PostgreSQL client (psql), and curl
# for optional use by the cipherstash-proxy.sh entrypoint.
RUN apt update && apt install -y ca-certificates postgresql-client curl

# Copy binary
COPY cipherstash-proxy /usr/local/bin/cipherstash-proxy
COPY cipherstash-proxy.sh /usr/local/bin/cipherstash-proxy.sh

# Copy EQL install scripts
COPY cipherstash-eql.sql /opt/cipherstash-eql.sql

# Make the AWS global bundle available for use in the cipherstash-proxy.sh script.
ENV CS_DATABASE__AWS_BUNDLE_PATH="./aws-rds-global-bundle.pem"
RUN curl -ks "https://truststore.pki.rds.amazonaws.com/global/global-bundle.pem" -o "$CS_DATABASE__AWS_BUNDLE_PATH"

ENTRYPOINT ["cipherstash-proxy.sh"]
