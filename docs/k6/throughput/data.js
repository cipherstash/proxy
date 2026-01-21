window.BENCHMARK_DATA = {
  "lastUpdate": 1768971949118,
  "repoUrl": "https://github.com/cipherstash/proxy",
  "entries": {
    "k6 Throughput": [
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
        "date": 1768971944183,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "jsonb-large-payload_rate",
            "value": 86.23,
            "unit": "iter/s"
          },
          {
            "name": "jsonb-ste-vec-insert_rate",
            "value": 92.2,
            "unit": "iter/s"
          }
        ]
      }
    ]
  }
}