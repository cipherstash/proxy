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
	selector := map[string]interface{}{
		"string": "hello",
	}
	selectJsonbContains(t, selector, true)

	selector = map[string]interface{}{
		"string": "blah",
	}
	selectJsonbContains(t, selector, false)
}

func TestSelectJsonbContainsWithNumber(t *testing.T) {
	selector := map[string]interface{}{
		"number": 42,
	}
	selectJsonbContains(t, selector, true)

	selector = map[string]interface{}{
		"number": 11,
	}
	selectJsonbContains(t, selector, false)
}

func TestSelectJsonbContainsWithNumericArray(t *testing.T) {
	selector := map[string]interface{}{
		"array_number": []int{42, 84},
	}
	selectJsonbContains(t, selector, true)

	selector = map[string]interface{}{
		"array_number": []int{1, 2},
	}
	selectJsonbContains(t, selector, false)
}

func TestSelectJsonbContainsWithStringArray(t *testing.T) {
	selector := map[string]interface{}{
		"array_string": []string{"hello", "world"},
	}
	selectJsonbContains(t, selector, true)

	selector = map[string]interface{}{
		"array_string": []string{"blah", "vtha"},
	}
	selectJsonbContains(t, selector, false)
}

func TestSelectJsonbContainsWithNestedObject(t *testing.T) {
	selector := map[string]interface{}{
		"nested": map[string]interface{}{
			"number": 1815,
			"string": "world",
		},
	}
	selectJsonbContains(t, selector, true)

	selector = map[string]interface{}{
		"nested": map[string]interface{}{
			"number": 1914,
			"string": "world",
		},
	}
	selectJsonbContains(t, selector, false)
}

func selectJsonbContains(t *testing.T, selector map[string]interface{}, expected bool) {
	conn := setupPgxConnection(t)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	tx, err := conn.Begin(ctx)
	require.NoError(t, err)
	defer tx.Rollback(ctx)

	require := require.New(t)

	insertStmt := "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)"
	selectStmt := "SELECT encrypted_jsonb @> $1 FROM encrypted LIMIT 1"
	selectTemplate := "SELECT encrypted_jsonb @> '%s' FROM encrypted"

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
				jsonBytes, err := json.Marshal(selector)
				require.NoError(err)
				jsonStr := string(jsonBytes)

				var rv bool
				err = conn.QueryRow(context.Background(), selectStmt, mode, jsonStr).Scan(&rv)
				require.NoError(err)
				require.Equal(expected, rv)

				err = conn.QueryRow(context.Background(), fmt.Sprintf(selectTemplate, jsonStr), mode).Scan(&rv)
				require.NoError(err)

				require.Equal(expected, rv)
			})
		})
	}
}

// expected is a pointer to express that if nil, the returned json should be empty and
// cannot be unmarshalled
func selectJsonbPathQueryFirst(t *testing.T, selector string, expected *interface{}) {
	conn := setupPgxConnection(t)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	tx, err := conn.Begin(ctx)
	require.NoError(t, err)
	defer tx.Rollback(ctx)

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

				if expected == nil {
					require.Equal(0, len(fetchedBytes))
				} else {
					var result interface{}
					err = json.Unmarshal(fetchedBytes, &result)
					require.NoError(err)
					require.Equal(*expected, result)
				}

				err = conn.QueryRow(context.Background(), fmt.Sprintf(selectTemplate, selector), mode).Scan(&fetchedBytes)
				require.NoError(err)

				if expected == nil {
					require.Equal(0, len(fetchedBytes))
				} else {
					var result interface{}
					err = json.Unmarshal(fetchedBytes, &result)
					require.NoError(err)
					require.Equal(*expected, result)
				}
			})
		})
	}
}

func TestSelectJsonbPathQueryFirstString(t *testing.T) {
	var expected interface{} = "hello"
	selectJsonbPathQueryFirst(t, "$.array_string[*]", &expected)
}

func TestSelectJsonbPathQueryFirstNumber(t *testing.T) {
	var expected interface{} = 42.0
	selectJsonbPathQueryFirst(t, "$.array_number[*]", &expected)
}

func TestSelectJsonbPathQueryFirstWithUnknown(t *testing.T) {
	selectJsonbPathQueryFirst(t, "$.vtha", nil)
}
