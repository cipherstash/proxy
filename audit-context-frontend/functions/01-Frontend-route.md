# Frontend::route

File: `packages/cipherstash-proxy/src/postgresql/middleware/frontend.rs`, L1248-L1295.
Helpers analysed with it because they are private and only reachable through it: `may_fall_back` (L1300-L1302), `type_check` (L1304-L1315), `mapping_error` (L1320-L1393), `check_copy_into` (L1400-L1455), `to_encryptable_statement` (L909-L958).
All line numbers without a file name refer to `frontend.rs`.
`eql_mapper.rs` means `packages/eql-mapper/src/eql_mapper.rs`; `context/mod.rs` means `packages/cipherstash-proxy/src/postgresql/context/mod.rs`; `middleware.rs` means `packages/cipherstash-proxy/src/proxy/schema/middleware.rs`.

The format and example resources named in the task (`ANALYSIS_FORMAT.md`, `FUNCTION_MICRO_ANALYSIS_EXAMPLE.md`, `DOMAIN_NOTES.md`) were not found in the repository, so this document uses its own section layout.

## 1. Purpose

`route` takes one parsed statement, the client-declared parameter type OIDs, and a table resolver, and returns one of three outcomes (L1248-L1253):

- `Ok(Route::Map { typed, statement })`: the statement type checked and touches something encrypted (L1276-L1279).
- `Ok(Route::Forward(reason))`: the statement is sent as the client wrote it, with `reason` one of `MappingDisabled`, `NotTypeChecked`, `NothingEncrypted`, `Fallback(err)` (enum at L72-L81).
- `Err(_)`: the statement is refused.

`route` is synchronous, takes `&self`, and writes no connection state.
Its only side effects are log lines and the `STATEMENTS_UNMAPPABLE_TOTAL` counter inside `mapping_error` (L1331, L1348, L1360, L1372).

## 2. Inputs and who controls them

| Input | Source | Controlled by |
|---|---|---|
| `statement` | `SqlParser::parse_statements` (L411) or `SqlParser::parse_statement` (L817) over client SQL bytes | Authenticated client |
| `param_types` | `vec![]` on the simple path (L451); `Parse.parameter_types` cast `u32 as i32` on the extended path (L829-L830) | Authenticated client |
| `resolver` | `batch_table_resolvers` per statement (L450) or `get_table_resolver()` (L834) | Upstream catalog content, plus this connection's confirmed DDL and temporary tables |
| `context.unsafe_disable_mapping()` | `ConnectionSettings` under `protocol_state` read lock (`context/mod.rs` L1630-L1636) | Authenticated client via `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING`, after PostgreSQL confirms |
| `context.has_encrypted_columns()` | `encrypt_config()` and the base schema (`context/mod.rs` L1748-L1751) | Upstream catalog content and this connection's DDL overlay |
| `context.mapping_errors_enabled()` | `config.development.enable_mapping_errors` (`config/tandem.rs` L324-L329) | Operator |

## 3. Callers and what they establish first

### 3.1 `query_handler` (simple protocol), L451

Before calling `route` the caller runs, in order:

1. `validate_setting` on every statement (L412-L414).
2. `settings_simple_query_requires_fail_closed` (L415-L420), which returns `true` when a `requires_schema` statement follows a statement that changes settings (`context/mod.rs` L1539-L1546).
3. `prepare_schema_for_statement().await` (L428).
4. `simple_query_requires_fail_closed` (L429-L434), which returns `true` when a `requires_schema` statement follows DDL classified by `ddl_may_change_encryption` (`middleware.rs` L541-L552).
5. `wait_for_schema_execution().await` and `ensure_schema_modelled()`, only when at least one statement satisfies `requires_schema` (L435-L438).

Every statement in the batch is routed before any is acted on (L448-L452), and one `Err` from any statement fails the whole Query through `collect::<Result<Vec<_>, _>>()?` (L452).

After routing the caller applies two batch rules that `route` does not know about:

- Any `Route::Map` together with any `ast::Statement::Copy` in the batch is refused with `CopyWithEncryptedStatement` (L456-L462).
- Any `Route::Map` together with any `Forward::Fallback` returns the fallback's carried error (L467-L474).

The caller acts on `Route::Forward` by pushing the original statement (L536-L539) and on `Route::Map` by pushing the transformed or original typed statement (L508-L526).

### 3.2 `parse_handler` (extended protocol), L834

Before calling `route` the caller runs `begin_parse` (L810-L812), `SqlParser::parse_statement` (L817), and `prepare_parse().await` (L823-L825).
`prepare_parse` runs `prepare_schema_for_statement` (`lifecycle.rs` L148), then `wait_for_schema_execution` and `ensure_schema_modelled` only when `requires_schema(statement)` (`lifecycle.rs` L149-L152), then `validate_setting` (`lifecycle.rs` L158).

`route` runs once per Parse.
The result is stored with `finish_parse(operation, Some(statement))` for `Map` (L876) and `finish_parse(operation, None)` for `Forward` (L880).
No call to `route` exists on the Bind or Execute path; the three call sites found by search are L451, L834 and the definition L1248.

### 3.3 Gate above both callers

`Frontend::intercept` forwards every message without reaching either handler when `context.mapping_disabled()` (operator config) is true (L189-L211).
`route` itself never reads `mapping_disabled()`; it reads only the per-connection `unsafe_disable_mapping()` (L1254).

## 4. Block-by-block

### Block A: per-connection mapping switch, L1254-L1256

`self.context.unsafe_disable_mapping()?` takes the `protocol_state` read lock and returns `settings.unsafe_disable_mapping()` (`context/mod.rs` L1630-L1636), which is `self.unsafe_disable_mapping.current().unwrap_or(false)` (`context/settings.rs` L398-L400).
A poisoned lock yields `Err(ContextError::ProtocolStateUnavailable)` (`context/mod.rs` L1634), which `?` turns into a refusal.

When the value is `true`, `route` returns `Forward::MappingDisabled` before any other check.
This is the first branch, so the `COPY` check (L1257), the nested-write check (L1266), the type check (L1274) and the write policy (L1285-L1292) are all skipped on this path.
The value read is the effective value at the moment `route` runs: Query time on the simple path and Parse time on the extended path.

### Block B: `COPY ... FROM` into a table, L1257-L1262

`eql_mapper::copies_into_table` returns `Some` only for `Statement::Copy { source: CopySource::Table { .. }, to: false, .. }` (`eql_mapper.rs` L103-L116).
It returns `None` for `COPY table TO ...` (`to: true`) and for every `COPY (query) ...` form (`CopySource::Query`).

When it returns `Some`, `check_copy_into` runs only if `has_encrypted_columns()` is `true` (L1258-L1260).
When `has_encrypted_columns()` is `false`, the statement is forwarded with no resolution of the table at all (L1261).

The returned reason is `Forward::NothingEncrypted` (L1261).
The enum's doc comment says "Only `NothingEncrypted` is backed by a type check" (L69-L70) and the variant's own doc says "Type checked, and it touches no encrypted column" (L77).
On this path no type check ran; the backing is `check_copy_into`, or nothing when `has_encrypted_columns()` is `false`.
This is a mismatch between the documented meaning of the variant and one of the two sites that produce it.

`check_copy_into` (L1400-L1455):

- Resolves the table with `resolver.resolve_table` and maps `AmbiguousTable` to `MappingError::AmbiguousTable` and every other error to `UnresolvableTable` (L1405-L1411).
- With an empty column list, targets are all of the table's known columns (L1417-L1418); otherwise each named column is looked up with `table.get_column` and a miss is `CopyIntoUnknownColumn` (L1420-L1423).
- An empty target list is refused as `UnresolvableTable` (L1428-L1433).
- A target whose name is unquoted and case-insensitively equals `like` is refused as `UnresolvableTable` (L1440-L1445).
- A target whose `kind` is not `ColumnKind::Native` is refused as `CopyIntoEncryptedColumn` (L1446-L1452). This covers both `Eql` and `UnmappableEncrypted` (`model/schema.rs` L49-L72).

`check_copy_into` returns `Ok(())` only when the table resolves to exactly one table, the target list is non-empty, and every target is `Native` and not the `LIKE` placeholder.

### Block C: statements the type checker skips, L1263-L1273

`requires_type_check` is `true` only for `Query`, `Insert`, `Update`, `Delete`, `Merge`, `Prepare`, `Explain` (`eql_mapper.rs` L61-L72).
Every other statement kind reaches this block.

Inside the block, the statement is refused with `StatementCouldNotBeTypeChecked("it writes data inside a statement Proxy does not type check")` when `has_encrypted_columns() && writes_data(statement)` (L1266-L1271).
Otherwise it is forwarded as `Forward::NotTypeChecked` (L1272).

`writes_data` is a visitor that breaks on the first node that downcasts to `Statement::Insert`, `Update`, `Delete` or `Merge` (`eql_mapper.rs` L76-L96).
Its result depends entirely on which child nodes the `sqltk` visitor traversal reaches.
The unit test covers top-level writes, `PREPARE ... AS INSERT`, `EXPLAIN ANALYZE DELETE` and a data-modifying CTE (`eql-mapper/src/lib.rs` L662-L681).
The test does not cover `COPY (INSERT ... RETURNING ...) TO STDOUT`, which is the one case the comment at L1264-L1265 names.
All four inputs the test covers that are nested are kinds that `requires_type_check` returns `true` for, so none of them exercises Block C.

Statement kinds that reach `Forward::NotTypeChecked` with encrypted columns configured, because neither `requires_type_check` nor `writes_data` matches them (derived from the two match lists at `eql_mapper.rs` L62-L71 and L84-L89): `COPY table TO`, `COPY (SELECT ...) TO`, `EXECUTE`, `CALL`, `DO`, `CREATE TABLE ... AS`, `CREATE VIEW`, `CREATE FUNCTION`, `TRUNCATE`, `DECLARE CURSOR`, `FETCH`, `SET`, transaction control, and all other DDL.
Whether the visitor descends into the query of `CREATE TABLE ... AS`, `CREATE VIEW`, `DECLARE`, or the `CopySource::Query` of `COPY` is not established by anything read here.

### Block D: type check succeeded, L1274-L1281

`self.type_check` clones the resolver `Arc` and calls `eql_mapper::type_check` (L1309).
On `Ok(typed)`, `to_encryptable_statement(&typed, param_types)?` runs (L1275).

`to_encryptable_statement` (L909-L958):

- Calls `get_param_columns`, `get_projection_columns`, `get_literal_columns`, each with `?` (L914-L916).
  Each builds a `ColumnMapper` over `schema_middleware.encrypt_config()` (`context/mod.rs` L1758-L1787) and looks up every EQL-typed node's `table.column` in that config.
  A miss returns `Err(EncryptError::UnknownColumn)` (`column_mapper.rs` L189-L200).
  So a column the resolver types as EQL but the encrypt config does not contain is a refusal, not a forward.
- Returns `Ok(None)` when there is no encrypted param column, no encrypted projection column, and `typed.requires_transform()` is `false` (L921-L926).
  `literal_columns` is not part of that condition; an encrypted literal is covered only through `requires_transform()`, a dry-run transform that reports whether any edit happened (`type_checked_statement.rs` L150-L155).
- Otherwise returns `Ok(Some(Statement))` with `param_types` stored unvalidated (L949-L955).

`route` returns `Route::Map` for `Some` (L1276-L1279) and `Forward::NothingEncrypted` for `None` (L1280).
The `typed` value in `Route::Map` borrows `statement` for `'a` (L1250, L1253).

### Block E: type check failed, L1282-L1293

1. `writes_encrypted = has_encrypted_columns() && writes_data(statement)` (L1285-L1286).
2. `err = self.mapping_error(err, writes_encrypted)` (L1287).
3. `may_fall_back(&err, writes_encrypted)` decides `Forward::Fallback(err)` (L1289) or `Err(err)` (L1291).

`may_fall_back` is `!mapping_errors_enabled() && !writes_encrypted && err.is_coverage_gap()` (L1301).
`is_coverage_gap` is `true` only for `MappingError::StatementCouldNotBeTypeChecked(_)` and `MappingError::Internal(_)` (`error.rs` L67-L74).

`mapping_error` classifies in this order (L1320-L1393):

| Order | Condition | Result | Coverage gap |
|---|---|---|---|
| 1 | `err.as_unmappable_encrypted_column()` is `Some` (L1323) | `UnmappableEncryptedColumn` | no |
| 2 | `err.is_unsatisfied_bounds()` (L1342) | `UnsupportedEncryptedOperation` | no |
| 3 | `err.as_unresolved_table()` is `Some` and `writes_encrypted` (L1354) | `UnresolvableTable` / `AmbiguousTable` | no |
| 4 | `EqlMapperError::InternalError` (L1374) | `Internal` | yes |
| 5 | anything else (L1383) | `StatementCouldNotBeTypeChecked` | yes |

Row 3 is filtered by `writes_encrypted` (L1354).
An unresolved-table error on a statement that is not a write, or on any statement while `has_encrypted_columns()` is `false`, falls to row 5 and is a coverage gap.

Row 5 is the default arm.
It receives every `EqlMapperError` variant not caught above: `Transform`, `Scope`, `Import`, `Type`, `Param` (`eql_mapper.rs` L126-L148).
A type error on a statement that names encrypted columns and is not a write is therefore a coverage gap unless it is specifically `UnsatisfiedBounds` or `UnmappableEncryptedColumn`.

The detectors in rows 1-3 match only specific nestings:

- `as_unmappable_encrypted_column`: `Type(_)` and `Import(ImportError::TypeError(_))` (`eql_mapper.rs` L164-L172). `Scope(ScopeError::TypeError(_))` and `Import(ImportError::ScopeError(_))` are not matched.
- `is_unsatisfied_bounds`: `Type`, `Scope`, `Import(ScopeError)`, `Import(TypeError)` (`eql_mapper.rs` L177-L185); `TypeError::is_unsatisfied_bounds` recurses through `TypeError::ScopeError` (`inference/type_error.rs` L146-L152).
- `as_unresolved_table`: `Type`, `Scope`, `Import(SchemaError)`, `Import(ScopeError)`, `Import(TypeError)` (`eql_mapper.rs` L193-L202).

Combined decision table for a failed type check:

| `has_encrypted_columns` | writes data | `mapping_errors_enabled` | error class | outcome |
|---|---|---|---|---|
| any | any | any | rows 1-2 | `Err` |
| true | true | any | any | `Err` (L1301, `!writes_encrypted` is false) |
| true | false | false | rows 4-5 | `Forward::Fallback` |
| true | false | true | rows 4-5 | `Err` |
| false | any | false | rows 4-5 | `Forward::Fallback` |
| false | any | true | rows 4-5 | `Err` |

`PREPARE` and `MERGE` are listed in `requires_type_check` with the note that type checking fails on them (`eql_mapper.rs` L53-L60), so they always arrive in Block E.
A `PREPARE` of a non-write therefore follows the "writes data = false" rows above.
The matching SQL `EXECUTE` is not in `requires_type_check` (`eql_mapper.rs` L62-L71) and reaches Block C.

## 5. Invariants

1. Refusal is expressed only as `Err`; `Route` has no refusal variant (L44-L57).
2. With `unsafe_disable_mapping()` false and `has_encrypted_columns()` true, a statement for which `writes_data` is true has exactly two non-error outcomes: `Route::Map` (L1276) or `Forward::NothingEncrypted` after a successful type check (L1280). Block C refuses it (L1266-L1271) and Block E refuses it (L1301).
3. `Forward::Fallback` is produced at one site (L1289) and only when `mapping_errors_enabled()` is false, `writes_encrypted` is false, and the mapped error is `StatementCouldNotBeTypeChecked` or `Internal` (L1301, `error.rs` L67-L74).
4. With `has_encrypted_columns()` true, a `COPY table FROM` is forwarded only when `check_copy_into` returns `Ok`, which requires a uniquely resolved table, a non-empty target list, and every target `ColumnKind::Native` (L1258-L1260, L1405-L1454).
5. `Route::Map` is produced only after `eql_mapper::type_check` returned `Ok` and every EQL-typed param, projection and literal node resolved to a column config (L1274-L1279, L914-L916).
6. `route` mutates no connection or schema state; it takes `&self` and every context call it makes is a getter (L1249, `context/mod.rs` L1630-L1636, L1748-L1751, L1758-L1787, L1798-L1800).
7. `unsafe_disable_mapping()` true short-circuits every other check (L1254-L1256).

## 6. Assumptions

1. `writes_data` finds every write that can reach an encrypted column inside any statement kind. Established only by the `sqltk` visitor traversal behind `statement.accept` (`eql_mapper.rs` L95). The `COPY (INSERT ...) TO` case named at L1264-L1265 has no test in `eql-mapper/src/lib.rs` L662-L681. Statement kinds that write through a node that is not `Insert`/`Update`/`Delete`/`Merge` are outside the match at `eql_mapper.rs` L84-L89.
2. Statement kinds outside `requires_type_check` carry no plaintext destined for an encrypted column and return no ciphertext that needs decrypting. Stated as a presumption at `eql_mapper.rs` L51. Nothing found that enforces it for `EXECUTE`, `CALL`, `DO`, `COPY ... TO`, `CREATE TABLE ... AS`, or cursors.
3. `has_encrypted_columns()` describes the same schema generation as the `resolver` argument. `schema_has_encrypted_columns` reads `self.base` (`middleware.rs` L382-L384) while the resolver is built from `self.resolver` and `self.temporary` (`middleware.rs` L373-L378) or from a batch scratch copy (`middleware.rs` L872-L916). `encrypt_config()` is a third lock (`middleware.rs` L387-L389). The three are read at different moments with separate locks. Nothing found in `route` ties them together; the callers' `wait_for_schema_execution` (L436, `lifecycle.rs` L150) runs only when `requires_schema` is true.
4. `has_encrypted_columns() == false` means no write in this statement can reach an encrypted column. On that basis a `COPY FROM` skips `check_copy_into` (L1258), nested writes in untyped statements are forwarded (L1266), and a failed write may fall back (L1285-L1288). The value comes from the snapshot loaded from the upstream catalog; a column encrypted by another connection after the snapshot was pinned is not visible (CONTEXT.md, "Stale route").
5. The resolver's `ColumnKind` for each column is correct. `check_copy_into` trusts `ColumnKind::Native` (L1446) and the type checker trusts the same data. Established by the schema loader in `proxy/schema/manager.rs`, not read here.
6. A table name resolves to the same table in PostgreSQL as in the resolver. `Schema::resolve_table` matches a one-part name against a table of that name in any loaded schema and rejects names with more than two parts (`model/schema.rs` L188-L202). Proxy does not track `search_path` (CONTEXT.md, "Connection temporary tables"). Nothing found in `route` or `check_copy_into` checks it.
7. The route chosen at Parse stays valid for every later Bind and Execute of that prepared statement, including the `unsafe_disable_mapping` value read at L1254. No call to `route` exists outside L451 and L834. `begin_bind` reads `unsafe_disable_mapping()` again (`lifecycle.rs` L239) and `bind_handler` branches on it (L1011); how that interacts with a statement stored as `None` at L880 was not read.
8. `param_types` values are meaningful type OIDs. They arrive as `u32 as i32` (L830) and are stored in `Statement::new` (L949-L955) with no validation in `route` or `to_encryptable_statement`.
9. `requires_transform()` is true whenever a statement has an encrypted literal. `to_encryptable_statement` returns `None` without consulting `literal_columns` (L921-L926). Established by the dry-run transformer at `type_checked_statement.rs` L150-L155, whose rules were not read.
10. `eql_mapper::type_check` returning `Ok` means every table and column the statement touches was resolved and typed. Stated in its doc comment (`eql_mapper.rs` L19-L24) and by `resolve_unresolved_value_nodes` (`eql_mapper.rs` L262-L265). The inference rules were not read.
11. Every refusal raised by the mapper for an unmappable column or an unresolved table arrives in one of the wrapper shapes the three detectors match (`eql_mapper.rs` L164-L202). An error in a shape the detectors do not match becomes row 5, a coverage gap.
12. The batch rules at L456-L474 are applied by every caller that routes more than one statement. They live in `query_handler`, not in `route`.
13. The operator-level `mapping_disabled()` gate was applied before `route` is reached. Established at L189-L211; `route` does not check it.

## 7. Calls

| Callee | Kind | What `route` depends on |
|---|---|---|
| `Context::unsafe_disable_mapping` (`context/mod.rs` L1630) | internal | Returns the effective per-connection setting; lock poisoning is an `Err`. |
| `eql_mapper::copies_into_table` (`eql_mapper.rs` L103) | external, source available | Identifies exactly `COPY <table> FROM`; all other `COPY` forms return `None`. |
| `Context::has_encrypted_columns` (`context/mod.rs` L1748) | internal | `true` when the encrypt config is non-empty or the base schema has any non-`Native` column (`model/schema.rs` L146-L153). |
| `check_copy_into` (L1400) | internal | Sole guard for `COPY FROM`. |
| `TableResolver::resolve_table` (`model/table_resolver.rs` L96) | external, source available | Bare name found in both temporary and permanent layers is `AmbiguousTable` (L84-L88); otherwise single-layer lookup. |
| `Table::get_column` (`model/schema.rs` L277) | external, source available | Unique case-aware match or `ColumnNotFound`. |
| `eql_mapper::requires_type_check` (`eql_mapper.rs` L61) | external, source available | Fixed list of seven statement kinds. |
| `eql_mapper::writes_data` (`eql_mapper.rs` L76) | external, source available; traversal in `sqltk` not read | Detects any nested `Insert`/`Update`/`Delete`/`Merge`. |
| `Frontend::type_check` -> `eql_mapper::type_check` (`eql_mapper.rs` L35) | external, source available; inference not read | `Ok` only for a fully typed statement; an unexpected `Break` becomes `InternalError` (`eql_mapper.rs` L43-L45), which is a coverage gap. |
| `Frontend::to_encryptable_statement` (L909) | internal | `None` only when nothing is encrypted and no transform is needed; `Err` when config is missing for an EQL column. |
| `ColumnMapper::get_*_columns` (`column_mapper.rs` L32, L68, L133) | internal | Positional `Vec<Option<Column>>`; missing config is `UnknownColumn`. |
| `TypeCheckedStatement::requires_transform` (`type_checked_statement.rs` L150) | external, source available; transformer rules not read | Dry run reports whether the rewrite would edit anything. |
| `Frontend::mapping_error` (L1320) | internal | Classifies the mapper error; the default arm is a coverage gap. |
| `EqlMapperError::as_unmappable_encrypted_column` / `is_unsatisfied_bounds` / `as_unresolved_table` (`eql_mapper.rs` L164, L177, L193) | external, source available | Recognise refusals through specific wrapper nestings only. |
| `Frontend::may_fall_back` (L1300) | internal | The single fallback predicate. |
| `Error::is_coverage_gap` (`error.rs` L67) | internal | Allow-list of two variants. |
| `Context::mapping_errors_enabled` (`context/mod.rs` L1798) | internal | Operator config value. |

## 8. Shared state read

- `ConnectionProtocolState.settings` (read lock, `context/mod.rs` L1631-L1635).
- `SchemaMiddleware.encrypt_config`, `.base`, `.resolver`, `.temporary` (`middleware.rs` L373-L389).
- `TandemConfig.development.enable_mapping_errors` (`config/tandem.rs` L324-L329).
- Prometheus counter `STATEMENTS_UNMAPPABLE_TOTAL` (written, L1331, L1348, L1360, L1372).

## 9. Notes on comments that do not match the code

- L98 says "`type_check` already logged and counted the error". The logging and counting are in `mapping_error` (L1372-L1391); `type_check` (L1304-L1315) only emits a debug line on success.
- L69-L70 and L77 say `NothingEncrypted` is backed by a type check. L1261 returns it for `COPY FROM` with no type check.
- `ColumnMapper::get_column`'s doc says it returns `None` when mapping is disabled (`column_mapper.rs` L158-L160). The body returns `Err(UnknownColumn)` on every miss (`column_mapper.rs` L189-L200).

## 10. Open questions

- Unclear whether the `sqltk` visitor reaches a write inside `CopySource::Query`, `CREATE TABLE ... AS`, `CREATE VIEW`, `DECLARE CURSOR`, or a function body; need to inspect the `sqltk` 0.11 `Visitable` implementation for `Statement` and `CopySource`.
- Unclear whether Backend can change `SchemaMiddleware` state (`adopt_latest`, `apply_ddl`, temporary tables) on another task between the caller's `wait_for_schema_execution` and the reads inside `route`; need to inspect how Frontend and Backend are scheduled in `postgresql/driver.rs` and `middleware/mod.rs`.
- Unclear why `schema_has_encrypted_columns` reads `base` rather than the overlaid resolver; need to inspect `SchemaMiddleware::apply_ddl` and `apply_encrypt_config` to see whether a column made encrypted inside the current transaction is always reflected in `encrypt_config`.
- Unclear what `bind_handler` does when `knowledge.mapping_disabled` differs from the value `route` saw at Parse; need to inspect `bind_handler` around L1011.
- Unclear which type-check failures on reads of encrypted columns land in the default arm of `mapping_error`; need to inspect `TypeError`, `ScopeError` and `ImportError` variants and where each is raised.
- Unclear whether `TypeCheckedStatement::requires_transform` is true for every statement with an encrypted literal; need to inspect `make_transformer` and the transformation rules in `packages/eql-mapper/src/transformation_rules`.
- Unclear whether the parser's `LIKE`-as-column behaviour (L1436-L1439) is the only way a loaded table carries a placeholder column; need to inspect `collect_ddl_with_column_kind`.
- Unclear how `UnmappableEncryptedColumn` raised through `Scope(ScopeError::TypeError(_))` or `Import(ImportError::ScopeError(_))` is classified; need to inspect whether the importer or scope tracker can wrap that error in those shapes.
