package main

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"math/rand"
	"os"
	"testing"
	"time"

	"github.com/jackc/pgx/v5"
	"github.com/stretchr/testify/require"
)

func TestMain(m *testing.M) {
	dbURL := os.Getenv("DATABASE_URL")
	if dbURL == "" {
		fmt.Println("DATABASE_URL environment variable not set")
		os.Exit(1)
	}

	conn, err := pgx.Connect(context.Background(), dbURL)
	if err != nil {
		fmt.Printf("Unable to connect to database: %v\n", err)
		os.Exit(1)
	}

	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	insertStmt := "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)"
	id := rand.Int()
	jsonBytes, err := json.Marshal(testData())
	if err != nil {
		fmt.Printf("Unable to marshall test data: %v\n", err)
		os.Exit(1)
	}

	_, err = conn.Exec(ctx, insertStmt, id, string(jsonBytes))
	if err != nil {
		fmt.Printf("Unable to insert test data: %v\n", err)
		os.Exit(1)
	}

	exitCode := m.Run()

	deleteStmt := "DELETE FROM encrypted where id=$1"

	cleanupCtx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	_, err = conn.Exec(cleanupCtx, deleteStmt, id)
	if err != nil {
		fmt.Printf("Unable to delete test data: %v\n", err)
		os.Exit(1)
	}

	os.Exit(exitCode)
}

func TestSelectJsonbContainsWithString(t *testing.T) {
	t.Parallel()
	selector := map[string]interface{}{
		"string": "hello",
	}
	selectJsonbContains(t, selector, true)
}

func TestSelectJsonbContainsWithStringNegative(t *testing.T) {
	t.Parallel()
	selector := map[string]interface{}{
		"string": "blah",
	}
	selectJsonbContains(t, selector, false)
}

func TestSelectJsonbContainsWithNumber(t *testing.T) {
	t.Parallel()
	selector := map[string]interface{}{
		"number": 42,
	}
	selectJsonbContains(t, selector, true)
}

func TestSelectJsonbContainsWithNumberNegative(t *testing.T) {
	t.Parallel()
	selector := map[string]interface{}{
		"number": 11,
	}
	selectJsonbContains(t, selector, false)
}

func TestSelectJsonbContainsWithNumericArray(t *testing.T) {
	t.Parallel()
	selector := map[string]interface{}{
		"array_number": []int{42, 84},
	}
	selectJsonbContains(t, selector, true)
}

func TestSelectJsonbContainsWithNumericArrayNegative(t *testing.T) {
	t.Parallel()
	selector := map[string]interface{}{
		"array_number": []int{1, 2},
	}
	selectJsonbContains(t, selector, false)
}

func TestSelectJsonbContainsWithStringArray(t *testing.T) {
	t.Parallel()
	selector := map[string]interface{}{
		"array_string": []string{"hello", "world"},
	}
	selectJsonbContains(t, selector, true)
}

func TestSelectJsonbContainsWithStringArrayNegative(t *testing.T) {
	t.Parallel()
	selector := map[string]interface{}{
		"array_string": []string{"blah", "vtha"},
	}
	selectJsonbContains(t, selector, false)
}

func TestSelectJsonbContainsWithNestedObject(t *testing.T) {
	t.Parallel()
	selector := map[string]interface{}{
		"nested": map[string]interface{}{
			"number": 1815,
			"string": "world",
		},
	}
	selectJsonbContains(t, selector, true)
}

func TestSelectJsonbContainsWithNestedObjectNegative(t *testing.T) {
	t.Parallel()
	selector := map[string]interface{}{
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
	expectedResult := ExpectedResult{
		Type:  ExpectedNativeBool,
		Value: expected,
	}

	jsonBytes, err := json.Marshal(selector)
	require.NoError(t, err)

	selectJsonb(t, string(jsonBytes), selectStmt, selectTemplate, expectedResult)
}

func TestJsonbContainedByWithString(t *testing.T) {
	t.Parallel()
	selector := map[string]interface{}{
		"string": "hello",
	}
	selectJsonbContains(t, selector, true)
}

func TestJsonbContainedByWithStringNegative(t *testing.T) {
	t.Parallel()
	selector := map[string]interface{}{
		"string": "blah",
	}
	selectJsonbContains(t, selector, false)
}

func TestJsonbContainedByWithNumber(t *testing.T) {
	t.Parallel()
	selector := map[string]interface{}{
		"number": 42,
	}
	selectJsonbContains(t, selector, true)
}

func TestJsonbContainedByWithNumberNegative(t *testing.T) {
	t.Parallel()
	selector := map[string]interface{}{
		"number": 11,
	}
	selectContainedByJsonb(t, selector, false)
}

func TestJsonbContainedByWithNumericArray(t *testing.T) {
	t.Parallel()
	selector := map[string]interface{}{
		"array_number": []int{42, 84},
	}
	selectJsonbContains(t, selector, true)
}

func TestJsonbContainedByWithNumericArrayNegative(t *testing.T) {
	t.Parallel()
	selector := map[string]interface{}{
		"array_number": []int{1, 2},
	}
	selectJsonbContains(t, selector, false)
}

func TestJsonbContainedByWithStringArray(t *testing.T) {
	t.Parallel()
	selector := map[string]interface{}{
		"array_string": []string{"hello", "world"},
	}
	selectJsonbContains(t, selector, true)
}

func TestJsonbContainedByWithStringArrayNegative(t *testing.T) {
	t.Parallel()
	selector := map[string]interface{}{
		"array_string": []string{"blah", "vtha"},
	}
	selectJsonbContains(t, selector, false)
}

func TestJsonbContainedByWithNestedObject(t *testing.T) {
	t.Parallel()
	selector := map[string]interface{}{
		"nested": map[string]interface{}{
			"number": 1815,
			"string": "world",
		},
	}
	selectJsonbContains(t, selector, true)
}

func TestJsonbContainedByWithNestedObjectNegative(t *testing.T) {
	t.Parallel()
	selector := map[string]interface{}{
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
func selectJsonbPathQueryFirst(t *testing.T, selector string, expected ExpectedResult) {
	selectJsonb(
		t,
		selector,
		"SELECT jsonb_path_query_first(encrypted_jsonb, $1) FROM encrypted",
		"SELECT jsonb_path_query_first(encrypted_jsonb, '%s') FROM encrypted",
		expected,
	)
}

func TestSelectJsonbPathQueryFirstString(t *testing.T) {
	t.Parallel()
	var expected = ExpectedResult{
		Type:  ExpectedJsonValue,
		Value: "hello",
	}
	selectJsonbPathQueryFirst(t, "$.array_string[*]", expected)
}

func TestSelectJsonbPathQueryFirstNumber(t *testing.T) {
	t.Parallel()
	var expected = ExpectedResult{
		Type:  ExpectedJsonValue,
		Value: 42.0,
	}
	selectJsonbPathQueryFirst(t, "$.array_number[*]", expected)
}

func TestSelectJsonbPathQueryFirstWithUnknown(t *testing.T) {
	t.Parallel()
	var expected = ExpectedResult{
		Type: ExpectedEmpty,
	}
	selectJsonbPathQueryFirst(t, "$.vtha", expected)
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

	require := require.New(t)

	tx, err := conn.Begin(ctx)
	require.NoError(err)
	defer tx.Rollback(ctx)

	for _, mode := range modes {
		t.Run(mode.String(), func(t *testing.T) {

			t.Run("select", func(t *testing.T) {
				switch expectedResult.Type {
				case ExpectedNoResult:
					// test parameterised version
					err := tx.QueryRow(context.Background(), selectStmt, mode, selector).Scan(nil)
					require.ErrorIs(err, sql.ErrNoRows)

					// test template version
					err = tx.QueryRow(context.Background(), fmt.Sprintf(selectTemplate, selector), mode).Scan(nil)
					require.ErrorIs(err, sql.ErrNoRows)

				case ExpectedNativeBool:
					var result bool

					// test parameterised version
					err = tx.QueryRow(context.Background(), selectStmt, mode, selector).Scan(&result)
					require.NoError(err)
					require.Equal(expectedResult.Value, result)

					// test template version
					err = tx.QueryRow(context.Background(), fmt.Sprintf(selectTemplate, selector), mode).Scan(&result)
					require.NoError(err)
					require.Equal(expectedResult.Value, result)

				case ExpectedEmpty:
					var fetchedBytes []byte

					// test parameterised version
					err := tx.QueryRow(context.Background(), selectStmt, mode, selector).Scan(&fetchedBytes)
					require.NoError(err)
					require.Equal(0, len(fetchedBytes))

					// test template version
					err = tx.QueryRow(context.Background(), fmt.Sprintf(selectTemplate, selector), mode).Scan(&fetchedBytes)
					require.NoError(err)
					require.Equal(0, len(fetchedBytes))

				case ExpectedJsonValue:
					var fetchedBytes []byte

					var result interface{}

					// test parameterised version
					err := tx.QueryRow(context.Background(), selectStmt, mode, selector).Scan(&fetchedBytes)
					require.NoError(err)
					err = json.Unmarshal(fetchedBytes, &result)
					require.NoError(err)
					require.Equal(expectedResult.Value, result)

					// test template version
					err = tx.QueryRow(context.Background(), fmt.Sprintf(selectTemplate, selector), mode).Scan(&fetchedBytes)
					require.NoError(err)
					err = json.Unmarshal(fetchedBytes, &result)
					require.NoError(err)
					require.Equal(expectedResult.Value, result)
				}
			})
		})
	}
}

func TestSelectJsonbPathQueryNumber(t *testing.T) {
	t.Parallel()
	expected := ExpectedResult{
		Type:  ExpectedJsonValue,
		Value: 42.0,
	}
	selectJsonb(t, "$.number", selectJsonbPathQueryStmt(), selectJsonbPathQueryTemplate(), expected)
}

func TestSelectJsonbPathQueryString(t *testing.T) {
	t.Parallel()
	expected := ExpectedResult{
		Type:  ExpectedJsonValue,
		Value: "world",
	}
	selectJsonb(t, "$.nested.string", selectJsonbPathQueryStmt(), selectJsonbPathQueryTemplate(), expected)
}

func TestSelectJsonbPathQueryValue(t *testing.T) {
	t.Parallel()
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
	t.Parallel()
	expected := ExpectedResult{
		Type: ExpectedNoResult,
	}
	selectJsonb(t, "$.vtha", selectJsonbPathQueryStmt(), selectJsonbPathQueryTemplate(), expected)
}

func TestSelectJsonbPathQueryWithAlias(t *testing.T) {
	t.Parallel()
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
	t.Parallel()
	expected := ExpectedResult{
		Type:  ExpectedNativeBool,
		Value: true,
	}
	selectJsonb(t, "$.number", selectJsonPathExistsQueryStmt(), selectJsonPathExistsQueryTemplate(), expected)
}

func TestSelectJsonbPathExistsString(t *testing.T) {
	t.Parallel()
	expected := ExpectedResult{
		Type:  ExpectedNativeBool,
		Value: true,
	}
	selectJsonb(t, "$.nested.string", selectJsonPathExistsQueryStmt(), selectJsonPathExistsQueryTemplate(), expected)
}

func TestSelectJsonbPathExistsValue(t *testing.T) {
	t.Parallel()
	expected := ExpectedResult{
		Type:  ExpectedNativeBool,
		Value: true,
	}
	selectJsonb(t, "$.nested", selectJsonPathExistsQueryStmt(), selectJsonPathExistsQueryTemplate(), expected)
}

func TestSelectJsonbPathExistsWithUnknownSelector(t *testing.T) {
	t.Parallel()
	expected := ExpectedResult{
		Type:  ExpectedNativeBool,
		Value: false,
	}
	selectJsonb(t, "$.vtha", selectJsonPathExistsQueryStmt(), selectJsonPathExistsQueryTemplate(), expected)
}

func TestSelectJsonbPathExistsWithAlias(t *testing.T) {
	t.Parallel()
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
