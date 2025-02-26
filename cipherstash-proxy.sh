#!/usr/bin/env bash
set -eu

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
    echo "Not installing AWS RDS certificate bundle."
    ;;

  # Okay, go ahead and install the bundle:
  *)
    set -x
    if [ ! -f "$CS_DATABASE__AWS_BUNDLE_PATH" ]; then
      echo "Unable to find AWS RDS certificate bundle at: $CS_DATABASE__AWS_BUNDLE_PATH"
      exit 1
    fi

    echo "Installing AWS RDS certificate bundle..."
    csplit --quiet --elide-empty-files --prefix /usr/local/share/ca-certificates/aws --suffix '.%d.crt' "$CS_DATABASE__AWS_BUNDLE_PATH" '/-----BEGIN CERTIFICATE-----/' '{*}'
    update-ca-certificates
    ;;
esac

exec cipherstash-proxy
