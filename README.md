# CipherStash Proxy


Experiments in Minimally Viable Proxying.











## Tests

Assumes a database called `my_little_proxy` and a `blah` table.


```sql
CREATE TABLE blah (
    id bigint GENERATED ALWAYS AS IDENTITY,
    t TEXT,
    j JSONB,
    vtha JSONB,
    PRIMARY KEY(id)
);
```

Run the proxy

```bash
cargo run
```


Run the tests (there aren't many yet)

```bash
cargo test -- --nocapture
```





Assuming
`mise use postgres`

```
pg_ctl start
createdb my-little-proxy -U postgres

CREATE DATABASE mlp;
CREATE USER mlp WITH ENCRYPTED PASSWORD 'password';
GRANT ALL PRIVILEGES ON DATABASE mlp TO mlp;

-- REVOKE ALL PRIVILEGES ON DATABASE mlp FROM mlp;


psql postgresql://mlp:password@127.0.0.1:5432/mlp



-- check SSL in use
SELECT datname,usename, ssl, client_addr
  FROM pg_stat_ssl
  JOIN pg_stat_activity
    ON pg_stat_ssl.pid = pg_stat_activity.pid;

```


