package main

import (
	"context"
	"fmt"
	"math/rand"
	"os"
	"testing"
	"time"

	"github.com/jackc/pgx/v5"
	// "github.com/jackc/pgx/v5/pgtype"
	"github.com/stretchr/testify/require"
)

var modes = []pgx.QueryExecMode{
	pgx.QueryExecModeCacheStatement,
	pgx.QueryExecModeCacheDescribe,
	pgx.QueryExecModeDescribeExec,
	pgx.QueryExecModeExec,
	pgx.QueryExecModeSimpleProtocol,
}

func setupPgxConnection(t *testing.T) *pgx.Conn {
	require := require.New(t)
	dbURL := os.Getenv("DATABASE_URL")
	require.NotEmpty(dbURL, "DATABASE_URL environment variable not set")

	conn, err := pgx.Connect(context.Background(), dbURL)
	require.NoError(err, "unable to connect to the database")
	return conn
}

func TestPgxConnect(t *testing.T) {
	t.Parallel()
	conn := setupPgxConnection(t)
	require := require.New(t)

	var result int
	err := conn.QueryRow(context.Background(), "select 1").Scan(&result)
	require.NoError(err)
	require.Equal(1, result)
}

func TestPgxUnencryptedInsertAndSelect(t *testing.T) {
	t.Parallel()
	conn := setupPgxConnection(t)
	require := require.New(t)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	tx, err := conn.Begin(ctx)
	require.NoError(err)
	defer tx.Rollback(ctx)

	_, err = tx.Exec(ctx, `
CREATE TEMPORARY TABLE t (name text not null unique);

INSERT INTO t (name) VALUES
	('Ada'),
	('Grace'),
	('Susan');
`)
	require.NoError(err)

	var result string
	err = tx.QueryRow(context.Background(), "SELECT name FROM t").Scan(&result)
	require.NoError(err)
	require.Equal("Ada", result)
}

func TestPgxEncryptedMapText(t *testing.T) {
	t.Parallel()
	conn := setupPgxConnection(t)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	tx, err := conn.Begin(ctx)
	require.NoError(t, err)
	defer tx.Rollback(ctx)

	column := "encrypted_text"
	value := "hello, world"
	insertStmt := fmt.Sprintf(`INSERT INTO encrypted (id, %s) VALUES ($1, $2)`, column)
	selectStmt := fmt.Sprintf(`SELECT id, %s FROM encrypted WHERE id=$1`, column)

	for _, mode := range modes {
		id := rand.Int()
		t.Run(mode.String(), func(t *testing.T) {
			t.Run("insert", func(t *testing.T) {
				_, err := tx.Exec(ctx, insertStmt, mode, id, value)
				require.NoError(t, err)
			})

			t.Run("select", func(t *testing.T) {
				var rid int
				var rv string
				err := tx.QueryRow(context.Background(), selectStmt, mode, id).Scan(&rid, &rv)
				require.NoError(t, err)
				require.Equal(t, id, rid)
				require.Equal(t, value, rv)
			})
		})
	}
}

func TestPgxEncryptedMapInts(t *testing.T) {
	t.Parallel()
	conn := setupPgxConnection(t)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	tx, err := conn.Begin(ctx)
	require.NoError(t, err)
	defer tx.Rollback(ctx)

	columns := []string{"encrypted_int2", "encrypted_int4", "encrypted_int8"}
	value := 99

	for _, column := range columns {
		t.Run(column, func(t *testing.T) {
			insertStmt := fmt.Sprintf(`INSERT INTO encrypted (id, %s) VALUES ($1, $2)`, column)
			selectStmt := fmt.Sprintf(`SELECT id, %s FROM encrypted WHERE id=$1`, column)
			for _, mode := range modes {
				id := rand.Int()
				t.Run(mode.String(), func(t *testing.T) {
					t.Run("insert", func(t *testing.T) {
						_, err := tx.Exec(ctx, insertStmt, mode, id, value)
						require.NoError(t, err)
					})

					t.Run("select", func(t *testing.T) {
						var rid int
						var rv int
						err := tx.QueryRow(context.Background(), selectStmt, mode, id).Scan(&rid, &rv)
						require.NoError(t, err)
						require.Equal(t, id, rid)
						require.Equal(t, value, rv)
					})
				})
			}
		})
	}
}

func TestPgxEncryptedMapFloat(t *testing.T) {
	t.Parallel()
	conn := setupPgxConnection(t)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	tx, err := conn.Begin(ctx)
	require.NoError(t, err)
	defer tx.Rollback(ctx)

	column := "encrypted_float8"
	value := 1.5
	insertStmt := fmt.Sprintf(`INSERT INTO encrypted (id, %s) VALUES ($1, $2)`, column)
	selectStmt := fmt.Sprintf(`SELECT id, %s FROM encrypted WHERE id=$1`, column)

	for _, mode := range modes {
		id := rand.Int()
		// Skip undefined behaviour of floats in simple protocol
		if mode == pgx.QueryExecModeExec || mode == pgx.QueryExecModeSimpleProtocol {
			continue
		}
		t.Run(mode.String(), func(t *testing.T) {
			t.Run("insert", func(t *testing.T) {
				_, err := tx.Exec(ctx, insertStmt, mode, id, value)
				require.NoError(t, err)
			})

			t.Run("select", func(t *testing.T) {
				var rid int
				var rv float64
				err := tx.QueryRow(context.Background(), selectStmt, mode, id).Scan(&rid, &rv)
				require.NoError(t, err)
				require.Equal(t, id, rid)
				require.Equal(t, value, rv)
			})
		})
	}
}

func TestPgxInsertEncryptedWithDomainTypeAndReturning(t *testing.T) {
	t.Parallel()
	conn := setupPgxConnection(t)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	dtypes, err := conn.LoadTypes(context.Background(), []string{"domain_type_with_check"})
	require.NoError(t, err)
	conn.TypeMap().RegisterTypes(dtypes)

	tx, err := conn.Begin(ctx)
	require.NoError(t, err)
	defer tx.Rollback(ctx)

	encrypted_column_value := "hello, world"
	plaintext_domain_value := "BV"

	insertStmt := fmt.Sprintf(`INSERT INTO encrypted (id, encrypted_text, plaintext_domain) VALUES ($1, $2, $3) RETURNING id, encrypted_text, plaintext_domain`)

	for _, mode := range modes {
		id := rand.Int()

		t.Run(mode.String(), func(t *testing.T) {
			t.Run("insert", func(t *testing.T) {
				var rid int
				var rev string
				var rdv string
				err := conn.QueryRow(context.Background(), insertStmt, mode, id, encrypted_column_value, plaintext_domain_value).Scan(&rid, &rev, &rdv)
				require.NoError(t, err)
				require.Equal(t, id, rid)
				require.Equal(t, encrypted_column_value, rev)
				require.Equal(t, plaintext_domain_value, rdv)
			})

		})
	}
}

