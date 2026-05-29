-- Schema for the passthrough memory reproduction.
--
-- This deliberately uses PLAINTEXT columns only (jsonb, not eql_v2_encrypted)
-- so the proxy runs in the customer's reported configuration: passthrough,
-- no encrypted columns. Note that unless CS_DEVELOPMENT__DISABLE_MAPPING=true,
-- the proxy still fully parses and type-checks every statement against this
-- schema before discarding the result -- which is the allocation churn we are
-- measuring.

CREATE TABLE IF NOT EXISTS credit_data_order_v2 (
    id              uuid PRIMARY KEY,
    organization_id uuid NOT NULL,
    order_id        uuid NOT NULL,
    account_review  boolean NOT NULL DEFAULT false,
    full_report     jsonb NOT NULL,
    raw_report      jsonb NOT NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now()
);
