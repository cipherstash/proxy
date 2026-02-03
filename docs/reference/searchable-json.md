# Searchable JSON Functions and Operators

This document outlines the supported JSONB functions and operators in CipherStash Proxy for encrypted data.


## Table of Contents

- [Setup](#setup)
- [Important Limtiations](#important-limitations)
- [Operators](#operators)
- [->](#field_access_operator)
- [->>](#field_access_as_text_operator)
- [@>](#contains_operator)
- [<@](#contained_by_operator)
- [->>](#field_access_as_text_operator)
- [Functions](#functions)
- [jsonb_path_query](#jsonb_path_query)
- [jsonb_path_query_first](#jsonb_path_query_first)
- [jsonb_path_exists](#jsonb_path_exists)
- [jsonb_array_elements](#jsonb_array_elements)
- [jsonb_array_length](#jsonb_array_length)


## Setup

### Schema

```sql
  CREATE TABLE cipherstash (
    id SERIAL PRIMARY KEY,
    encrypted_jsonb eql_v2_encrypted
  )
```

### Encrypted column configuration
```sql
SELECT eql_v2.add_search_config(
  'cipherstash',
  'encrypted_jsonb',
  'ste_vec',
  'jsonb',
  '{"prefix": "cipherstash/encrypted_jsonb"}'
);
```

> **Note:** JSONB literals in INSERT and UPDATE statements work directly without explicit `::jsonb` type casts. The proxy infers the JSONB type from the target column and handles encryption transparently.

#### Configuration options

The `ste_vec` index configuration accepts the following options:

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `prefix` | string | (required) | Unique prefix for the index, typically `table/column` |
| `term_filters` | array | `[]` | Filters applied to indexed terms (e.g., `[{"kind": "downcase"}]`) |
| `array_index_mode` | string or object | `"all"` | Controls which array selectors are generated during indexing |

#### Array index mode

The `array_index_mode` option controls how arrays within JSONB documents are indexed. This affects which JSONPath selectors can be used to query array data.

**Preset values:**

- `"all"` (default) - Generates all selector types. This is backwards compatible with existing configurations.
- `"none"` - Disables array indexing entirely.

**Object form for fine-grained control:**

```json
{
  "item": true,
  "wildcard": true,
  "position": false
}
```

| Selector | JSONPath | Description |
|----------|----------|-------------|
| `item` | `[@]` | EQL array element selector for functions like `jsonb_array_length` |
| `wildcard` | `[*]` | Standard JSONPath wildcard for iterating array elements |
| `position` | `[0]`, `[1]`, etc. | Positional access to specific array indices |

**Example with array_index_mode:**

```sql
SELECT eql_v2.add_search_config(
  'cipherstash',
  'encrypted_jsonb',
  'ste_vec',
  'jsonb',
  '{"prefix": "cipherstash/encrypted_jsonb", "array_index_mode": "all"}'
);
```

**Example disabling positional indexing:**

```sql
SELECT eql_v2.add_search_config(
  'events',
  'payload',
  'ste_vec',
  'jsonb',
  '{"prefix": "events/payload", "array_index_mode": {"item": true, "wildcard": true, "position": false}}'
);
```

### JSON document structure

Examples assume an encrypted JSON document with the following structure:
```
{
    "string": "hello",
    "number": 1,
    "object": {
        "string": "world",
        "number": 99,
    },
    "string_array": ["hello", "world"],
    "numeric_array": [1, 2, 3, 4],
};
```


## Important Limitations

Encrypted literals cannot be passed as arguments to SQL functions. Encrypted columns can only be passed to SQL functions if the value has an encrypted search index that supports that specific function.

Examples:
- `AVG()` cannot be used on encrypted numeric values ❌
- `MIN()` and `MAX()` can be used on encrypted values with ORE index ✅
- `LOWER()` cannot be used on encrypted text (operates only on plaintext) ❌

⚠️ **CAST Operations**: CAST operations cannot work on encrypted data because casting would require decryption within the database, which is impossible. EQL's `ste_vec` configuration enables direct comparison and ordering operations on encrypted values without requiring CAST.

⚠️ **Chained Operators**: The `->` operator cannot be chained on `ste_vec` encrypted columns. Use JSONPath functions like `jsonb_path_query_first()` for deep nested access instead.

⚠️ **Array Access**: A selector path to an array field `$.array` will return the decrypted array as a json literal To access an encrypted array as an array of encrypted values (for use with functions like `jsonb_array_length`) requires the special EQL array element selector `[@]`. The selector is an extension of JSONPath and works similar to the standard wildcard `[*]` path. The wildcard selector follows the default PostgreSQL behaviour and will return the array elements as a setof encrypted values.




---------------------------------------------------------------

## Operators

<a id='field_access_operator'></a>
### `-> text returns eql_v2_encrypted decrypted as jsonb`

Extracts JSON object field with the given key.


#### Syntax

```sql
SELECT encrypted_column -> 'field' FROM table_name;
```

#### Examples

```sql
-- field path returns value
SELECT encrypted_jsonb -> 'number' FROM cipherstash;

------------------
 jsonb_path_query
------------------
 1
(1 row)
```


```sql
-- object path returns nested object
SELECT encrypted_jsonb -> 'object' FROM cipherstash;

-------------------------------------
          jsonb_path_query
-------------------------------------
 { "string": "world", "number": 99 }
(1 row)
```


```sql
-- array field path returns array
SELECT encrypted_jsonb -> 'string_array' FROM cipherstash;

-------------------
 jsonb_path_query
-------------------
 ["hello","world"]
(1 row)
```


---------------------------------------------------------------



<a id='field_access_as_text_operator'></a>
### `->> text returns eql_v2_encrypted decrypted as jsonb`

Extracts JSON object field with the given key.


#### Important Note

The `->>` selector is currently an alias for the `->` selector.
This is a limitation of the current version of Cipherstash Proxy that will be addressed in an upcoming release.

The data returned by `->>` is decrypted as the literal `json` value, instead of converting to `text` like the vanilla PostgreSQL operator.
The returned json can be cast to any valid type in the client.



#### Syntax

```sql
SELECT encrypted_column ->> 'field' FROM table_name;
```

#### Examples

```sql
-- field path returns value
SELECT encrypted_jsonb ->> 'number' FROM cipherstash;

------------------
 jsonb_path_query
------------------
 1
(1 row)
```


```sql
-- object path returns nested object
SELECT encrypted_jsonb ->> 'object' FROM cipherstash;

-------------------------------------
          jsonb_path_query
-------------------------------------
 { "string": "world", "number": 99 }
(1 row)
```


```sql
-- array field path returns array
SELECT encrypted_jsonb -> 'string_array' FROM cipherstash;

-------------------
 jsonb_path_query
-------------------
 ["hello","world"]
(1 row)
```


---------------------------------------------------------------


### `@>` (Contains Operator)


<a id='contains_operator'></a>
### `eql_v2_encrypted @> eql_v2_encrypted returns boolean`

Does the left `eql_v2_encrypted` value contain the right `eql_v2_encrypted` path/value entries at the top level?


#### Syntax

```sql
SELECT encrypted_column @> '{ .. }' FROM table_name;
```

#### Examples

```sql
-- field/value returns true
SELECT encrypted_jsonb @> '{"number": 1}' FROM cipherstash;

----------
 ?column?
----------
 t
(1 row)
```

```sql
-- field/value returns false if no match
SELECT encrypted_jsonb @> '{"number": 99}' FROM cipherstash;

----------
 ?column?
----------
 f
(1 row)
```


```sql
-- nested object
SELECT encrypted_jsonb @> '{"object": {"string": "world", "number": 99},' FROM cipherstash;

----------
 ?column?
----------
 t
(1 row)
```


---------------------------------------------------------------


### `<@` (Contained By Operator)


<a id='contained_by_operator'></a>
### `eql_v2_encrypted <@ eql_v2_encrypted returns boolean`

Is the first JSON value contained in the second?


#### Syntax

```sql
SELECT '{ .. }' <@ encrypted_column FROM table_name;
```

#### Examples

```sql
-- field/value returns true
SELECT '{"number": 1}' @> encrypted_jsonb FROM cipherstash;

----------
 ?column?
----------
 t
(1 row)
```

```sql
-- field/value returns false if no match
SELECT '{"number": 99}' @> encrypted_jsonb FROM cipherstash;

----------
 ?column?
----------
 f
(1 row)
```


```sql
-- nested object
SELECT '{"object": {"string": "world", "number": 99}' @> encrypted_jsonb FROM cipherstash;

----------
 ?column?
----------
 t
(1 row)
```


---------------------------------------------------------------


## Functions

<a id='jsonb_path_query'></a>
### `jsonb_path_query(target eql_v2_encrypted, path jsonpath) returns setof eql_v2_encrypted decrypted as jsonb`

Returns all JSON items returned by the JSON path for the specified JSON value.

#### Syntax

```sql
SELECT jsonb_path_query(encrypted_column, '$.path') FROM table_name;
```

#### Examples

```sql
-- field path returns value
SELECT jsonb_path_query(encrypted_jsonb, '$.number') FROM cipherstash;

------------------
 jsonb_path_query
------------------
 1
(1 row)
```


```sql
-- object path returns nested object
SELECT jsonb_path_query(encrypted_jsonb, '$.object') FROM cipherstash;

-------------------------------------
          jsonb_path_query
-------------------------------------
 { "string": "world", "number": 99 }
(1 row)
```


```sql
-- object field path returns nested value
SELECT jsonb_path_query(encrypted_jsonb, '$.object.string') FROM cipherstash;

------------------
 jsonb_path_query
------------------
 "world"
(1 row)
```


```sql
-- array field path returns array
SELECT jsonb_path_query(encrypted_jsonb, '$.string_array') FROM cipherstash;

-------------------
 jsonb_path_query
-------------------
 ["hello","world"]
(1 row)
```


---------------------------------------------------------------


<a id='jsonb_path_query_first'></a>
### `jsonb_path_query_first(target eql_v2_encrypted, path jsonpath) returns eql_v2_encrypted decrypted as jsonb`

Returns all JSON items returned by the JSON path for the specified JSON value.

#### Syntax

```sql
SELECT jsonb_path_query_first(encrypted_column, '$.path') FROM table_name;
```

#### Examples

```sql
-- Returns first element of array
SELECT jsonb_path_query_first(encrypted_jsonb, '$.string_array[*]') FROM cipherstash;

------------------------
 jsonb_path_query_first
------------------------
 "hello"
(1 row)
```


```sql
-- Returns first element of array
SELECT jsonb_path_query_first(encrypted_jsonb, '$.numeric_array[*]') FROM cipherstash;

------------------------
 jsonb_path_query_first
------------------------
 1
(1 row)
```


---------------------------------------------------------------

<a id='jsonb_path_exists'></a>
### `jsonb_path_exists(target eql_v2_encrypted, path jsonpath) returns bool`

Checks whether the JSON path returns any item for the specified JSON value.

#### Syntax

```sql
SELECT jsonb_path_exists(encrypted_column, '$.path') FROM table_name;
```

#### Examples

```sql
-- Check if field exists
SELECT jsonb_path_exists(encrypted_jsonb, '$.number') FROM cipherstash;

 jsonb_path_exists
-------------------
 t
(1 row)
```

```sql
-- returns false if field not found
SELECT jsonb_path_exists(encrypted_jsonb, '$.unknown') FROM cipherstash;

 jsonb_path_exists
-------------------
 f
(1 row)
```


---------------------------------------------------------------



<a id='jsonb_array_elements'></a>
### `jsonb_array_elements(target eql_v2_encrypted) returns setof eql_v2_encrypted decrypted as jsonb`

Expands the top-level JSON array into a set of values.


#### Important Note

To access encrypted array elements requires the array element selector `[@]`.

The selector is an extension of JSONPath and works similar to the standard wildcard `[*]` path.


```
$.path[@]
$.string_array[@]
$.numeric_array[@]
```


#### Syntax

```sql
SELECT jsonb_array_elements(jsonb_path_query(encrypted_column, '$.path[@]')) FROM table_name;
```

#### Examples

```sql
-- string array
SELECT jsonb_array_elements(jsonb_path_query(encrypted_jsonb, '$.string_array[@]')) FROM cipherstash;

 jsonb_array_elements
----------------------
 "hello"
 "world"
(2 rows)
```

```sql
-- numeric array
SELECT jsonb_array_elements(jsonb_path_query(encrypted_jsonb, '$.numeric_array[@]')) FROM cipherstash;

 jsonb_array_elements
----------------------
 1
 2
 3
 4
(4 rows)
```


---------------------------------------------------------------



<a id='jsonb_array_length'></a>
### `jsonb_array_length(target eql_v2_encrypted) returns integer`

Returns the number of elements in the top-level JSON array.

#### Important Note

To access encrypted array elements requires the array element selector `[@]`.

The selector is an extension of JSONPath and works similar to the standard wildcard `[*]` path.


```
$.path[@]
$.string_array[@]
$.numeric_array[@]
```


#### Syntax

```sql
SELECT jsonb_array_length(jsonb_path_query(encrypted_column, '$.path[@]')) FROM table_name;
```

#### Examples

```sql
-- string array
SELECT jsonb_array_length(jsonb_path_query(encrypted_jsonb, '$.string_array[@]')) FROM cipherstash;

  jsonb_array_length
--------------------
                  2
(1 row)
```

```sql
-- numeric array
SELECT jsonb_array_length(jsonb_path_query(encrypted_jsonb, '$.numeric_array[@]')) FROM cipherstash;

 jsonb_array_length
--------------------
                  4
(1 row)
```


```sql
-- returns NULL if field not found
SELECT jsonb_array_length(jsonb_path_query(encrypted_jsonb, '$.unknown')) FROM cipherstash;

 jsonb_array_length
--------------------
(0 rows)
```

---------------------------------------------------------------









=======================================








---------------------------------------------------------------



## Containment Operators

> **Note:** Containment operators work directly with JSONB literals without requiring explicit `::jsonb` type casts. The examples below use the simplified syntax intentionally.

### `@>` (Contains Operator)

Tests whether the left JSONB value contains the right JSONB value.

**Syntax:**
```sql
SELECT encrypted_jsonb @> '{"field": "value"}' FROM table_name;
```

**Examples:**
```sql
-- Check if contains string field
SELECT encrypted_jsonb @> '{"string": "hello"}' FROM encrypted;

-- Check if contains numeric field
SELECT encrypted_jsonb @> '{"number": 42}' FROM encrypted;

-- Check if contains array
SELECT encrypted_jsonb @> '{"array_number": [42, 84]}' FROM encrypted;

-- Check if contains nested object
SELECT encrypted_jsonb @> '{"nested": {"number": 1815, "string": "world"}}' FROM encrypted;
```

**Supported Containment Types:**
- String fields
- Numeric fields
- Complete arrays
- Nested objects

### `<@` (Contained By Operator)

Tests whether the left JSONB value is contained by the right JSONB value.

**Syntax:**
```sql
SELECT '{"field": "value"}' <@ encrypted_jsonb FROM table_name;
```

**Examples:**
```sql
-- Check if value is contained by the document
SELECT '{"string": "hello"}' <@ encrypted_jsonb FROM encrypted;

-- Check numeric containment
SELECT '{"number": 42}' <@ encrypted_jsonb FROM encrypted;

-- Check array containment
SELECT '{"array_string": ["hello", "world"]}' <@ encrypted_jsonb FROM encrypted;
```

## Comparison Operators in WHERE Clauses

All standard comparison operators work with JSON field extraction:

### Equality (`=`)

```sql
-- Using field access operator
SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> 'string' = 'B';

-- Using JSONPath
SELECT encrypted_jsonb FROM encrypted WHERE jsonb_path_query_first(encrypted_jsonb, '$.string') = 'B';
```

### Greater Than (`>`)

```sql
-- String comparison
SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> 'string' > 'C';

-- Numeric comparison
SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> 'number' > 4;
```

### Greater Than or Equal (`>=`)

```sql
SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> 'string' >= 'C';
SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> 'number' >= 4;
```

### Less Than (`<`)

```sql
SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> 'string' < 'B';
SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> 'number' < 3;
```

### Less Than or Equal (`<=`)

```sql
SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> 'string' <= 'B';
SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> 'number' <= 3;
```

## JSONPath Syntax

CipherStash Proxy supports JSONPath expressions for field access:

- `$.field` - Access top-level field
- `$.nested.field` - Access nested field
- `$.array[*]` - Array wildcard (all elements)
- `$.array[@]` - Array elements for processing functions

## Usage Patterns

### Parameterized Queries

All functions support parameterized queries for security:

```sql
-- Parameterized field access
SELECT encrypted_jsonb -> $1 FROM encrypted;

-- Parameterized JSONPath query
SELECT jsonb_path_query(encrypted_jsonb, $1) FROM encrypted;

-- Parameterized containment check
SELECT encrypted_jsonb @> $1 FROM encrypted;
```

### Combining with Other Functions

JSON functions can be combined with standard SQL operations:

```sql
-- Using aliases
SELECT jsonb_path_exists(encrypted_jsonb, '$.nested') AS has_nested FROM encrypted;

-- Using in WHERE clauses
SELECT * FROM encrypted WHERE jsonb_path_exists(encrypted_jsonb, '$.active') = true;

-- Combining multiple conditions
SELECT * FROM encrypted
WHERE encrypted_jsonb -> 'status' = 'active'
  AND jsonb_array_length(jsonb_path_query(encrypted_jsonb, '$.tags[@]')) > 0;
```

## Data Type Support

The following JSON data types are fully supported:

- **Strings**: `"hello world"`
- **Numbers**: `42`, `3.14`
- **Booleans**: `true`, `false`
- **Arrays**: `[1, 2, 3]`, `["a", "b", "c"]`
- **Objects**: `{"key": "value"}`
- **Nested structures**: `{"user": {"name": "John", "age": 30}}`

## Error Handling

- Non-existent fields return `NULL`
- Invalid JSONPath expressions may cause query errors
- Type mismatches in comparisons follow PostgreSQL JSONB semantics
- Array functions on non-arrays return empty results