# Frontend::transform_statement

- File: `packages/cipherstash-proxy/src/postgresql/middleware/frontend.rs`
- Lines: L718-L764
- Signature: `fn transform_statement(&mut self, typed_statement: &TypeCheckedStatement<'_>, encrypted_literals: &Vec<Option<EqlOutput>>) -> Result<Option<eql_mapper::TransformedStatement>, Error>`
- Callers: `Frontend::query_handler` at L509 and `Frontend::parse_handler` at L850.

Paths are relative to `packages/cipherstash-proxy/src/` unless they start with `packages/`.
The format reference files named in the task (`ANALYSIS_FORMAT.md` and siblings) were not found on disk, so this document uses the same section layout as the earlier files in this directory.

## 0. Correction to the selection premise

The task states that this function "relies on quote_literal for embedding".
It does not.
`quote_literal` is defined at frontend.rs L1480-L1487 and has exactly two call sites, L1722 and L1728, both inside `Frontend::simple_error_query` (L1703-L1729).
Neither `transform_statement` nor any callee read for this analysis calls it.
The encrypted payload is embedded as an AST node, `sqltk::parser::ast::Value::SingleQuotedString` (L733, L1692), and the quoting happens later when a caller renders the AST with `Display` (L574 via `SqlParser::statement_sql`, L867 via `to_string()`).
The component that turns the string into a quoted SQL literal is therefore the `sqltk-parser` crate's `Display` impl, whose source is not on disk (section 5.4).

## 1. Purpose

Builds a map from literal AST node to replacement `ast::Value` and hands it to `TypeCheckedStatement::transform`, which returns a rewritten AST plus a `ParamPlan`.
The function produces an AST, not SQL text.
It performs no encryption, no I/O, and no write to connection state.
`self` is used once, to read `self.context.client_id` for a debug log (L751); the `&mut` receiver is not exercised.

## 2. Inputs and who controls them

| Input | Origin | Controlled by |
|---|---|---|
| `typed_statement.statement` | Client SQL parsed by `SqlParser::parse_statements` (L411) or `parse_statement` (L817), type checked by `eql_mapper::type_check` (L1309) | Authenticated client, within what the parser and type checker accept |
| `typed_statement.literals` | `EqlMapper::literal_types` (packages/eql-mapper/src/eql_mapper.rs L355-L378): every registered `ast::Value` that is not a `Placeholder` (L361) and whose resolved type is `Value::Eql` (L368-L370) | Client SQL plus the schema snapshot |
| `encrypted_literals` | Return of `Frontend::encrypt_literals` (L631-L711) | See section 2.1 |
| `typed_statement.node_types` and the rule set | Fixed at type check; consumed inside `transform` (packages/eql-mapper/src/type_checked_statement.rs L312-L343) | eql-mapper |

### 2.1 What an `encrypted_literals` entry can hold

`encrypt_literals` returns `vec![]` when there are no literals (L638-L644).
Otherwise each position is one of:

- `None`: no plaintext was produced. `literal_from_sql` returns `None` for `Value::Null` (postgresql/data/from_sql.rs L73), and `json_value_selector_literal_plaintext` returns `Ok(None)` when `literal_json_value` returns `None` (frontend.rs L1638-L1640, from_sql.rs L205).
- `Some(EqlOutput::Store(_))` or `Some(EqlOutput::Query(_))` produced by the encryption service through `Context::encrypt` (context/mod.rs L1647-L1675), then possibly projected by `project_query_operand` (frontend.rs L1522-L1533, applied at L677-L686).
- `Some(EqlOutput::Query(q))` where `q` is a client-supplied payload that `inbound_eql::parse` classified as query shaped (postgresql/inbound_eql.rs L36-L48, L67-L83). `merge_inbound_eql` writes it into the slot with no further processing (frontend.rs L1165-L1167).
- `Some(EqlOutput::Store(ciphertext))` where `ciphertext` is the client-supplied `EqlCiphertext` after decryption and SEM-term comparison succeed (frontend.rs L1181-L1238). The value stored is the client's deserialized object, not the re-derived one (L1238).

The last two bullets mean the JSON text that `transform_statement` embeds is, on those paths, a re-serialization of an object deserialized from client bytes.
Which string-typed fields survive that round trip depends on the `EqlCiphertextV3` and `EqlQueryPayloadV3` definitions in `cipherstash-client`, re-exported at lib.rs L24-L25; that source is not on disk.

## 3. Block-by-block

### 3.1 Literal to `ast::Value` conversion (L724-L739)

One output entry per input entry, same order.

- L732-L734: `Some(EqlOutput::Query(EqlQueryPayload::Selector(s)))` becomes `Value::SingleQuotedString(s.clone())`. The string is used verbatim. No check of `s` is made here.
- L735: every other `Some(en)` becomes `to_json_literal_value(&en)?`. That helper (L1688-L1693) calls `serde_json::to_string` and wraps the result in `Value::SingleQuotedString`. A serialization failure returns `Err` through the `?` on L1692 and L735; the conversion into `Error` is a `From<serde_json::Error>` impl not inspected here.
- L736: `None` stays `None`.

`to_json_literal_value` receives `&&EqlOutput` (L735 passes `&en` where `en: &EqlOutput`), so the JSON is whatever `EqlOutputV3`'s `Serialize` impl emits for the enum itself.
The same serialization is used for bind params at postgresql/rewrite/bind.rs L338.
Both branches produce the same AST variant, `SingleQuotedString`; the selector branch differs only in skipping JSON encoding (comment at L727-L731, mirrored at bind.rs L325-L335).

### 3.2 Pairing with AST nodes (L743-L748)

`typed_statement.literals.iter().zip(encrypted_expressions)` pairs by position.
`zip` stops at the shorter side, so a length difference is not an error at this line.
`filter_map` drops `None` entries (L747).
The key is `NodeKey::new(*original_node)` (L747), an identity key for the borrowed AST node.
Results are collected into a `HashMap` (L748); two entries with the same key collapse to one without an error at this line.

Positional agreement between `typed_statement.literals` and `encrypted_literals` is not established in this function.
It rests on three facts in other functions:

- `encrypt_literals` iterates `typed_statement.literal_values()` (L637), which returns `&self.literals` (type_checked_statement.rs L261-L263), the same `Vec`.
- `ColumnMapper::get_literal_columns` iterates the same `Vec` (postgresql/column_mapper.rs L139) and pushes one entry per literal, because `get_column` returns `Ok(Some(_))` or `Err` and never `Ok(None)` (column_mapper.rs L166-L200); the `if col.is_some()` filter at L150 is therefore never false on the `Ok` path.
- `Context::encrypt` returns a vector whose length equals `plaintexts.len()` on both of its `Ok` paths (context/mod.rs L1652-L1656 and L1667-L1674), and `ZeroKms::encrypt` places outputs by recorded index (proxy/zerokms/zerokms.rs L360, L497-L514).

Both callers pass the `typed_statement` and the `literal_columns` from the same `Route::Map` value (L482-L485 with L500-L509; L835-L838 with L845-L850).

### 3.3 Debug log (L750-L753)

Logs `client_id` and the map size only.
No literal content is logged in this function.
The callers log the full transformed statement at debug level (L511-L514, L580-L587, L852-L856).

### 3.4 `requires_transform` gate (L755-L757)

`requires_transform` (type_checked_statement.rs L150-L155) builds the rule tuple with `"DUMMY"` literals for every entry of `self.literals` (L265-L275), runs it in dry mode, discards the `Result` of `apply_transform` (`let _ =`, L152), and returns `did_edit()`.
Both callers already call `requires_transform()` on the same `typed_statement` and only enter the branch when it is true (frontend.rs L492, L839).
The inputs to the dry run are identical between the two calls, so the `Ok(None)` return at L756 is not reached from either current caller.
The gate runs after L724-L748, so a serialization error at L735 is returned even for a statement the gate would have declined.

The caller-side consequence of `requires_transform() == false` is that the original statement is forwarded unchanged (L524-L526 for Query; `message.query` left untouched for Parse, L839-L870).
In dry mode `SubstituteEncryptedLiterals::would_edit` returns true only when the visited node path ends in `Expr::Value` and the node's key is in the map (substitute_encrypted_literals.rs L57-L62).

### 3.5 `transform` call (L759-L763)

`typed_statement.transform(encrypted_nodes)` consumes the map.
Any `EqlMapperError` is converted to `MappingError::StatementCouldNotBeTransformed(e.to_string())` (L761; variant at error.rs L175-L176).
On success the `TransformedStatement { statement, params }` is returned unchanged (L763).

## 4. What `TypeCheckedStatement::transform` establishes

Source: packages/eql-mapper/src/type_checked_statement.rs L164-L182.

### 4.1 Pre-check (L168, L277-L297)

- L281: `count_not_null_literals() != encrypted_literals.len()` returns `Err`. `count_not_null_literals` counts entries of `self.literals` whose AST variant is not `Value::Null` (L305-L310).
- L289-L295: every key in the supplied map must be the node key of some entry in `self.literals`.

Together with the map being keyed by distinct nodes, these two checks mean that on the `Ok` path the map covers exactly the non-`Null` EQL-typed literals.
This is the only place the "every literal that needed a ciphertext got one" property is checked for `transform_statement`; frontend.rs L743-L748 silently drops `None` entries and silently truncates on length mismatch.
A non-`Null` literal whose slot is `None`, a short `encrypted_literals`, and a duplicate node key all surface here as a count mismatch.
The error text at L282-L286 prints `self.literals.len()`, not the non-null count that was compared.

### 4.2 Rule application (L169-L171)

`make_transformer` (L312-L343) builds a fixed tuple; `SubstituteEncryptedLiterals` is first.
`SubstituteEncryptedLiterals::apply` (substitute_encrypted_literals.rs L34-L55):

- L39-L42: acts only when the node path's last element is an `Expr` of variant `Expr::Value`.
- L44-L46: removes the replacement from its map by the key of the inner `Value`; no entry means no edit.
- L48-L52: overwrites the target with `Expr::Value` holding the replacement and `Span::empty()`. The `downcast_mut::<Expr>().unwrap()` at L48 panics if the target is not an `Expr`.

The rule defines a postcondition, "no unused encrypted literals" (substitute_encrypted_literals.rs L64-L73).
A search of `packages/` for `check_postcondition` finds the trait default (transformation_rules/mod.rs L92), the tuple impl that forwards to its members (mod.rs L204-L205), and this one implementation (substitute_encrypted_literals.rs L64).
No call site invokes it on the transformer built at type_checked_statement.rs L169, and `DryRunnable`'s `Transform` impl (mod.rs L171-L182) does not call it.
So the property "every entry of the supplied map was substituted into the AST" has a written check that nothing found executes.
What remains is the count and key check of 4.1, which runs before substitution and says nothing about whether the traversal reached each literal through an `Expr::Value` path.

Later rules wrap operands in casts.
`cast_encrypted_operand` (transformation_rules/helpers.rs L105-L133) casts only when the original node is `Expr::Value` (L111) with an EQL type in `node_types` (L115) and the domain function returns `Some` (L119).
`cast_expr_to_v3_domain` (helpers.rs L136-L154) builds `<expr>::JSONB::<schema>.<domain>` with `Ident::new(schema)` and `Ident::new(domain)` (L145-L146), which carry no quote style.
For a full payload the domain string is `eql_term.eql_value().domain_identity().domain.value` (helpers.rs L77), a name taken from the schema model.
For `JsonAccessor` and `JsonPath` both domain functions return `None` (helpers.rs L53, L74), so a selector literal is left as a bare string literal with no cast.

### 4.3 Renumbering and param plan (L176-L181)

`RenumberParams` runs as a second pass (L176-L177); `param_plan` (L195-L259) fails with `InternalError` when the rewritten statement refers to an undeclared param (L208-L212) and calls `plan.check_covers(&inputs)` (L256).
For the simple-protocol caller the plan is discarded (frontend.rs L516-L518).
For the Parse caller it replaces `statement.output_params` (frontend.rs L861-L865).

### 4.4 Error paths

Every `?` in `transform` (L168, L171, L177, L179) reaches frontend.rs L761 and becomes `StatementCouldNotBeTransformed`.
`transform` takes `&self` and returns a new `Statement`; the original AST behind `typed_statement.statement` is not mutated (type_checked_statement.rs L53, L171).

## 5. How the returned AST becomes SQL text (callers)

This is outside L718-L764 but it is where the embedded strings become bytes sent upstream.

### 5.1 Simple protocol (query_handler)

- L518: the transformed `Statement` is pushed into `forwarded_statements`; `encrypted = true`.
- L524-L526: a mapped statement that was not transformed is pushed as a clone of the original AST.
- L536-L539: forwarded statements are pushed as clones of the parsed AST.
- L571-L578: when `encrypted` is true, every entry of `forwarded_statements` is rendered with `SqlParser::statement_sql`, joined with `";"`, and sent as one `Query`.

So one transformed statement causes every statement in the batch to be re-rendered from its AST, including those the mapper did not touch (comment at L454-L455).
`SqlParser::statement_sql` (postgresql/parser.rs L40-L69) returns `statement.to_string()` for everything except three `SET` shapes that encode a `RESET`.
One of those shapes returns the content of a `SingleQuotedString` raw as SQL (parser.rs L58-L66); it requires an empty span on the value and a variable name equal to `RESET_PLACEHOLDER` with an empty span.
`SubstituteEncryptedLiterals` also gives its replacement an empty span (substitute_encrypted_literals.rs L49-L52), but only under an `Expr::Value` node reached by the transformer, and the variable-name condition at parser.rs L62-L63 is independent of it.

### 5.2 Extended protocol (parse_handler)

- L867-L868: `transformed_statement.statement.to_string()` replaces `message.query`. `statement_sql` is not used on this path.

### 5.3 Session settings at render time

The JSON text contains backslash escapes whenever `serde_json` emits one, and it is rendered as an ordinary `'...'` literal, not an `E'...'` literal (variant chosen at L733 and L1692).
How PostgreSQL reads backslashes inside `'...'` depends on the upstream session's `standard_conforming_strings` setting.
A search of `packages/` for `standard_conforming` returns no match, so nothing found pins or inspects that setting.

### 5.4 The quoting itself

`Value::SingleQuotedString`'s `Display` impl lives in `sqltk-parser` 0.56.0-cipherstash.3 (Cargo.lock L4601-L4602).
No copy of that crate's source was found under `~/.cargo/registry/src`.
What it does with `'`, with `\`, and with sequences such as `''` or `\'` already present in the string is not established by anything read here.

## 6. Invariants

1. On `Ok(Some(_))`, the map passed to `transform` contains exactly one entry per non-`Null` entry of `typed_statement.literals`. Evidence: type_checked_statement.rs L281 and L289-L295, reached from frontend.rs L760.
2. Every replacement value is `ast::Value::SingleQuotedString`. Evidence: frontend.rs L733 and L1692; no other variant is constructed in L724-L739.
3. A selector payload is embedded without JSON encoding; every other payload is embedded as `serde_json::to_string` output. Evidence: frontend.rs L732-L735.
4. The function writes no connection or process state. Evidence: the only use of `self` is the read at L751.
5. Any failure inside `transform` is returned as `MappingError::StatementCouldNotBeTransformed`; no partially rewritten statement is returned. Evidence: frontend.rs L759-L763.
6. The original AST is not mutated. Evidence: `transform(&self, ..)` returns a new `Statement` at type_checked_statement.rs L171-L181.

## 7. Assumptions

| # | Assumption | Established by |
|---|---|---|
| A1 | `encrypted_literals[i]` belongs to `typed_statement.literals[i]` | Not in this function. Shared iteration of `self.literals` at frontend.rs L637 and column_mapper.rs L139, plus length preservation at context/mod.rs L1652-L1656, L1667-L1674 and zerokms.rs L497-L514. A length error is caught at type_checked_statement.rs L281; a same-length permutation is not checked anywhere |
| A2 | Rendering a `SingleQuotedString` produces a SQL literal whose value equals the Rust string, for any content | `sqltk-parser` `Display`; source not available. Nothing found in this repository |
| A3 | The upstream session reads backslashes inside `'...'` literally | Nothing found (no reference to `standard_conforming_strings` in `packages/`) |
| A4 | A `Selector` string is safe to embed verbatim and is the 32-hex token the EQL functions expect | Inbound path only: `is_selector_hash` at inbound_eql.rs L41 and L173. For selectors returned by the encryption service (zerokms.rs L269-L288, L354-L360) nothing found in this crate |
| A5 | `EqlOutputV3` serializes to the bare EQL payload object | `cipherstash-client` `Serialize` impl; source not available |
| A6 | Every literal in the map is reached by the transformer under an `Expr::Value` path and substituted | Count and key check at type_checked_statement.rs L281-L295 runs before substitution. The post-substitution check at substitute_encrypted_literals.rs L64-L73 has no caller found |
| A7 | `requires_transform()` is true whenever a literal needs replacing, so the callers enter the branch that calls this function | Dry run at type_checked_statement.rs L150-L155, which depends on `would_edit` at substitute_encrypted_literals.rs L57-L62 and discards the `apply_transform` result at L152 |
| A8 | Payload string fields that originate from the client (inbound EQL) are constrained before embedding | Structural validation at inbound_eql.rs L94-L180 and authentication at frontend.rs L1185-L1237 for `Store`; for `Query` payloads only inbound_eql.rs L127-L180. Field types are defined in `cipherstash-client`; not inspected |
| A9 | Cast targets built from the schema model render as valid identifiers | `Ident::new` with no quote style at helpers.rs L145-L146; the domain name source is helpers.rs L77. Nothing found that constrains the name's characters |
| A10 | Re-rendering untouched batch statements from the AST yields SQL equivalent to what the client sent | `sqltk-parser` `Display` round trip; COPY is excluded at frontend.rs L456-L462. Nothing else found |

## 8. Calls

| Callee | Kind | What the caller depends on |
|---|---|---|
| `to_json_literal_value` (frontend.rs L1688-L1693) | internal | Returns `SingleQuotedString(json)`; propagates serde errors |
| `serde_json::to_string` on `EqlOutputV3` | external, source not on disk | A5 |
| `NodeKey::new` (sqltk) | external, source not on disk | Identity keying consistent with `as_node_key()` used at type_checked_statement.rs L302 |
| `TypeCheckedStatement::requires_transform` (type_checked_statement.rs L150-L155) | external, source available | A7 |
| `TypeCheckedStatement::transform` (type_checked_statement.rs L164-L182) | external, source available | Invariant 1, substitution, casts, renumbering, `ParamPlan` |
| `SubstituteEncryptedLiterals::apply` (substitute_encrypted_literals.rs L34-L55) | external, source available | A6 |
| `Statement::apply_transform` and AST `Display` (sqltk, sqltk-parser) | external, source not on disk | Traversal reaches literals; A2, A10 |

## 9. Shared state

None read or written beyond `self.context.client_id` (L751).

## 10. Open questions

- Unclear how `Value::SingleQuotedString` is rendered for content containing `'`, `\`, `''` or `\'`; need to inspect `sqltk-parser` 0.56.0-cipherstash.3 `src/ast/value.rs`.
- Unclear which fields of `EqlCiphertextV3` and `EqlQueryPayloadV3` are free-form strings and whether unknown fields are retained on deserialization; need to inspect `cipherstash-client` 0.42.2.
- Unclear what `EqlOutputV3`'s `Serialize` impl emits for each variant; need to inspect `cipherstash-client` 0.42.2.
- Unclear whether the encryption service can return a `Selector` string outside `[0-9a-f]{32}`; need to inspect `encrypt_eql_v3` with `QueryOp::SteVecSelector` in `cipherstash-client`.
- Unclear whether `check_postcondition` is invoked through a path not visible to a text search (macro-generated or in `sqltk`); need to inspect `sqltk` 0.11.0 `apply_transform` and the `impl_for_tuples` expansion.
- Unclear whether every EQL-typed `ast::Value` registered by the type inferencer sits under an `Expr::Value` node; need to inspect `registry.get_nodes_and_types::<ast::Value>()` and the AST positions where `Value` appears outside `Expr`.
- Unclear whether `typed_statement.literals` can contain the same node twice; need to inspect the registry behind eql_mapper.rs L357-L361.
- Unclear what characters a domain name in the schema model can contain when it reaches helpers.rs L145-L146; need to inspect the schema loader in proxy/schema/manager.rs and `domain_identity()`.
- Unclear whether the upstream session's `standard_conforming_strings` can differ from `on` on a Proxy connection; need to inspect connection startup parameters and whether client `SET` of that parameter is forwarded.
- `RenumberParams`, `FailOnPlaceholderChange`, `ParamPlan::check_covers` and the remaining rewrite rules were not read line by line; need to inspect them before relying on the `ParamPlan` returned at L763.
