package main

import (
	"context"
	"encoding/json"
	"fmt"
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

	obj := map[string]interface{}{
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
				jsonBytes, err := json.Marshal(obj)
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

func TestSelectJsonbPathQueryFirstString(t *testing.T) {
	conn := setupPgxConnection(t)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	require := require.New(t)

	insertStmt := "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)"
	selectStmt := "SELECT jsonb_path_query_first(encrypted_jsonb, $1) FROM encrypted"
	selectTemplate := "SELECT jsonb_path_query_first(encrypted_jsonb, '%s') FROM encrypted"

	obj := map[string]interface{}{
		"string": "hello",
		"number": 42,
		"nested": map[string]interface{}{
			"number": 1815,
			"string": "world",
		},
		"array_string": []string{"hello", "world"},
		"array_number": []int{42, 84},
	}

	selector := "$.array_string[*]"

	for _, mode := range modes {
		id := rand.Int()
		t.Run(mode.String(), func(t *testing.T) {
			t.Run("insert", func(t *testing.T) {
				jsonBytes, err := json.Marshal(obj)
				require.NoError(err)

				_, err = conn.Exec(ctx, insertStmt, mode, id, string(jsonBytes))
				require.NoError(err)
			})

			t.Run("select", func(t *testing.T) {
				var fetchedBytes []byte
				err := conn.QueryRow(context.Background(), selectStmt, mode, selector).Scan(&fetchedBytes)
				require.NoError(err)

				var result string
				err = json.Unmarshal(fetchedBytes, &result)
				require.NoError(err)
				require.Equal("hello", result)

				err = conn.QueryRow(context.Background(), fmt.Sprintf(selectTemplate, selector), mode).Scan(&fetchedBytes)
				require.NoError(err)

				err = json.Unmarshal(fetchedBytes, &result)
				require.NoError(err)
				require.Equal("hello", result)
			})
		})
	}
}
