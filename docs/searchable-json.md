# Searchable JSON Functions and Operators

This document outlines the supported JSONB functions and operators in CipherStash Proxy for encrypted data.


## Table of Contents

- [Setup](#setup)
- [jsonb_path_query](#jsonb_path_querytarget-eql_v2_encrypted-path-jsonpath)
- [jsonb_path_query_first](#jsonb_path_query_firsttarget-eql_v2_encrypted-path-jsonpath)
- [jsonb_path_exists](#jsonb_path_existstarget-eql_v2_encrypted-path-jsonpath)
- [jsonb_array_elements](#jsonb_array_elementstarget-eql_v2_encrypted)
- [jsonb_array_length)](#jsonb_array_lengthtarget-eql_v2_encrypted)


### Setup

Schema

```sql
  CREATE TABLE cipherstash (
    id SERIAL PRIMARY KEY,
    encrypted_jsonb eql_v2_encrypted
  )
```

Encrypted column configuration
```
SELECT eql_v2.add_search_config(
  'cipherstash',
  'encrypted_jsonb',
  'ste_vec',
  'jsonb',
  '{"prefix": "cipherstash/encrypted_jsonb"}'
);
```

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


---------------------------------------------------------------


### `jsonb_path_query(target eql_v2_encrypted, path jsonpath)`

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


### `jsonb_path_query_first(target eql_v2_encrypted, path jsonpath)`

Returns all JSON items returned by the JSON path for the specified JSON value.

#### Syntax

```sql
SELECT jsonb_path_query_first(encrypted_column, '$.path') FROM table_name;
```

#### Examples

```sql
-- field path returns value
SELECT jsonb_path_query_first(encrypted_jsonb, '$.number') FROM cipherstash;

------------------------
 jsonb_path_query_first
------------------------
 1
(1 row)
```


```sql
-- object path returns nested object
SELECT jsonb_path_query_first(encrypted_jsonb, '$.object') FROM cipherstash;

-------------------------------------
       jsonb_path_query_first
-------------------------------------
 { "string": "world", "number": 99 }
(1 row)
```


```sql
-- object field path returns nested value
SELECT jsonb_path_query_first(encrypted_jsonb, '$.object.string') FROM cipherstash;

------------------------
 jsonb_path_query_first
------------------------
 "world"
(1 row)
```


---------------------------------------------------------------

### `jsonb_path_exists(target eql_v2_encrypted, path jsonpath)`

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



### `jsonb_array_elements(target eql_v2_encrypted)`

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



### `jsonb_array_length(target eql_v2_encrypted)`

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


