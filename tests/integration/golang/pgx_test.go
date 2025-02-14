package main

import (
	"context"
	"fmt"
	"math/rand"
	"os"
	"testing"
	"time"

	"github.com/jackc/pgx/v5"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

var modes = []pgx.QueryExecMode{
	pgx.QueryExecModeCacheStatement,
	pgx.QueryExecModeCacheDescribe,
	pgx.QueryExecModeDescribeExec,
	pgx.QueryExecModeExec,
	pgx.QueryExecModeSimpleProtocol,
}

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
	value := "hello, world"
	insertStmt := fmt.Sprintf(`INSERT INTO encrypted (id, %s) VALUES ($1, $2)`, column)
	selectStmt := fmt.Sprintf(`SELECT id, %s FROM encrypted WHERE id=$1`, column)

	for _, mode := range modes {
		id := rand.Int()
		t.Run(mode.String(), func(t *testing.T) {
			assert := assert.New(t)
			_, err := conn.Exec(ctx, insertStmt, mode, id, value)
			assert.NoError(err)

			var rid int
			var rv string
			err = conn.QueryRow(context.Background(), selectStmt, mode, id).Scan(&rid, &rv)
			assert.NoError(err)
			assert.Equal(id, rid)
			assert.Equal(value, rv)
		})
	}
}

func TestPgxEncryptedMapInts(t *testing.T) {
	require := require.New(t)
	conn := setupPgxConnection(require)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	columns := []string{"encrypted_int2", "encrypted_int4", "encrypted_int8"}
	value := 99

	for _, column := range columns {
		t.Run(column, func(t *testing.T) {
			insertStmt := fmt.Sprintf(`INSERT INTO encrypted (id, %s) VALUES ($1, $2)`, column)
			selectStmt := fmt.Sprintf(`SELECT id, %s FROM encrypted WHERE id=$1`, column)
			for _, mode := range modes {
				id := rand.Int()
				t.Run(mode.String(), func(t *testing.T) {
					assert := assert.New(t)
					_, err := conn.Exec(ctx, insertStmt, mode, id, value)
					assert.NoError(err)

					var rid int
					var rv int
					err = conn.QueryRow(context.Background(), selectStmt, mode, id).Scan(&rid, &rv)
					assert.NoError(err)
					assert.Equal(id, rid)
					assert.Equal(value, rv)
				})
			}
		})
	}
}

func TestPgxEncryptedMapFloat(t *testing.T) {
	require := require.New(t)
	conn := setupPgxConnection(require)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

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
			assert := assert.New(t)
			_, err := conn.Exec(ctx, insertStmt, mode, id, value)
			assert.NoError(err)

			var rid int
			var rv float64
			err = conn.QueryRow(context.Background(), selectStmt, mode, id).Scan(&rid, &rv)
			assert.NoError(err)
			assert.Equal(id, rid)
			assert.Equal(value, rv)
		})
	}
}

/*
func TestPgxEncryptedMapDate(t *testing.T) {
	require := require.New(t)
	conn := setupPgxConnection(require)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	column := "encrypted_date"
	dates := []time.Time{
		time.Date(1, 1, 1, 0, 0, 0, 0, time.UTC),
		time.Date(1000, 1, 1, 0, 0, 0, 0, time.UTC),
		time.Date(1600, 1, 1, 0, 0, 0, 0, time.UTC),
		time.Date(1700, 1, 1, 0, 0, 0, 0, time.UTC),
		time.Date(1800, 1, 1, 0, 0, 0, 0, time.UTC),
		time.Date(1900, 1, 1, 0, 0, 0, 0, time.UTC),
		time.Date(1990, 1, 1, 0, 0, 0, 0, time.UTC),
		time.Date(1999, 12, 31, 0, 0, 0, 0, time.UTC),
		time.Date(2000, 1, 1, 0, 0, 0, 0, time.UTC),
		time.Date(2001, 1, 2, 0, 0, 0, 0, time.UTC),
		time.Date(2004, 2, 29, 0, 0, 0, 0, time.UTC),
		time.Date(2013, 7, 4, 0, 0, 0, 0, time.UTC),
		time.Date(2013, 12, 25, 0, 0, 0, 0, time.UTC),
		time.Date(2029, 1, 1, 0, 0, 0, 0, time.UTC),
		time.Date(2081, 1, 1, 0, 0, 0, 0, time.UTC),
		time.Date(2096, 2, 29, 0, 0, 0, 0, time.UTC),
		time.Date(2550, 1, 1, 0, 0, 0, 0, time.UTC),
		time.Date(9999, 12, 31, 0, 0, 0, 0, time.UTC),
	}
	insertStmt := fmt.Sprintf(`INSERT INTO encrypted (id, %s) VALUES ($1, $2)`, column)
	selectStmt := fmt.Sprintf(`SELECT id, %s FROM encrypted WHERE id=$1`, column)

	for _, value := range dates {
		t.Run(value.String(), func(t *testing.T) {
			for _, mode := range modes {
				id := rand.Int()
				t.Run(mode.String(), func(t *testing.T) {
					assert := assert.New(t)
					_, err := conn.Exec(ctx, insertStmt, mode, id, value)
					assert.NoError(err)

					var rid int
					var rv time.Time
					err = conn.QueryRow(context.Background(), selectStmt, mode, id).Scan(&rid, &rv)
					assert.NoError(err)
					assert.Equal(id, rid)
					assert.Equal(value, rv)
				})
			}
		})
	}
}
*/
