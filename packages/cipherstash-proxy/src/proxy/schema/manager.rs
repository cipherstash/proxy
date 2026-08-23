use super::eql_domains;
use crate::config::DatabaseConfig;
use crate::error::Error;
use crate::proxy::encrypt_config::from_domain::column_config_from_domain;
use crate::proxy::EncryptConfig;
use crate::proxy::{AGGREGATE_QUERY, SCHEMA_QUERY};
use crate::{connect, log::SCHEMA};
use arc_swap::ArcSwap;
use cipherstash_client::eql::Identifier;
use eql_mapper::{Column, Schema, Table};
use sqltk::parser::ast::Ident;
use std::future::Future;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::{sync::Mutex, task::JoinHandle, time};
use tracing::{debug, info, warn};

#[derive(Clone, Debug)]
/// An immutable, atomically published schema and encryption-metadata generation.
pub struct CommittedSchemaSnapshot {
    version: u64,
    schema: Arc<Schema>,
    encrypt_config: Arc<EncryptConfig>,
}

#[derive(Clone, Debug)]
/// Shared access to committed snapshots and their publication generations.
pub struct CommittedSchemaStore {
    snapshot: Arc<ArcSwap<CommittedSchemaSnapshot>>,
    requested_publication: Arc<AtomicU64>,
    published_publication: Arc<AtomicU64>,
}

impl CommittedSchemaStore {
    /// Creates the initial committed generation from aligned schema metadata.
    pub(crate) fn from_parts(schema: Schema, encrypt_config: EncryptConfig) -> Self {
        Self {
            snapshot: Arc::new(ArcSwap::new(Arc::new(CommittedSchemaSnapshot::new(
                1,
                schema,
                encrypt_config,
            )))),
            requested_publication: Arc::new(AtomicU64::new(0)),
            published_publication: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Loads one internally consistent committed snapshot.
    pub fn load(&self) -> Arc<CommittedSchemaSnapshot> {
        self.snapshot.load().clone()
    }

    /// Returns whether a requested publication has not yet completed.
    pub fn publication_pending(&self) -> bool {
        self.requested_publication.load(Ordering::Acquire)
            > self.published_publication.load(Ordering::Acquire)
    }

    /// Advances the requested publication generation.
    pub fn mark_publication_pending(&self) {
        self.requested_publication.fetch_add(1, Ordering::AcqRel);
    }

    /// Marks all currently requested publication generations as satisfied.
    pub(crate) fn publication_succeeded(&self) {
        self.published_publication.store(
            self.requested_publication.load(Ordering::Acquire),
            Ordering::Release,
        );
    }

    #[cfg(test)]
    /// Creates a committed store without connecting to PostgreSQL.
    pub fn for_testing(schema: Schema, encrypt_config: EncryptConfig) -> Self {
        Self::from_parts(schema, encrypt_config)
    }

    #[cfg(test)]
    /// Publishes an aligned test snapshot as the next version.
    pub fn publish_for_testing(&self, schema: Schema, encrypt_config: EncryptConfig) {
        let version = self.load().version() + 1;
        self.snapshot.store(Arc::new(CommittedSchemaSnapshot::new(
            version,
            schema,
            encrypt_config,
        )));
        self.publication_succeeded();
    }
}

impl CommittedSchemaSnapshot {
    /// Creates an immutable snapshot at an explicit monotonic version.
    fn new(version: u64, schema: Schema, encrypt_config: EncryptConfig) -> Self {
        Self {
            version,
            schema: Arc::new(schema),
            encrypt_config: Arc::new(encrypt_config),
        }
    }

    /// Returns the monotonic snapshot version.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Returns the structural schema from this generation.
    pub fn schema(&self) -> Arc<Schema> {
        self.schema.clone()
    }

    /// Returns encryption metadata from the same generation as the schema.
    pub fn encrypt_config(&self) -> Arc<EncryptConfig> {
        self.encrypt_config.clone()
    }
}

#[derive(Clone, Debug)]
pub struct SchemaManager {
    config: DatabaseConfig,
    snapshot: Arc<ArcSwap<CommittedSchemaSnapshot>>,
    requested_generation: Arc<AtomicU64>,
    requested_publication: Arc<AtomicU64>,
    published_publication: Arc<AtomicU64>,
    reload_lock: Arc<Mutex<()>>,
    _reload_handle: Arc<JoinHandle<()>>,
}

impl SchemaManager {
    pub async fn init(config: &DatabaseConfig) -> Result<Self, Error> {
        let config = config.clone();
        init_reloader(config).await
    }

    /// Loads the current atomic committed snapshot.
    pub fn load(&self) -> Arc<CommittedSchemaSnapshot> {
        self.snapshot.load().clone()
    }

    /// Returns a cloneable store for per-connection schema middleware.
    pub fn store(&self) -> CommittedSchemaStore {
        CommittedSchemaStore {
            snapshot: self.snapshot.clone(),
            requested_publication: self.requested_publication.clone(),
            published_publication: self.published_publication.clone(),
        }
    }

    pub async fn reload(&self) -> bool {
        coalesced_reload(
            self.snapshot.clone(),
            self.requested_generation.clone(),
            self.requested_publication.clone(),
            self.published_publication.clone(),
            self.reload_lock.clone(),
            || load_snapshot_with_retry(&self.config),
        )
        .await
    }
}

/// Coalesces concurrent reload requests while preserving generation ordering.
async fn coalesced_reload<F, Fut>(
    snapshot: Arc<ArcSwap<CommittedSchemaSnapshot>>,
    requested_generation: Arc<AtomicU64>,
    requested_publication: Arc<AtomicU64>,
    published_publication: Arc<AtomicU64>,
    reload_lock: Arc<Mutex<()>>,
    load: F,
) -> bool
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(Schema, EncryptConfig), Error>>,
{
    let requested = requested_generation.fetch_add(1, Ordering::AcqRel) + 1;
    let _guard = reload_lock.lock().await;
    let publication_generation = requested_publication.load(Ordering::Acquire);

    if snapshot.load().version() >= requested {
        published_publication.fetch_max(publication_generation, Ordering::AcqRel);
        return true;
    }

    // A catalog read can only satisfy requests already visible when the read
    // begins. A request arriving while it is in progress must trigger a later
    // read, because PostgreSQL may have committed that DDL after this read's
    // transaction snapshot was established.
    let loaded_generation = requested_generation.load(Ordering::Acquire);

    match load().await {
        Ok((schema, encrypt_config)) => {
            debug!(target: SCHEMA, msg = "Reloaded committed schema snapshot", version = loaded_generation);
            publish_if_newer(
                &snapshot,
                CommittedSchemaSnapshot::new(loaded_generation, schema, encrypt_config),
            );
            published_publication.fetch_max(publication_generation, Ordering::AcqRel);
            true
        }
        Err(err) => {
            warn!(
                msg = "Error reloading committed schema snapshot",
                error = err.to_string()
            );
            false
        }
    }
}

/// Publishes a candidate only when it advances the committed version.
fn publish_if_newer(
    store: &ArcSwap<CommittedSchemaSnapshot>,
    candidate: CommittedSchemaSnapshot,
) -> bool {
    if candidate.version() <= store.load().version() {
        return false;
    }
    store.store(Arc::new(candidate));
    true
}

/// Loads the initial snapshot and starts periodic authoritative refreshes.
async fn init_reloader(config: DatabaseConfig) -> Result<SchemaManager, Error> {
    // Skip retries on startup as the likely failure mode is configuration
    let (schema, encrypt_config) = load_snapshot(&config).await?;
    info!(msg = "Loaded committed schema snapshot");

    let snapshot = Arc::new(ArcSwap::new(Arc::new(CommittedSchemaSnapshot::new(
        1,
        schema,
        encrypt_config,
    ))));
    let requested_generation = Arc::new(AtomicU64::new(1));
    let requested_publication = Arc::new(AtomicU64::new(0));
    let published_publication = Arc::new(AtomicU64::new(0));
    let reload_lock = Arc::new(Mutex::new(()));

    let config_ref = config.clone();
    let snapshot_ref = snapshot.clone();
    let generation_ref = requested_generation.clone();
    let requested_publication_ref = requested_publication.clone();
    let published_publication_ref = published_publication.clone();
    let reload_lock_ref = reload_lock.clone();

    let reload_handle = tokio::spawn(async move {
        let reload_interval = tokio::time::Duration::from_secs(config_ref.config_reload_interval);

        let mut interval = tokio::time::interval_at(
            tokio::time::Instant::now() + reload_interval,
            reload_interval,
        );

        loop {
            interval.tick().await;

            coalesced_reload(
                snapshot_ref.clone(),
                generation_ref.clone(),
                requested_publication_ref.clone(),
                published_publication_ref.clone(),
                reload_lock_ref.clone(),
                || load_snapshot_with_retry(&config_ref),
            )
            .await;
        }
    });

    Ok(SchemaManager {
        config,
        snapshot,
        requested_generation,
        requested_publication,
        published_publication,
        reload_lock,
        _reload_handle: Arc::new(reload_handle),
    })
}

/// Fetches an atomic snapshot and retries transient catalog-read failures.
///
/// When databases and the proxy start up at the same time they might not be ready to accept connections before the
/// proxy tries to query the schema. To give the proxy the best chance of initialising correctly this method will
/// retry the query a few times before passing on the error.
async fn load_snapshot_with_retry(
    config: &DatabaseConfig,
) -> Result<(Schema, EncryptConfig), Error> {
    let mut retry_count = 0;
    let max_retry_count = 10;
    let max_backoff = Duration::from_secs(2);

    loop {
        match load_snapshot(config).await {
            Ok(snapshot) => {
                return Ok(snapshot);
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
    load_snapshot(config).await.map(|(schema, _)| schema)
}

/// Reads schema and encryption metadata in one repeatable-read transaction.
async fn load_snapshot(config: &DatabaseConfig) -> Result<(Schema, EncryptConfig), Error> {
    let client = connect::database(config).await?;
    client
        .batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await?;

    let tables = client.query(SCHEMA_QUERY, &[]).await?;

    let mut schema = Schema::new("public");
    let mut encrypt_config = EncryptConfig::new();

    if tables.is_empty() {
        warn!(msg = "Database schema contains no tables");
    }

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

                if let Some(domain) = column_domain_name.as_deref() {
                    if let Some(config) = column_config_from_domain(&table_name, col, domain) {
                        encrypt_config
                            .insert(Identifier::new(table_name.clone(), col.clone()), config);
                    }
                }
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

    client.batch_execute("COMMIT").await?;
    Ok((schema, encrypt_config))
}

#[cfg(test)]
mod test {
    use super::*;
    use eql_mapper::ColumnKind;
    use std::sync::atomic::AtomicUsize;
    use tokio::sync::Notify;

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

    #[test]
    fn an_older_generation_cannot_replace_a_newer_snapshot() {
        let snapshot = ArcSwap::new(Arc::new(CommittedSchemaSnapshot::new(
            3,
            Schema::new("public"),
            EncryptConfig::new(),
        )));

        assert!(!publish_if_newer(
            &snapshot,
            CommittedSchemaSnapshot::new(2, Schema::new("stale"), EncryptConfig::new(),),
        ));
        assert_eq!(snapshot.load().version(), 3);
    }

    #[tokio::test]
    async fn requests_arriving_during_a_reload_are_coalesced_into_one_follow_up() {
        let snapshot = Arc::new(ArcSwap::new(Arc::new(CommittedSchemaSnapshot::new(
            1,
            Schema::new("public"),
            EncryptConfig::new(),
        ))));
        let requested_generation = Arc::new(AtomicU64::new(1));
        let requested_publication = Arc::new(AtomicU64::new(1));
        let published_publication = Arc::new(AtomicU64::new(0));
        let reload_lock = Arc::new(Mutex::new(()));
        let loads = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());

        let first = tokio::spawn(coalesced_reload(
            snapshot.clone(),
            requested_generation.clone(),
            requested_publication.clone(),
            published_publication.clone(),
            reload_lock.clone(),
            {
                let loads = loads.clone();
                let started = started.clone();
                let release = release.clone();
                move || async move {
                    loads.fetch_add(1, Ordering::AcqRel);
                    started.notify_one();
                    release.notified().await;
                    Ok((Schema::new("public"), EncryptConfig::new()))
                }
            },
        ));

        started.notified().await;
        requested_publication.fetch_add(1, Ordering::AcqRel);
        let second = tokio::spawn(coalesced_reload(
            snapshot.clone(),
            requested_generation.clone(),
            requested_publication.clone(),
            published_publication.clone(),
            reload_lock.clone(),
            {
                let loads = loads.clone();
                move || async move {
                    loads.fetch_add(1, Ordering::AcqRel);
                    Ok((Schema::new("public"), EncryptConfig::new()))
                }
            },
        ));

        let third = tokio::spawn(coalesced_reload(
            snapshot.clone(),
            requested_generation.clone(),
            requested_publication.clone(),
            published_publication.clone(),
            reload_lock.clone(),
            {
                let loads = loads.clone();
                move || async move {
                    loads.fetch_add(1, Ordering::AcqRel);
                    Ok((Schema::new("public"), EncryptConfig::new()))
                }
            },
        ));

        while requested_generation.load(Ordering::Acquire) < 4 {
            tokio::task::yield_now().await;
        }
        release.notify_one();

        assert!(first.await.unwrap());
        assert!(second.await.unwrap());
        assert!(third.await.unwrap());
        assert_eq!(loads.load(Ordering::Acquire), 2);
        assert_eq!(snapshot.load().version(), 4);
        assert_eq!(published_publication.load(Ordering::Acquire), 2);
    }
}
