package main

import (
	"context"
	"os"
	"testing"

	"github.com/jackc/pgx/v5"
	"github.com/stretchr/testify/require"
)

func TestPgxConnect(t *testing.T) {
	require := require.New(t)
	dbURL := os.Getenv("DATABASE_URL")
	require.NotEmpty(dbURL)

	conn, err := pgx.Connect(context.Background(), dbURL)
	require.NoError(err)

	var result int
	err = conn.QueryRow(context.Background(), "select 1").Scan(&result)
	require.NoError(err)
	require.Equal(1, result)
}
