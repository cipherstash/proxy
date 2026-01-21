window.BENCHMARK_DATA = {
  "lastUpdate": 1768971950487,
  "repoUrl": "https://github.com/cipherstash/proxy",
  "entries": {
    "k6 Latency": [
      {
        "commit": {
          "author": {
            "name": "cipherstash",
            "username": "cipherstash"
          },
          "committer": {
            "name": "cipherstash",
            "username": "cipherstash"
          },
          "id": "e81a59d25b7bce6f98348091223c5e46dea956c5",
          "message": "feat(benchmark): add k6 benchmarks for JSONB/encrypted query performance",
          "timestamp": "2026-01-20T03:20:49Z",
          "url": "https://github.com/cipherstash/proxy/pull/352/commits/e81a59d25b7bce6f98348091223c5e46dea956c5"
        },
        "date": 1768971949896,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "jsonb-large-payload_p95",
            "value": 139.34,
            "unit": "ms"
          },
          {
            "name": "jsonb-large-payload_p99",
            "value": 155.3,
            "unit": "ms"
          },
          {
            "name": "jsonb-ste-vec-insert_p95",
            "value": 127.45,
            "unit": "ms"
          },
          {
            "name": "jsonb-ste-vec-insert_p99",
            "value": 140.72,
            "unit": "ms"
          }
        ]
      }
    ]
  }
}