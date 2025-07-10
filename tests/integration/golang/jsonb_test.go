package main

import (
	"context"
	"database/sql"
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

func testData() map[string]interface{} {
	return map[string]interface{}{
		"string": "hello",
		"number": 42,
		"nested": map[string]interface{}{
			"number": 1815,
			"string": "world",
		},
		"array_string": []string{"hello", "world"},
		"array_number": []int{42, 84},
	}
}

func selectJsonbContains(t *testing.T, selector map[string]interface{}, expected bool) {
	selectJsonbContainment(t, selector,
		"SELECT encrypted_jsonb @> $1 FROM encrypted LIMIT 1",
		"SELECT encrypted_jsonb @> '%s' FROM encrypted LIMIT 1",
		expected)
}

func selectJsonbContainment(t *testing.T, selector map[string]interface{}, selectStmt string, selectTemplate string, expected bool) {
	conn := setupPgxConnection(t)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	tx, err := conn.Begin(ctx)
	require.NoError(t, err)
	defer tx.Rollback(ctx)

	require := require.New(t)

	insertStmt := "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)"

	for _, mode := range modes {
		id := rand.Int()
		t.Run(mode.String(), func(t *testing.T) {
			t.Run("insert", func(t *testing.T) {
				jsonBytes, err := json.Marshal(testData())
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

func TestJsonbContainedByWithString(t *testing.T) {
	selector := map[string]interface{}{
		"string": "hello",
	}
	selectJsonbContains(t, selector, true)

	selector = map[string]interface{}{
		"string": "blah",
	}
	selectContainedByJsonb(t, selector, false)
}

func TestJsonbContainedByWithNumber(t *testing.T) {
	selector := map[string]interface{}{
		"number": 42,
	}
	selectJsonbContains(t, selector, true)

	selector = map[string]interface{}{
		"number": 11,
	}
	selectContainedByJsonb(t, selector, false)
}

func TestJsonbContainedByWithNumericArray(t *testing.T) {
	selector := map[string]interface{}{
		"array_number": []int{42, 84},
	}
	selectJsonbContains(t, selector, true)

	selector = map[string]interface{}{
		"array_number": []int{1, 2},
	}
	selectJsonbContains(t, selector, false)
}

func TestJsonbContainedByWithStringArray(t *testing.T) {
	selector := map[string]interface{}{
		"array_string": []string{"hello", "world"},
	}
	selectJsonbContains(t, selector, true)

	selector = map[string]interface{}{
		"array_string": []string{"blah", "vtha"},
	}
	selectJsonbContains(t, selector, false)
}

func TestJsonbContainedByWithNestedObject(t *testing.T) {
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

func selectContainedByJsonb(t *testing.T, selector map[string]interface{}, expected bool) {
	selectJsonbContainment(t, selector,
		"SELECT $1 <@ encrypted_jsonb FROM encrypted LIMIT 1",
		"SELECT '%s' <@ encrypted_jsonb FROM encrypted LIMIT 1",
		expected)
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

	for _, mode := range modes {
		id := rand.Int()
		t.Run(mode.String(), func(t *testing.T) {
			t.Run("insert", func(t *testing.T) {
				jsonBytes, err := json.Marshal(testData())
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

func selectJsonbPathQueryStmt() string {
	return "SELECT jsonb_path_query(encrypted_jsonb, $1) FROM encrypted"
}

func selectJsonbPathQueryTemplate() string {
	return "SELECT jsonb_path_query(encrypted_jsonb, '%s') FROM encrypted"
}

func selectJsonb(t *testing.T, selector string, selectStmt string, selectTemplate string, expectedResult ExpectedResult) {
	conn := setupPgxConnection(t)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	tx, err := conn.Begin(ctx)
	require.NoError(t, err)
	defer tx.Rollback(ctx)

	require := require.New(t)

	insertStmt := "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)"

	for _, mode := range modes {
		id := rand.Int()
		t.Run(mode.String(), func(t *testing.T) {
			t.Run("insert", func(t *testing.T) {
				jsonBytes, err := json.Marshal(testData())
				require.NoError(err)

				_, err = conn.Exec(ctx, insertStmt, mode, id, string(jsonBytes))
				require.NoError(err)
			})

			t.Run("select", func(t *testing.T) {
				var fetchedBytes []byte
				err := conn.QueryRow(context.Background(), selectStmt, mode, selector).Scan(&fetchedBytes)
				if expectedResult.Type == ExpectedNoResult {
					require.ErrorIs(err, sql.ErrNoRows)
				} else if expectedResult.Type == ExpectedNativeBool {
					var result bool
					err = conn.QueryRow(context.Background(), selectStmt, mode, selector).Scan(&result)
					require.NoError(err)
					require.Equal(expectedResult.Value, result)

					err = conn.QueryRow(context.Background(), fmt.Sprintf(selectTemplate, selector), mode).Scan(&result)
					require.NoError(err)
					require.Equal(expectedResult.Value, result)
				} else {
					require.NoError(err)
					if expectedResult.Type == ExpectedEmpty {
						require.Equal(0, len(fetchedBytes))
					} else {
						var result interface{}
						err = json.Unmarshal(fetchedBytes, &result)
						require.NoError(err)
						require.Equal(expectedResult.Value, result)
					}

					err = conn.QueryRow(context.Background(), fmt.Sprintf(selectTemplate, selector), mode).Scan(&fetchedBytes)
					require.NoError(err)

					if expectedResult.Type == ExpectedEmpty {
						require.Equal(0, len(fetchedBytes))
					} else {
						var result interface{}
						err = json.Unmarshal(fetchedBytes, &result)
						require.NoError(err)
						require.Equal(expectedResult.Value, result)
					}
				}
			})
		})
	}
}

func TestSelectJsonbPathQueryNumber(t *testing.T) {
	expected := ExpectedResult{
		Type:  ExpectedJsonValue,
		Value: 42.0,
	}
	selectJsonb(t, "$.number", selectJsonbPathQueryStmt(), selectJsonbPathQueryTemplate(), expected)
}

func TestSelectJsonbPathQueryString(t *testing.T) {
	expected := ExpectedResult{
		Type:  ExpectedJsonValue,
		Value: "world",
	}
	selectJsonb(t, "$.nested.string", selectJsonbPathQueryStmt(), selectJsonbPathQueryTemplate(), expected)
}

func TestSelectJsonbPathQueryValue(t *testing.T) {
	expected := ExpectedResult{
		Type: ExpectedJsonValue,
		Value: map[string]interface{}{
			"number": 1815.0,
			"string": "world",
		},
	}
	selectJsonb(t, "$.nested", selectJsonbPathQueryStmt(), selectJsonbPathQueryTemplate(), expected)
}

func TestSelectJsonbPathQueryWithUnknown(t *testing.T) {
	expected := ExpectedResult{
		Type: ExpectedNoResult,
	}
	selectJsonb(t, "$.vtha", selectJsonbPathQueryStmt(), selectJsonbPathQueryTemplate(), expected)
}

func TestSelectJsonbPathQueryWithAlias(t *testing.T) {
	expected := ExpectedResult{
		Type: ExpectedJsonValue,
		Value: map[string]interface{}{
			"number": 1815.0,
			"string": "world",
		},
	}
	selectJsonb(t, "$.nested", "SELECT jsonb_path_query(encrypted_jsonb, $1) as selected FROM encrypted", "SELECT jsonb_path_query(encrypted_jsonb, '%s') as selected FROM encrypted", expected)
}

func selectJsonPathExistsQueryStmt() string {
	return "SELECT jsonb_path_exists(encrypted_jsonb, $1) FROM encrypted"
}

func selectJsonPathExistsQueryTemplate() string {
	return "SELECT jsonb_path_exists(encrypted_jsonb, '%s') FROM encrypted"
}

func TestSelectJsonbPathExistsNumber(t *testing.T) {
	expected := ExpectedResult{
		Type:  ExpectedNativeBool,
		Value: true,
	}
	selectJsonb(t, "$.number", selectJsonPathExistsQueryStmt(), selectJsonPathExistsQueryTemplate(), expected)
}

func TestSelectJsonbPathExistsString(t *testing.T) {
	expected := ExpectedResult{
		Type:  ExpectedNativeBool,
		Value: true,
	}
	selectJsonb(t, "$.nested.string", selectJsonPathExistsQueryStmt(), selectJsonPathExistsQueryTemplate(), expected)
}

func TestSelectJsonbPathExistsValue(t *testing.T) {
	expected := ExpectedResult{
		Type:  ExpectedNativeBool,
		Value: true,
	}
	selectJsonb(t, "$.nested", selectJsonPathExistsQueryStmt(), selectJsonPathExistsQueryTemplate(), expected)
}

func TestSelectJsonbPathExistsWithUnknownSelector(t *testing.T) {
	expected := ExpectedResult{
		Type:  ExpectedNativeBool,
		Value: false,
	}
	selectJsonb(t, "$.vtha", selectJsonPathExistsQueryStmt(), selectJsonPathExistsQueryTemplate(), expected)
}

func TestSelectJsonbPathExistsWithAlias(t *testing.T) {
	expected := ExpectedResult{
		Type:  ExpectedNativeBool,
		Value: true,
	}
	selectJsonb(t, "$.nested", "SELECT jsonb_path_exists(encrypted_jsonb, $1) as selected FROM encrypted", "SELECT jsonb_path_exists(encrypted_jsonb, '%s') as selected FROM encrypted", expected)
}

// Sum type does not exist natively in golang. This seems like a common pattern to use instead
type ExpectedResultType int

const (
	ExpectedEmpty ExpectedResultType = iota
	ExpectedJsonValue
	ExpectedNoResult
	ExpectedNativeBool
)

type ExpectedResult struct {
	Type  ExpectedResultType
	Value any
}
