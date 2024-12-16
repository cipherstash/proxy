#!/usr/bin/env bash
#MISE description="PSQL with TLS"

#!/bin/bash

# set -o xtrace

# sanity check direct connections
psql postgresql://cipherstash:password@localhost:5617/cipherstash <<-EOF
SELECT 1;
EOF

psql postgresql://cipherstash:password@localhost:5532/cipherstash <<-EOF
SELECT 1;
EOF

# Connect to the proxy
psql 'postgresql://cipherstash:password@localhost:6432/cipherstash' <<-EOF
SELECT 1;
EOF

# Connect to the proxy forcing TLS
psql 'postgresql://cipherstash:password@localhost:6432/cipherstash?sslmode=require' <<-EOF
SELECT 1;
EOF

# Connect without TLS
psql 'postgresql://cipherstash:password@localhost:6432/cipherstash?sslmode=disable' <<-EOF
SELECT 1;
EOF
if [ $? -eq 0 ]; then
    echo "PSQL should not be able to connect via TLS"
    exit 1
fi

# Attempt with an invalid password
psql postgresql://cipherstash:not-the-password@localhost:6432/cipherstash <<-EOF
SELECT 1;
EOF

if [ $? -eq 0 ]; then
    echo "PSQL connected with an invalid password"
    exit 1
fi

echo "----------------------------------"
echo "PSQL TLS connection tests complete"
echo "----------------------------------"