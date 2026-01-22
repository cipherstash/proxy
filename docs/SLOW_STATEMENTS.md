# Slow Statement Logging

CipherStash Proxy includes built-in slow statement logging for troubleshooting performance issues.

## Configuration

Enable slow statement logging via environment variables:

```bash
# Enable slow statement logging (required)
CS_LOG__SLOW_STATEMENTS=true

# Optional: Set minimum duration threshold (default: 2000ms)
CS_LOG__SLOW_STATEMENT_MIN_DURATION_MS=500

# Optional: Set log level (default: warn when enabled)
CS_LOG__SLOW_STATEMENTS_LEVEL=warn

# Recommended: Use structured logging for parsing
CS_LOG__FORMAT=structured
```

## Slow Statement Logs

When a statement exceeds the threshold, the proxy logs a detailed breakdown:

```json
{
  "client_id": 1,
  "duration_ms": 10500,
  "statement_type": "INSERT",
  "protocol": "extended",
  "encrypted": true,
  "encrypted_values_count": 3,
  "param_bytes": 1024,
  "query_fingerprint": "a1b2c3d4",
  "keyset_id": "uuid",
  "mapping_disabled": false,
  "breakdown": {
    "parse_ms": 5,
    "encrypt_ms": 450,
    "server_write_ms": 12,
    "server_wait_ms": 9800,
    "server_response_ms": 233
  }
}
```

## Prometheus Metrics

### Labeled Histograms

Duration histograms now include labels for filtering:

- `statement_type`: insert, update, delete, select, other
- `protocol`: simple, extended
- `mapped`: true, false
- `multi_statement`: true, false

Example queries:
```promql
# Average INSERT duration
histogram_quantile(0.5, rate(cipherstash_proxy_statements_session_duration_seconds_bucket{statement_type="insert"}[5m]))

# Compare encrypted vs passthrough
histogram_quantile(0.99, rate(cipherstash_proxy_statements_session_duration_seconds_bucket{mapped="true"}[5m]))
```

### ZeroKMS Cipher Init

```
cipherstash_proxy_keyset_cipher_init_duration_seconds
```

Measures time for cipher initialization including ZeroKMS network call. High values indicate ZeroKMS connectivity issues.

## Interpreting Results

| Symptom | Likely Cause |
|---------|--------------|
| High `encrypt_ms` | ZeroKMS latency or large payload |
| High `server_wait_ms` | Database latency |
| High `cipher_init_duration` | ZeroKMS cold start or network |
| High `parse_ms` | Complex SQL or schema lookup |
