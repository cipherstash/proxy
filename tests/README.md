



## TLS


### Configuration

Turns ssl on and enforces `md5` password hashing.

- pg/tls-postgresql.conf
- pg/md5-pg_hba.conf
- tls/localhost-key.pem
- tls/localhost-key.pem


## Commands

Connect to pg 17 over TLS
```
docker compose up --build
psql postgresql://mlp:mlp@host.docker.internal:5517/mlp
psql postgresql://mlp:mlp@192.168.65.0:5517/mlp
psql postgresql://mlp:mlp@localhost:5517/mlp
```



