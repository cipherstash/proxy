use super::eql_domains;
use crate::config::DatabaseConfig;
use crate::error::Error;
use crate::proxy::{AGGREGATE_QUERY, SCHEMA_QUERY};
use crate::{connect, log::SCHEMA};
use arc_swap::ArcSwap;
use eql_mapper::{Column, Schema, Table};
use sqltk::parser::ast::Ident;
use std::sync::Arc;
use std::time::Duration;
use tokio::{task::JoinHandle, time};
use tracing::{debug, info, warn};

#[derive(Clone, Debug)]
pub struct SchemaManager {
    config: DatabaseConfig,
    schema: Arc<ArcSwap<Schema>>,
    _reload_handle: Arc<JoinHandle<()>>,
}

impl SchemaManager {
    pub async fn init(config: &DatabaseConfig) -> Result<Self, Error> {
        let config = config.clone();
        init_reloader(config).await
    }

    pub fn load(&self) -> Arc<Schema> {
        self.schema.load().clone()
    }

    pub async fn reload(&self) {
        match load_schema_with_retry(&self.config).await {
            Ok(reloaded) => {
                debug!(target: SCHEMA, msg = "Reloaded database schema");
                self.schema.swap(Arc::new(reloaded));
            }
            Err(err) => {
                warn!(
                    msg = "Error reloading database schema",
                    error = err.to_string()
                );
            }
        };
    }
}

async fn init_reloader(config: DatabaseConfig) -> Result<SchemaManager, Error> {
    // Skip retries on startup as the likely failure mode is configuration
    let schema = load_schema(&config).await?;
    info!(msg = "Loaded database schema");

    let schema = Arc::new(ArcSwap::new(Arc::new(schema)));

    let config_ref = config.clone();
    let schema_ref = schema.clone();

    let reload_handle = tokio::spawn(async move {
        let reload_interval = tokio::time::Duration::from_secs(config_ref.config_reload_interval);

        let mut interval = tokio::time::interval_at(
            tokio::time::Instant::now() + reload_interval,
            reload_interval,
        );

        loop {
            interval.tick().await;

            match load_schema_with_retry(&config_ref).await {
                Ok(reloaded) => {
                    schema_ref.swap(Arc::new(reloaded));
                }
                Err(err) => {
                    warn!(
                        msg = "Error loading database schema",
                        error = err.to_string()
                    );
                }
            }
        }
    });

    Ok(SchemaManager {
        config,
        schema,
        _reload_handle: Arc::new(reload_handle),
    })
}

/// Fetch the dataset and retry on any error
///
/// When databases and the proxy start up at the same time they might not be ready to accept connections before the
/// proxy tries to query the schema. To give the proxy the best chance of initialising correctly this method will
/// retry the query a few times before passing on the error.
async fn load_schema_with_retry(config: &DatabaseConfig) -> Result<Schema, Error> {
    let mut retry_count = 0;
    let max_retry_count = 10;
    let max_backoff = Duration::from_secs(2);

    loop {
        match load_schema(config).await {
            Ok(schema) => {
                return Ok(schema);
            }

            Err(e) => {
                if retry_count >= max_retry_count {
                    return Err(e);
                }
            }
        }

        let sleep_duration_ms = (100 * 2_u64.pow(retry_count)).min(max_backoff.as_millis() as _);

        time::sleep(Duration::from_millis(sleep_duration_ms)).await;

        retry_count += 1;
    }
}

/// The legacy EQL v2 encrypted column type.
const EQL_V2_ENCRYPTED_TYPE: &str = "eql_v2_encrypted";

/// Whether a column is declared with the legacy EQL v2 encrypted type.
///
/// Both catalog columns are checked because the two shapes EQL v2 shipped land
/// in different places in `information_schema.columns`:
///
/// - as a composite type (what EQL v2 installs), `udt_name` is
///   `eql_v2_encrypted` and `domain_name` is NULL;
/// - as a DOMAIN, `udt_name` is the base type (`jsonb`) and only `domain_name`
///   carries `eql_v2_encrypted`.
///
/// Checking `udt_name` alone — as this loader previously did — silently misses
/// the domain shape, and a missed v2 column is precisely a plaintext column.
fn is_legacy_eql_v2(column_type_name: Option<&str>, column_domain_name: Option<&str>) -> bool {
    column_type_name == Some(EQL_V2_ENCRYPTED_TYPE)
        || column_domain_name == Some(EQL_V2_ENCRYPTED_TYPE)
}

/// Decides what a single catalog row means for the type checker.
///
/// Split out from [`load_schema`] so the classification — the security-relevant
/// part — is testable without a database.
fn classify_column(
    table_name: &str,
    column_name: &str,
    column_type_name: Option<&str>,
    column_domain_name: Option<&str>,
) -> Column {
    let ident = Ident::with_quote('"', column_name);

    // Prefer the v3 domain: encrypted columns are jsonb-backed DOMAINs whose
    // typname encodes the token type and capabilities. The domain identity and
    // traits are read from the eql-bindings catalog (ADR-0002).
    if let Some((identity, eql_traits)) = column_domain_name.and_then(eql_domains::resolve) {
        debug!(target: SCHEMA, msg = "eql_v3 column", table = table_name, column = column_name, domain = %identity.domain.value, traits = %eql_traits);
        return Column::eql(ident, eql_traits, identity);
    }

    // Legacy EQL v2 columns have no v3 domain identity, so this v3-only build
    // can neither encrypt writes to them nor decrypt reads from them.
    //
    // They are NOT served as native (plaintext) columns. That was the CIP-3688
    // defect: a partially-completed migration left one column behind, and Proxy
    // silently accumulated plaintext in it, with nothing but a startup log line
    // to say so. The column is marked unmappable instead, which makes the type
    // checker refuse every statement referencing the table — failing closed, and
    // naming the column that needs migrating.
    if is_legacy_eql_v2(column_type_name, column_domain_name) {
        warn!(target: SCHEMA, msg = "Column is declared with the legacy EQL v2 encrypted type, which this EQL v3 build cannot encrypt or decrypt. Statements referencing this table will be REFUSED so that plaintext is never written to the column. Migrate the column to an EQL v3 domain type.", table = table_name, column = column_name);
        return Column::unmappable_encrypted(ident, EQL_V2_ENCRYPTED_TYPE);
    }

    // Any other unrecognised type is an ordinary plaintext column.
    Column::native(ident)
}

pub async fn load_schema(config: &DatabaseConfig) -> Result<Schema, Error> {
    let client = connect::database(config).await?;

    let tables = client.query(SCHEMA_QUERY, &[]).await?;

    let mut schema = Schema::new("public");

    if tables.is_empty() {
        warn!(msg = "Database schema contains no tables");
        return Ok(schema);
    };

    for table in tables {
        let table_name: String = table.get("table_name");
        let columns: Vec<String> = table.get("columns");
        let column_type_names: Vec<Option<String>> = table.get("column_type_names");
        let column_domain_names: Vec<Option<String>> = table.get("column_domain_names");

        let mut table = Table::new(Ident::new(&table_name));

        columns
            .iter()
            .zip(column_type_names)
            .zip(column_domain_names)
            .for_each(|((col, column_type_name), column_domain_name)| {
                let column = classify_column(
                    &table_name,
                    col,
                    column_type_name.as_deref(),
                    column_domain_name.as_deref(),
                );

                table.add_column(Arc::new(column));
            });

        schema.add_table(table);
    }

    let aggregates = client.query(AGGREGATE_QUERY, &[]).await?;
    schema.aggregates = aggregates
        .into_iter()
        .map(|r| {
            let name: String = r.get("name");
            Arc::new(name)
        })
        .collect();

    Ok(schema)
}

#[cfg(test)]
mod test {
    use super::*;
    use eql_mapper::ColumnKind;

    /// The shape `information_schema.columns` reports for a column declared with
    /// the EQL v2 composite type, verified against PostgreSQL 17: `udt_name` is
    /// the type name, `domain_name` is NULL.
    const V2_COMPOSITE: (Option<&str>, Option<&str>) = (Some("eql_v2_encrypted"), None);

    /// The same column had EQL v2 shipped `eql_v2_encrypted` as a DOMAIN over
    /// jsonb: `udt_name` is the base type, `domain_name` carries the type name.
    const V2_DOMAIN: (Option<&str>, Option<&str>) = (Some("jsonb"), Some("eql_v2_encrypted"));

    fn kind(column_type_name: Option<&str>, column_domain_name: Option<&str>) -> ColumnKind {
        classify_column("users", "secret", column_type_name, column_domain_name).kind
    }

    #[test]
    fn legacy_v2_composite_column_is_never_native() {
        // The regression this pins: `Native` here is a plaintext passthrough, so
        // classifying a v2 column that way makes Proxy write plaintext into a
        // column its operator believes is encrypted (CIP-3688). Assert the exact
        // kind rather than `!= Native` so a future third "just serve it" kind
        // cannot quietly take its place either.
        assert_eq!(
            kind(V2_COMPOSITE.0, V2_COMPOSITE.1),
            ColumnKind::UnmappableEncrypted("eql_v2_encrypted".to_string())
        );
    }

    #[test]
    fn legacy_v2_domain_column_is_never_native() {
        // The loader originally keyed only on `udt_name`, which misses this
        // shape entirely — and a missed v2 column is a plaintext column.
        assert_eq!(
            kind(V2_DOMAIN.0, V2_DOMAIN.1),
            ColumnKind::UnmappableEncrypted("eql_v2_encrypted".to_string())
        );
    }

    #[test]
    fn v3_domain_columns_still_resolve_to_eql() {
        assert!(matches!(
            kind(Some("jsonb"), Some("eql_v3_text_search")),
            ColumnKind::Eql(_, _)
        ));
    }

    #[test]
    fn ordinary_columns_are_still_native() {
        assert_eq!(kind(Some("text"), None), ColumnKind::Native);
        assert_eq!(kind(Some("int4"), None), ColumnKind::Native);
        // An unrecognised domain is a plaintext column, not a refusal: refusing
        // every user-defined domain would be a very different change.
        assert_eq!(
            kind(Some("text"), Some("domain_type_with_check")),
            ColumnKind::Native
        );
        // Only the exact v2 type name refuses; a lookalike does not.
        assert_eq!(
            kind(Some("eql_v2_encrypted_backup"), None),
            ColumnKind::Native
        );
    }
}
