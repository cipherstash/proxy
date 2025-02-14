package main

import (
	"context"
	"fmt"
	"math/rand"
	"os"
	"testing"
	"time"

	"github.com/jackc/pgx/v5"
	"github.com/stretchr/testify/require"
)

func setupPgxConnection(require *require.Assertions) *pgx.Conn {
	dbURL := os.Getenv("DATABASE_URL")
	require.NotEmpty(dbURL, "DATABASE_URL environment variable not set")

	conn, err := pgx.Connect(context.Background(), dbURL)
	require.NoError(err, "unable to connect to the database")
	return conn
}

func TestPgxConnect(t *testing.T) {
	require := require.New(t)
	conn := setupPgxConnection(require)

	var result int
	err := conn.QueryRow(context.Background(), "select 1").Scan(&result)
	require.NoError(err)
	require.Equal(1, result)
}

func TestPgxUnencryptedInsertAndSelect(t *testing.T) {
	require := require.New(t)
	conn := setupPgxConnection(require)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	_, err := conn.Exec(ctx, `
CREATE TEMPORARY TABLE t (name text not null unique);

INSERT INTO t (name) VALUES
	('Ada'),
	('Grace'),
	('Susan');
`)
	require.NoError(err)

	var result string
	err = conn.QueryRow(context.Background(), "SELECT name FROM t").Scan(&result)
	require.NoError(err)
	require.Equal("Ada", result)
}

func TestPgxEncryptedMapText(t *testing.T) {
	require := require.New(t)
	conn := setupPgxConnection(require)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	column := "encrypted_text"
	id := rand.Int()
	value := "hello, world"
	sql := fmt.Sprintf(`INSERT INTO encrypted (id, %s) VALUES ($1, $2)`, column)
	_, err := conn.Exec(ctx, sql, id, value)
	require.NoError(err)
}
