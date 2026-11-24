# check_copy_into

- File: `packages/cipherstash-proxy/src/postgresql/middleware/frontend.rs`
- Lines: L1396-L1455 (doc comment L1396-L1399, body L1400-L1455)
- Signature: `fn check_copy_into(table_name: &ast::ObjectName, columns: &[ast::Ident], resolver: &TableResolver) -> Result<(), Error>`
- Kind: free function, synchronous, no `self`, no I/O, no writes to shared state.
- Format note: `ANALYSIS_FORMAT.md` was not present on disk (the `{baseDir}` placeholder did not resolve), so this file uses the section order of the earlier files in this directory: purpose, reachability, inputs, outputs, block-by-block, callees, invariants, assumptions, shared state, open questions.

Abbreviations used for citations:

- `fe` = `packages/cipherstash-proxy/src/postgresql/middleware/frontend.rs`
- `ctx` = `packages/cipherstash-proxy/src/postgresql/context/mod.rs`
- `lc` = `packages/cipherstash-proxy/src/postgresql/context/lifecycle.rs`
- `st` = `packages/cipherstash-proxy/src/postgresql/context/settings.rs`
- `sm` = `packages/cipherstash-proxy/src/proxy/schema/middleware.rs`
- `mgr` = `packages/cipherstash-proxy/src/proxy/schema/manager.rs`
- `sql` = `packages/cipherstash-proxy/src/proxy/sql/select_table_schemas.sql`
- `err` = `packages/cipherstash-proxy/src/error.rs`
- `em` = `packages/eql-mapper/src/eql_mapper.rs`
- `sch` = `packages/eql-mapper/src/model/schema.rs`
- `tr` = `packages/eql-mapper/src/model/table_resolver.rs`
- `sd` = `packages/eql-mapper/src/model/schema_delta.rs`
- `ic` = `packages/eql-mapper/src/model/ident_case.rs`
- `ix` = `packages/eql-mapper/src/iterator_ext.rs`

## 1. Purpose

Decides whether a `COPY <table> [(columns)] FROM ...` statement may be forwarded to PostgreSQL unchanged.
It returns `Ok(())` only when the table resolves to exactly one table in `resolver`, every target column resolves in that table, the target set is non-empty, and every target column has `ColumnKind::Native` and is not an unquoted column named `like` (fe L1405-L1454).
Every other outcome is an `Err` carrying a `MappingError`.
Proxy does not transform copy data (doc comment fe L1396-L1399), so this function is the only per-column check a `COPY ... FROM` receives before it is forwarded (fe L1257-L1262).

## 2. Reachability and caller contract

The only call site is `Frontend::route` (fe L1259).
`route` has two callers:

- `Frontend::query_handler`, once per statement of a simple-query batch, with the per-statement resolver from `context.batch_table_resolvers` (fe L448-L452).
- `Frontend::parse_handler`, for the single statement of a `Parse` message, with `context.get_table_resolver()` (fe L834).

`route` reaches `check_copy_into` only when all three of the following hold:

1. `self.context.unsafe_disable_mapping()?` is false (fe L1254-L1256).
   When it is true, `route` returns `Route::Forward(Forward::MappingDisabled)` before looking at the statement.
   The value is the confirmed connection setting `settings.unsafe_disable_mapping()` (ctx L1630-L1636, st L398-L400), which the client sets with `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING`.
2. `eql_mapper::copies_into_table(statement)` returns `Some` (fe L1257).
3. `self.context.has_encrypted_columns()` is true (fe L1258).
   When it is false, the function is skipped and the `COPY` is forwarded (fe L1261), as the test at fe L4593-L4599 records.

After the call, `route` returns `Route::Forward(Forward::NothingEncrypted)` for every `COPY ... FROM <table>` whether or not the check ran (fe L1261).
An `Err` from `check_copy_into` propagates out of `route` through `?` (fe L1259) and out of both callers through `?` (fe L452, L834).

Upstream of `route`, the operator-level `context.mapping_disabled()` (ctx L1794-L1796, `config/tandem.rs` L317-L322) diverts messages before `query_handler` or `parse_handler` run, per the earlier analysis in `audit-context/functions/02-Frontend-query_handler.md` section 2.
On that path `check_copy_into` is not reached.

Preconditions the callers establish before `route`:

- Simple query: `prepare_schema_for_statement` (fe L428), `simple_query_requires_fail_closed` refusal (fe L429-L434), and, when any statement `requires_schema`, `wait_for_schema_execution` plus `ensure_schema_modelled` (fe L435-L438).
  `requires_schema` is true for a `COPY ... FROM <table>` (em L120-L122).
- Extended protocol: `prepare_parse` runs `prepare_schema_for_statement`, and for a statement that `requires_schema`, `wait_for_schema_execution` and `ensure_schema_modelled` (lc L148-L152), before `route` at fe L834.

Actor: the authenticated client supplies the SQL text from which `table_name` and `columns` are parsed.

## 3. Inputs

| Input | Source | Controlled by |
|---|---|---|
| `table_name: &ast::ObjectName` | `Statement::Copy { source: CopySource::Table { table_name, .. }, to: false }` (em L105-L113) | client SQL |
| `columns: &[ast::Ident]` | same AST node, `columns` (em L109, L113); empty slice means no column list (doc em L98-L100) | client SQL |
| `resolver: &TableResolver` | simple query: `SchemaMiddleware::batch_resolvers` (sm L872-L916); extended: `SchemaMiddleware::effective_resolver` (ctx L1351-L1353, sm L373-L378) | catalog snapshot loaded by Proxy, plus this connection's modelled DDL and temporary tables |

The resolver handed in is always a `TableResolver::WithTemporary { temporary, permanent }` on the extended path (sm L374-L377).
On the simple-query path it starts as `effective_resolver()` (sm L873) and is replaced by `with_temporary(snapshot(temporary), snapshot(catalog))` after each earlier statement that produces schema changes (sm L909-L912).

## 4. Outputs and side effects

- `Ok(())`: the caller forwards the `COPY` statement text unchanged (fe L1261).
- `Err(MappingError::AmbiguousTable { table })`: the name resolves in both the temporary and permanent layer (fe L1408, tr L88).
- `Err(MappingError::UnresolvableTable { table })`: any other resolution failure (fe L1409), an empty target set (fe L1428-L1433), or a target column that is an unquoted `like` (fe L1440-L1445).
- `Err(MappingError::CopyIntoUnknownColumn { table, column })`: a listed column does not resolve uniquely in the table (fe L1412-L1415, L1422).
- `Err(MappingError::CopyIntoEncryptedColumn { table, column })`: a target column whose kind is not `Native` (fe L1446-L1452).

Error text is defined at err L145-L155.
The `table` string in `UnresolvableTable` and `AmbiguousTable` is `table_name.to_string()`, the client's spelling (fe L1406, L1430, L1442).
The `table` string in `CopyIntoUnknownColumn` and `CopyIntoEncryptedColumn` is `table.name.value`, the resolved table's stored name (fe L1413, L1448).
The `column` string is the client's `column.value` for the unknown case (fe L1414) and the resolved column's `name.value` for the encrypted case (fe L1449).

The function writes no state.
It takes a read lock inside `resolve_table` when the owning layer is `ViaSchemaWithEdits` (tr L99-L101) and releases it before returning.
The returned `Arc<Table>` is a detached copy on that path (sd L94, L238-L253), so later code in the function reads a snapshot, not live overlay state.

## 5. Block-by-block

### 5.1 Table resolution (fe L1405-L1411)

`resolver.resolve_table(table_name)` is called once.
`SchemaError::AmbiguousTable(_)` maps to `MappingError::AmbiguousTable`; every other `SchemaError` variant maps to `MappingError::UnresolvableTable` (fe L1407-L1410).
`?` returns the error; the `From<MappingError> for Error` conversion is applied by `?`.

### 5.2 `unknown` closure (fe L1412-L1415)

Builds `CopyIntoUnknownColumn` from the resolved table's name and the client's column spelling.
It is used only at fe L1422.

### 5.3 Target selection (fe L1417-L1424)

- Empty `columns`: `targets = table.columns.clone()`, every column of the resolved table (fe L1417-L1418).
- Non-empty `columns`: each listed ident is looked up with `table.get_column(column)`; the first failure returns `unknown(column)` through `collect::<Result<_, _>>()?` (fe L1420-L1423).
  The `SchemaError` from `get_column` is discarded by `map_err(|_| ...)` (fe L1422), so "not found" and "matches more than one column" both become `CopyIntoUnknownColumn`.

On the non-empty branch, `targets` holds only the listed columns.
Columns of the table that are not listed are not inspected by the loop at fe L1435-L1453.
Duplicate idents in `columns` are not rejected here; each resolves independently and the same `Arc<Column>` appears twice in `targets`.

### 5.4 Empty-target defence (fe L1425-L1433)

`targets.is_empty()` returns `UnresolvableTable`.
Given fe L1417, `targets` is empty only when `columns` is empty and `table.columns` is empty: a non-empty `columns` either yields one target per ident or returns at fe L1423.
The comment (fe L1425-L1427) names the case: a table Proxy knows with no columns Proxy knows.
Tables with zero known columns arise from `OverlayTable::new` (sd L188-L193), used when DDL targets a name the base schema does not resolve (sd L82) and by `capture_create_table` when `columns` is empty (sd L306-L317).

### 5.5 Per-column loop (fe L1435-L1453)

For each target, in order:

1. `LIKE` defence (fe L1436-L1445): when `column.name.quote_style.is_none()` and `column.name.value.eq_ignore_ascii_case("like")`, return `UnresolvableTable`.
   The comment states that the parser reads `CREATE TABLE t (LIKE u)` as a column named `LIKE`.
   The test at fe L4580-L4585 exercises `CREATE TABLE copied (LIKE encrypted); COPY copied FROM STDIN` and expects `UnresolvableTable { table: "copied" }`.
2. Kind check (fe L1446-L1452): any kind other than `ColumnKind::Native` returns `CopyIntoEncryptedColumn`.
   `ColumnKind` has three variants, `Native`, `Eql(..)`, `UnmappableEncrypted(..)` (sch L49-L72), so both `Eql` and `UnmappableEncrypted` take this branch.

The loop returns at the first failing column, so the reported column is the first in `targets` order: table column order for an empty list, client list order otherwise.

### 5.6 Success (fe L1454)

`Ok(())`.

## 6. Callees

### 6.1 `TableResolver::resolve_table` (tr L96-L104), source available

Calls `self.owner(name)?` then dispatches on the single owning layer.
`owner` (tr L69-L94):

- Not `WithTemporary`: returns `self` (tr L70-L76).
- Two-part name whose qualifier equals `pg_temp` under `IdentCase`: temporary layer (tr L79-L83).
- One-part name: resolves the name in both layers; both `Ok` gives `Err(AmbiguousTable)` (tr L88); temporary only gives the temporary layer (tr L89); otherwise the permanent layer (tr L90).
- Any other shape (two-part with another qualifier, three or more parts): permanent layer (tr L92).

The `unreachable!` at tr L102 holds when neither layer is itself `WithTemporary`.
`effective_resolver` passes `Temporary.resolver` (built by `TableResolver::new_temporary`, tr L32-L34, sm L138) and `self.resolver()` (built by `new_editable`, sm L399) (sm L374-L377), and `batch_resolvers` passes two `snapshot(..)` results, each `ViaSchemaWithEdits` (sm L227-L231, L909-L912).

Layer resolution:

- `ViaSchema`: `Schema::resolve_table` (tr L98).
- `ViaSchemaWithEdits`: `SchemaWithEdits::resolve_table` under a read lock (tr L99-L101).
  `.read().unwrap()` panics if the lock is poisoned.

`SchemaWithEdits::resolve_table` (sd L89-L101):

- Computes `canonical_key(name)`, which drops a two-part qualifier equal to the schema's own name and folds each ident with `canonical_ident` (sd L52-L69, L156-L168).
- An overlay entry `Dropped` gives `TableNotFound` (sd L93).
- An overlay entry `Table` gives a freshly built `Table` (sd L94, L238-L253).
- No overlay entry: falls through to `self.schema.resolve_table(name)` with the caller's original name (sd L96-L99).

`Schema::resolve_table` (sch L180-L202):

- Names with zero or more than two parts return `TableNotFound` (sch L188-L192).
- A one-part name matches a table of that name in any loaded schema; a two-part name additionally requires `table.is_in_schema(&self.name, qualifier)` (sch L196-L199, L260-L263).
- `find_unique` returns `Ok` only when exactly one table matches (ix L13-L17); zero or several both map to `TableNotFound` (sch L201).

What `check_copy_into` depends on this callee for:

- That `Ok(table)` is the single table Proxy's model associates with the client's name, and that names matching two modelled tables do not resolve.
  Established at ix L13-L17 and tr L84-L91 for the names Proxy models.
- That `table.columns` is the complete column list of that table as Proxy models it.
  Established by the loader (mgr L414-L426) and by the overlay edits (sd L306-L390); see sections 8.3 and 8.4 for the paths where the list is not the catalog's.

### 6.2 `Table::get_column` (sch L277-L283), source available

Returns the unique column whose name equals the argument under `IdentCase`; zero or several matches return `ColumnNotFound` (ix L13-L17, sch L282).
`check_copy_into` depends on it to select the column PostgreSQL would select for the same ident.
Both failure shapes are folded into `CopyIntoUnknownColumn` at fe L1422.

### 6.3 `IdentCase` equality (ic L37-L59), reached through 6.1 and 6.2

- Byte lengths must match (ic L39-L41).
- Both unquoted: per-character comparison of `char::to_lowercase` on both sides (ic L44-L46).
- One quoted, one unquoted: the unquoted side is lowercased with `char::to_lowercase` and compared to the quoted side's characters as written (ic L48-L54).
- Both quoted: exact string equality (ic L56).

`char::to_lowercase` is Unicode lowercasing.
The overlay's `canonical_ident` folds with `to_ascii_lowercase` (sd L156-L161), as does `postgres_identifier` (sm L1037-L1043).
The two foldings give different results for unquoted identifiers containing non-ASCII upper-case characters.

### 6.4 `eql_mapper::copies_into_table` (em L103-L116), called by the caller at fe L1257

Returns `Some((table_name, columns))` only for `Statement::Copy` with `source: CopySource::Table` and `to: false`; every other statement gives `None`.
`COPY ... FROM STDIN`, `FROM '<file>'` and `FROM PROGRAM` all match, since the `target` field is not inspected (em L105-L113; tests fe L4502-L4508).
`COPY <table> TO ...` and `COPY (query) TO ...` give `None` and fall to the `requires_type_check` branch (fe L1263-L1273).
`check_copy_into` depends on this function to supply the table and column list that PostgreSQL will act on for the same statement text.

### 6.5 `Context::has_encrypted_columns` (ctx L1748-L1751), caller-side gate at fe L1258

True when the connection's encrypt config is non-empty or `SchemaMiddleware::schema_has_encrypted_columns()` is true.
The latter reads `self.base`, the adopted committed snapshot (sm L382-L384, L393-L395), and `Schema::has_encrypted_columns` counts any non-`Native` kind (sch L146-L153).
It does not read the transaction overlay resolver or the temporary layer.
The encrypt config is updated by `apply_ddl` for modelled DDL (sm L928-L936).

### 6.6 Formatting and clones

`table_name.to_string()` (fe L1406, L1430, L1442), `String::clone` and `Vec<Arc<Column>>::clone` (fe L1413-L1418, L1448-L1449) are standard-library or `Display` calls with no dependency beyond allocation.

## 7. Invariants

Each holds on every path through the function.

1. `Ok(())` implies `resolver.resolve_table(table_name)` returned `Ok` (fe L1405-L1411; `?` returns on `Err`).
2. `Ok(())` implies `targets` was non-empty (fe L1428-L1433).
3. `Ok(())` implies every element of `targets` has `kind == ColumnKind::Native` (fe L1446-L1452) and is not an unquoted ident equal to `like` ignoring ASCII case (fe L1440-L1445).
4. With an empty `columns`, `targets` equals `table.columns` in full (fe L1417-L1418), so invariant 3 covers every column of the resolved table.
5. With a non-empty `columns`, `targets` contains exactly one resolved column per listed ident, each a unique `IdentCase` match in `table.columns` (fe L1420-L1423, sch L277-L283, ix L13-L17).
   Columns of the table not listed are outside `targets`.
6. Both `ColumnKind::Eql` and `ColumnKind::UnmappableEncrypted` produce `CopyIntoEncryptedColumn` (fe L1446; sch L49-L72 lists the variants).
7. A name resolving in both the temporary and permanent layers never returns `Ok` (tr L88, fe L1408).
8. A name with three or more parts never returns `Ok` (tr L92, sch L191, sd L99; an overlay key for such a name exists only if DDL created one under the same canonical key, sd L52-L69, L91).
9. The function mutates nothing reachable from its arguments (all parameters are shared references, fe L1401-L1403; the only lock taken is a read lock, tr L100).

## 8. Assumptions

Each entry states what the function takes on faith and where, if anywhere, it is established.

### 8.1 The resolved table is the relation PostgreSQL will copy into

The function equates "the unique table of this name in Proxy's model" with "the relation PostgreSQL resolves this name to for this session".

- For a bare name, the model matches a table of that name in any loaded schema (sch L177-L179, L196-L199).
  The loaded schemas are those in `current_schemas(false)` of Proxy's own schema-loading connection (sql L30; `load_snapshot` connects with `connect::database(config)`, mgr L464-L470).
- The client session's `search_path` is not consulted.
  A search of `packages/cipherstash-proxy/src` for `search_path` finds only test fixtures (`config/tandem.rs` L1450, `postgresql/middleware/mod.rs` L1324 and L1609, `postgresql/parser.rs` L381 and L405).
- Established by: nothing found.
  The line that would relate the client session's name resolution to the model is the `resolve_table` call at fe L1405.

A table in a schema the loader did not load is unknown to the model.
A bare name for it resolves in the model only if a same-named table exists in a loaded schema, in which case the model's table is the one checked.

### 8.2 Identifier comparison matches PostgreSQL's

- Catalog table names are stored as unquoted idents: `Ident::new(&table_name)` (mgr L411).
  Catalog column names and schema names are stored quoted (mgr L361, L410).
- An unquoted client table name is compared with an unquoted stored table name by Unicode-lowercasing both sides (ic L44-L46).
  A catalog table whose name contains upper-case characters therefore matches the client's unquoted lower-case spelling in the model.
- A quoted client table name is compared with the unquoted stored name by lowercasing the stored side (ic L52-L54), so a quoted mixed-case client name does not match the mixed-case catalog table of exactly that name.
- Unquoted client column names are Unicode-lowercased before comparison with the quoted stored column (ic L48-L50).
  The overlay and config paths fold ASCII only (sd L160, sm L1041).
- Established by: `IdentCase` (ic L37-L59) for ASCII identifiers where the stored ident carries the right quoting; nothing found for unquoted-stored mixed-case table names (mgr L411) or for non-ASCII upper-case characters in unquoted client identifiers.

### 8.3 `table.columns` is the catalog's column list, with correct kinds

- Loaded tables: columns and kinds come from `information_schema.columns` through `classify_column` (mgr L355-L387, L414-L426).
  A column is `Native` when its domain name does not resolve through `eql_domains::resolve` and neither its `udt_name` nor its domain name equals the legacy v2 type name (mgr L366-L386).
  The source is the snapshot current at the connection's last adoption (sm L392-L402), not the catalog at execution time.
- Only `table_type = 'BASE TABLE'` rows are loaded (sql L20).
  Views and other relation kinds are absent from the model, so a `COPY` naming one fails resolution at fe L1405 unless an overlay entry exists.
- Overlay tables: `CREATE VIEW` creates an overlay table whose columns are all `Native` (sd L292-L304).
  The middleware treats `CreateView` as not modelled (sm L1197) and as possibly changing encryption (sm L603-L607), and `batch_resolvers` applies `Catalog` changes only when `models_ddl` is true (sm L899-L906).
- `CREATE TABLE` overlay columns take their kind from `column_kind` (sm L1030-L1034) via the `classify` callback (sd L310-L315).
  `column_kind` has no `UnmappableEncrypted` outcome (sm L1030-L1034).
- Established by: `load_snapshot` (mgr L464-L494) and `SchemaMiddleware` DDL modelling (sm L928-L936, sd L306-L390), not by `check_copy_into`.

### 8.4 Columns Proxy's parser did not see are caught by the two defences

Tables whose real column set differs from the parsed `columns` list (`LIKE`, `CREATE TABLE AS`, inheritance, clone) are handled as follows:

- `create.query`, `create.like`, `create.clone`, `create.inherits` make `ddl_may_change_encryption` true (sm L557-L563) and `is_modelled_ddl` false (sm L1175-L1178).
  In a simple-query batch, a later `COPY ... FROM` after such a statement is refused by `simple_query_requires_fail_closed` (sm L541-L552, fe L429-L434).
  After confirmation, `ensure_schema_modelled` refuses schema-dependent statements while the unmodelled flags are set (ctx L1408-L1413, sm L433-L435).
- The parenthesised `(LIKE u)` form is stated to parse as a column named `LIKE` rather than setting `create.like` (fe L1436-L1439, test fe L4580-L4585).
  That table is modelled, with a `Native` column named `like` after `canonical_ident` (sd L310-L315, L156-L168).
  The loop's check at fe L1440 is what refuses it.
- The `LIKE` check applies only to columns in `targets`.
  With an explicit column list that does not name `like`, the pseudo-column is not in `targets` (fe L1420-L1423), and columns the `LIKE` clause contributed are not in `table.columns`, so naming one of them returns `CopyIntoUnknownColumn` at fe L1422.
- A real column declared in DDL as `"like"` is stored unquoted by `canonical_ident` (sd L163-L164) and is indistinguishable from the clause at fe L1440.
  A catalog-loaded column named `like` is stored quoted (mgr L361) and is not matched by fe L1440.
- Established by: fe L1428 and fe L1440 for the shapes above; `is_modelled_ddl` (sm L1169-L1200) for the flagged fields.
  `is_modelled_ddl` does not inspect partition-of or typed-table fields of `CreateTable` (sm L1171-L1180); whether such statements reach the overlay with an empty column list, and are then stopped at fe L1428 or fe L1422, is listed in section 10.

### 8.5 The schema checked is the schema in force when PostgreSQL runs the COPY

- Simple query: the check runs against the batch resolver for that statement position (fe L448-L452, sm L872-L916).
- Extended protocol: the check runs once, at `Parse` (fe L834).
  `check_copy_into` is not called from any other handler (the grep for `check_copy_into` finds only fe L1259 and the definition).
- Established by: `prepare_schema_for_statement`, `wait_for_schema_execution`, `ensure_schema_modelled` before `route` (fe L428-L438, lc L148-L152) for the moment of the check.
  For the interval between `Parse` and a later `Execute` of the same prepared statement, nothing found in this function or in `route`; see section 10.

### 8.6 Copy data reaches only the columns in `targets`

The function treats the listed columns (or all columns) of the named table as the complete set of destinations for the copied rows.
Database-side redirection of rows or values (triggers, rules, partition routing, generated columns, defaults) is outside the model.
Established by: nothing found in the Proxy code read for this analysis.

### 8.7 The caller's gates are the right conditions for skipping the check

- `has_encrypted_columns()` false skips the check (fe L1258).
  It reads the encrypt config and the base snapshot, not the overlay resolver (ctx L1748-L1751, sm L382-L384).
- `unsafe_disable_mapping()` true skips the check (fe L1254-L1256).
- Established by: `Context::has_encrypted_columns` (ctx L1748-L1751) and `ConnectionSettings::unsafe_disable_mapping` (st L398-L400); `check_copy_into` has no view of either.

### 8.8 The AST matches what PostgreSQL parses

`table_name` and `columns` come from the sqltk parser's reading of the client text (em L103-L116), and the client text is forwarded unchanged on `Ok` (fe L1261).
The function assumes PostgreSQL's parse of the same bytes names the same table and columns.
Established by: nothing found; the sqltk parser is an external dependency whose source was not read.

## 9. Shared state

Read only:

- The `TableResolver` layers behind `Arc<RwLock<SchemaWithEdits>>` (tr L13, L99-L101), owned by `SchemaMiddleware` (`resolver`, `temporary`; sm L367-L378) or by the per-batch scratch copies (sm L918-L925).

Written: none.

## 10. Open questions

- Unclear whether a named prepared `COPY ... FROM` statement is re-checked at `Bind` or `Execute` after the schema changes; need to inspect `Frontend::bind_handler`, `Frontend::execute_handler` and `Context::finish_parse(operation, None)` handling for forwarded statements (fe L878 onward).
- Unclear how copy data messages following an accepted `COPY ... FROM STDIN` are handled; need to inspect `Frontend::intercept` and the pg-proto pipeline for `CopyData` (only `CopyFail` appears in `frontend.rs`, at fe L3473).
- Unclear how the sqltk parser represents `CREATE TABLE child PARTITION OF parent ...`, `CREATE TABLE t OF type`, and `CREATE TABLE t (id int, LIKE u INCLUDING ALL)`; need to inspect the sqltk `CreateTable` AST and parser to confirm which fields are set and what `columns` contains.
- Unclear whether `column_config_from_domain` returns `Some` for every domain `eql_domains::resolve` accepts; need to inspect `packages/cipherstash-proxy/src/proxy/schema/eql_domains.rs` and `column_config_from_domain`, since `has_encrypted_columns` depends on the encrypt config for transaction-local tables (ctx L1748-L1751, sm L933-L935).
- Unclear what `SCHEMA_QUERY` returns for partitioned parents, partitions and foreign tables under `table_type = 'BASE TABLE'` (sql L20); need to inspect PostgreSQL's `information_schema.tables` definition for the supported server versions.
- Unclear whether the loader connection's `current_schemas(false)` can differ from a client session's effective search path in supported deployments; need to inspect `connect::database` and the `DatabaseConfig` options handling (`config/tandem.rs` near L1450).
- Unclear whether the sqltk parser accepts `COPY` target forms that PostgreSQL reads differently (for example `ONLY`, or a trailing `*`); need to inspect the sqltk `parse_copy` implementation.
- Unclear whether `Temporary.resolver` can hold non-`Native` columns; `apply_to_temporary` classifies with `column_kind` (sm L213-L223) while `is_schema_ddl` routes temporary creates that may change encryption elsewhere (sm L655-L662, L474-L482); need to inspect `SchemaMiddleware::execution_succeeded` (sm before L815).
