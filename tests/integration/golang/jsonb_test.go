package main

import (
	"context"
	"encoding/json"
	"math/rand"
	"testing"
	"time"

	"github.com/stretchr/testify/require"
)

// TODO: trivial test. Delete it later
func TestTrivial(t *testing.T) {
	require := require.New(t)
	require.Equal(0, 0)
}

func TestSelectJsonbContainsWithString(t *testing.T) {
	conn := setupPgxConnection(t)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	require := require.New(t)

	insertStmt := "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)"
	selectStmt := "SELECT encrypted_jsonb @> $1 FROM encrypted LIMIT 1"

	insert_jsonb := map[string]interface{}{
		"string": "hello",
		"number": 42,
		"nested": map[string]interface{}{
			"number": 1815,
			"string": "world",
		},
		"array_string": []string{"hello", "world"},
		"array_number": []int{42, 84},
	}

	select_jsonb := map[string]interface{}{
		"string": "hello",
	}

	for _, mode := range modes {
		id := rand.Int()
		t.Run(mode.String(), func(t *testing.T) {
			t.Run("insert", func(t *testing.T) {
				jsonBytes, err := json.Marshal(insert_jsonb)
				require.NoError(err)
				jsonStr := string(jsonBytes)

				_, err = conn.Exec(ctx, insertStmt, mode, id, jsonStr)
				require.NoError(err)
			})

			t.Run("select", func(t *testing.T) {
				jsonBytes, err := json.Marshal(select_jsonb)
				require.NoError(err)
				jsonStr := string(jsonBytes)

				var rv bool
				err = conn.QueryRow(context.Background(), selectStmt, mode, jsonStr).Scan(&rv)
				require.NoError(err)
				require.True(rv)
			})
		})
	}
}
