# Go integration tests for CipherStash Proxy

## Running

To run the integration tests:

```bash
# start postgres
mise run postgres:up
mise run postgres:setup

# start proxy
mise run proxy:up

# run the tests
mise run test:integration:lang:golang
```

This will run the tests inside a Docker container, which is networked with the `proxy` and `postgres` containers.

## Developing

To change the tests:

- Edit the appropriate `*_test.go` file in this repo
- Run the tests by using the same commands from the [Running](#running) section above

Alternatively, to run the tests outside of the container:

``` bash
# Tell the tests where to find Proxy
export DATABASE_URL="postgresql://cipherstash:password@localhost:6432/cipherstash"

# Run the tests
go test -v ./...
```

This requires you to have [Go installed](https://go.dev/dl/), but gives you a much faster feedback loop than `docker build`.

The test suite uses subtests heavily, so you can be very specific about what tests to run:

```bash
# Tell the tests where to find Proxy
export DATABASE_URL="postgresql://cipherstash:p%40ssword@localhost:6432/cipherstash"

# Run tests for encrypted_int8 columns
go test -v ./... -run TestPgxEncryptedMapInts/encrypted_int8

# Run tests for ints, but only with the Postgres simple protocol
go test ./... -v -run TestPgxEncryptedMapInts/.*/simple_protocol
```
