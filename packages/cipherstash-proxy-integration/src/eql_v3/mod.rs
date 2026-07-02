//! EQL v3 test variants.
//!
//! GATED: every test in this module is `#[ignore = "blocked on eql-mapper v3"]`
//! because the proxy's eql-mapper cannot speak EQL v3 yet (that redesign is
//! out of scope here). The tests exist so the v3 SQL surface is documented and
//! compile-checked, and so they can be switched on when the mapper lands.
//!
//! Running them requires the EQL v3 fixture in place of the default v2 fixture:
//!
//! ```shell
//! CS_EQL_V3_PATH=../encrypt-query-language/release mise run postgres:setup:v3
//! cargo nextest run -p cipherstash-proxy-integration -E 'test(eql_v3)' --run-ignored all
//! mise run postgres:setup   # restore the v2 fixture afterwards
//! ```
//!
//! The bulk of the integration suite is intentionally NOT duplicated here: it
//! rides on the fixture and the mapper, and will be enabled wholesale by the
//! eql-mapper v3 project. These modules cover only the payload/SQL-surface
//! coupled minority whose shape changes between v2 and v3:
//!
//! * `disable_mapping`   - raw column values are v3 envelopes (`v: 3`, no `k`)
//! * `indexing`          - on-column operator class -> functional term index
//! * `jsonb_containment` - `eql_v2.jsonb_contains()` -> `@>` on `eql_v3.json`
//! * `match_index`       - LIKE/ILIKE -> bloom containment (`@>` / `<@`)
//! * `regression_cast`   - `::eql_v2_encrypted` -> per-domain casts

mod disable_mapping;
mod indexing;
mod jsonb_containment;
mod match_index;
mod regression_cast;
