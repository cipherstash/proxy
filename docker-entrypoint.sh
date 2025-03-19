#!/usr/bin/env bash
set -eu

DATABASE_URL="postgresql://${CS_DATABASE__USERNAME}:${CS_DATABASE__PASSWORD}@${CS_DATABASE__HOST}:${CS_DATABASE__PORT}/${CS_DATABASE__NAME}"

postgres_ready () {
  psql ${DATABASE_URL} -c "SELECT 1" > /dev/null 2>&1
}

wait_for_postgres_or_exit() {
  host=${CS_DATABASE__HOST}
  port=${CS_DATABASE__PORT}
  max_retries=20
  interval=0.5
  attempt=1
  echo "Testing presence of PostgreSQL at ${host}:${port} with a maximum of ${max_retries} retries"

  until postgres_ready
  do
    if [ $attempt -lt $max_retries ]; then
      echo "Waiting for ${host}:${port}"
      sleep $interval
      attempt=$(expr $attempt + 1)
    else
      echo "Unable to connect to ${host}:${port} after ${max_retries} attempts"
      exit 64
    fi
  done
  echo "Connected to ${host}:${port} after ${attempt} attempts"
}

: "${CS_DATABASE__AWS_BUNDLE_PATH:=./aws-rds-global-bundle.pem}"

# Optionally pull in the AWS RDS global certificate bundle. This is required
# to communicate with AWS RDS instances, if they are not configured to use some
# other certificates.
# (This assumes that ca-certificates is installed, for csplit and update-ca-certificates.)
case "${CS_DATABASE__INSTALL_AWS_RDS_CERT_BUNDLE:-}" in
  # Have a guess at some common yaml-et-al encoding failures:
  "") ;&
  "false") ;&
  "no") ;&
  "0")
    >&2 echo "Not installing AWS RDS certificate bundle."
    ;;

  # Okay, go ahead and install the bundle:
  *)
    set -x
    if [ ! -f "$CS_DATABASE__AWS_BUNDLE_PATH" ]; then
      >&2 echo "Unable to find AWS RDS certificate bundle at: $CS_DATABASE__AWS_BUNDLE_PATH"
      exit 1
    fi

    >&2 echo "Installing AWS RDS certificate bundle..."
    csplit --quiet --elide-empty-files --prefix /usr/local/share/ca-certificates/aws --suffix '.%d.crt' "$CS_DATABASE__AWS_BUNDLE_PATH" '/-----BEGIN CERTIFICATE-----/' '{*}'
    update-ca-certificates
    ;;
esac

# Optionally install EQL in the target database
case "${CS_DATABASE__INSTALL_EQL:-}" in
  "true") ;&
  "yes") ;&
  "1")
    >&2 echo "Installing EQL in target PostgreSQL database..."

    if [ ! -f "/opt/cipherstash-eql.sql" ]; then
      >&2 echo "error: unable to find EQL installer at: /opt/cipherstash-eql.sql"
      exit 1
    fi

    # Wait for postgres to become available
    wait_for_postgres_or_exit

    # Attempt to install EQL
    psql --file=/opt/cipherstash-eql.sql --quiet $DATABASE_URL > /dev/null 2>&1
    if [ $? != 0 ]; then
      >&2 echo "error: unable to install EQL in target PostgreSQL database!"
      exit 2
    fi
    >&2 echo "Successfully installed EQL in target PostgreSQL database."
    ;;
  *)
    >&2 echo "Not installing EQL in target PostgreSQL database."
    ;;
esac

# Optionally install example schema in the target database
case "${CS_DATABASE__INSTALL_EXAMPLE_SCHEMA:-}" in
  "true") ;&
  "yes") ;&
  "1")
    >&2 echo "Applying example schema in target PostgreSQL database..."

    SQL_FILENAME="/opt/schema-example.sql"

    if [ ! -f "${SQL_FILENAME}" ]; then
      >&2 echo "error: unable to find example schema at: ${SQL_FILENAME}"
      exit 1
    fi

    # Wait for postgres to become available
    wait_for_postgres_or_exit

    # Attempt to install EQL
    psql --file=${SQL_FILENAME} --quiet $DATABASE_URL > /dev/null 2>&1
    if [ $? != 0 ]; then
      >&2 echo "error: unable to apply example schema in target PostgreSQL database!"
      exit 2
    fi

    >&2 echo "Successfully applied example schema in target PostgreSQL database."
    >&2 echo "Example tables: users"
    ;;
  *)
    >&2 echo "Not installing example schema in target PostgreSQL database."
    ;;
esac

>&2 echo "Proxy container setup complete!"
>&2 echo "Running CipherStash Proxy..."

exec cipherstash-proxy "$@"
