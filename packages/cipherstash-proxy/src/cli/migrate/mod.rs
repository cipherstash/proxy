use crate::error::Error;
use crate::log::MIGRATE;
use crate::tls::NoCertificateVerification;
use crate::TandemConfig;
use postgres_protocol::escape::{escape_identifier, escape_literal};
use rustls::ClientConfig;
use rustls_platform_verifier::ConfigVerifierExt;
use std::error::Error as ClapError;
use std::sync::Arc;
use std::{self};
use tokio_postgres::{Client, NoTls};
use tracing::{debug, error, info, warn};

const ID: &str = "id";
const LOCALHOST: &str = "127.0.0.1";

#[derive(clap::Args, Clone, Debug)]
#[command(version, about, long_about)]
///
/// Encrypt one or more columns in table
/// Requires a running and configured CipherStash Proxy instance.
///
pub struct Migrate {
    ///
    /// Name of database table
    ///
    #[arg(short, long)]
    table: String,

    ///
    /// Source and destination columns as space-delimited key pairs `--columns source=destination`
    ///
    #[arg(short, required = true, long, num_args(1..), value_parser = parse_key_val::<String, String>)]
    columns: Vec<(String, String)>,

    ///
    /// Primary key column/s
    /// Compound primary keys can be provided as a space delimted list: `--primary-key id user_id`
    ///
    #[arg(short = 'k', long, num_args(1..), value_delimiter = ' ', default_values_t = vec![ID.to_string()])]
    primary_key: Vec<String>,

    // Updates `batch_size` records at a time
    #[arg(short, long, default_value_t = 100)]
    batch_size: usize,

    /// Run without update. Data is fetched, but updates are not performed.
    #[arg(short, long, default_value_t = false)]
    dry_run: bool,

    /// Turn on additional logging output
    #[arg(short, long, default_value_t = false)]
    verbose: bool,
}

impl Migrate {
    ///
    /// Returns true if this is not a `dry_run`
    ///
    fn commit(&self) -> bool {
        !self.dry_run
    }

    ///
    /// Run the encryption migration process
    ///
    pub async fn run(&self, config: TandemConfig) -> Result<(), Error> {
        let connection_string = format!(
            "postgresql://{}:{}@{}:{}/{}",
            config.database.username,
            config.database.password,
            config.server.host,
            config.server.port,
            config.database.name
        );

        debug!(target: MIGRATE, ?config);

        let client =
            connect_with_tls(&connection_string, config.database.with_tls_verification).await?;

        info!(
            target: MIGRATE,
            msg = "Connected to Database",
            database = config.database.name,
            host = config.server.host,
            port = config.server.port,
            username = config.database.username,
            with_tls_verification = config.database.with_tls_verification,
        );

        info!(target: MIGRATE, msg = "Encrypting table", table = self.table, columns = ?self.columns);

        if !self.commit() {
            warn!(msg = "Dry run is enabled");
        }

        let mut batch_count = 0;
        let mut updated_count = 0;

        let quoted_table = escape_identifier(&self.table);

        let primary_key_idents = self
            .primary_key
            .iter()
            .map(|pk| escape_identifier(pk))
            .collect::<Vec<String>>()
            .join(", ");

        let column_idents = self
            .columns
            .iter()
            .map(|(source_col, _target_col)| escape_identifier(source_col))
            .collect::<Vec<String>>()
            .join(", ");

        let sql =
        format!("SELECT {primary_key_idents}, {column_idents} FROM {quoted_table} ORDER BY {primary_key_idents}");

        if self.commit() {
            client.simple_query("BEGIN;").await?;
        }

        loop {
            let offset = batch_count * self.batch_size;

            let sql = format!("{sql} LIMIT {} OFFSET {offset} FOR UPDATE", self.batch_size);

            if self.verbose {
                info!(target: MIGRATE, msg = "Fetch records to encrypt", table = self.table,  columns = ?self.columns);
            }
            debug!(target: MIGRATE, msg = "Select", sql);

            let rows = match client.simple_query(&sql).await {
                Ok(rows) => rows,
                Err(err) => {
                    error!(target: MIGRATE, msg = "Error fetching records", table = self.table, error = err.to_string());
                    std::process::exit(exitcode::SOFTWARE);
                }
            };

            let row_count = rows.len() - 1; // Last row is always CommandComplete

            // Batch by building a single statement string
            let mut update_sql = String::from("");
            let mut records = vec![];

            for row in rows {
                match row {
                    tokio_postgres::SimpleQueryMessage::Row(row) => {
                        let primary_key_vals = self
                            .primary_key
                            .iter()
                            .map(|c| row.get(c.as_str()).unwrap_or("").to_owned())
                            .collect::<Vec<String>>();

                        // Get the plaintext value
                        let column_vals = self
                            .columns
                            .iter()
                            .map(|(source_col, _target_col)| {
                                row.get(source_col.as_str()).unwrap_or("").to_owned()
                            })
                            .collect::<Vec<String>>();
                        // Set the col to update to the encrypted column
                        // And set the plaintext value to the p value
                        let update_str = self
                            .columns
                            .iter()
                            .zip(column_vals.iter())
                            .map(|((_source_col, target_col), val)| {
                                format!("\"{target_col}\"='{val}'")
                            })
                            .collect::<Vec<String>>()
                            .join(", ");

                        let where_str = self
                            .primary_key
                            .iter()
                            .zip(primary_key_vals.iter())
                            .map(|(col, val)| {
                                let col = escape_identifier(col);
                                let val = escape_literal(val);
                                format!("{col}={val}")
                            })
                            .collect::<Vec<String>>()
                            .join(" AND ");

                        let sql = format!(
                            "UPDATE {} SET {update_str} WHERE {where_str};\n",
                            self.table
                        );

                        updated_count += 1;
                        update_sql.push_str(&sql);
                        records.extend(primary_key_vals.to_owned());
                    }
                    tokio_postgres::SimpleQueryMessage::CommandComplete(..) => {}
                    tokio_postgres::SimpleQueryMessage::RowDescription(..) => {}
                    _ => {
                        error!(target: MIGRATE, msg = "Invalid row. You should not even be here.");
                        unreachable!()
                    }
                }
            }

            if self.verbose {
                info!(target: MIGRATE, msg = "Encrypting", ?records);
            }

            debug!(target: MIGRATE, msg = "Update", update_sql = update_sql);

            if self.commit() {
                {
                    client.simple_query(&update_sql).await?;
                    client.simple_query("COMMIT;").await?;
                }
            }

            if row_count < self.batch_size {
                info!(
                    target: MIGRATE,
                    msg = "Encryption complete",
                    updated = updated_count,
                    batches = batch_count+1,
                );

                println!();
                break;
            }

            batch_count += 1;
        }

        Ok(())
    }
}

/// Parse a single key-value pair - copied from clap example https://github.com/clap-rs/clap/blob/master/examples/typed-derive.rs#L25
fn parse_key_val<T, U>(s: &str) -> Result<(T, U), Box<dyn ClapError + Send + Sync + 'static>>
where
    T: std::str::FromStr,
    T::Err: ClapError + Send + Sync + 'static,
    U: std::str::FromStr,
    U::Err: ClapError + Send + Sync + 'static,
{
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid KEY=value: no `=` found in `{s}`"))?;
    Ok((s[..pos].parse()?, s[pos + 1..].parse()?))
}

pub async fn connect_with_tls(
    connection_string: &str,
    with_tls_verification: bool,
) -> Result<Client, Error> {
    let mut tls_config = ClientConfig::with_platform_verifier();

    if !with_tls_verification {
        let mut dangerous = tls_config.dangerous();
        dangerous.set_certificate_verifier(Arc::new(NoCertificateVerification {}));
    }

    let tls = tokio_postgres_rustls::MakeRustlsConnect::new(tls_config);
    let (client, connection) = tokio_postgres::connect(connection_string, tls)
        .await
        .inspect_err(|_| {
            error!(target: MIGRATE, msg = "Error connecting Encryption Migrator to Proxy");
        })?;

    tokio::spawn(async move {
        if let Err(err) = connection.await {
            error!("Connection error: {}", err);
        }
    });
    Ok(client)
}

pub async fn connect_with_no_tls(connection_string: &str) -> Result<Client, Error> {
    let (client, connection) = tokio_postgres::connect(connection_string, NoTls)
        .await
        .inspect_err(|_| {
            error!(target: MIGRATE, msg = "Error connecting Encryption Migrator to Proxy");
        })?;

    tokio::spawn(async move {
        if let Err(err) = connection.await {
            error!("Connection error: {}", err);
        }
    });
    Ok(client)
}

mod tests {
    use super::Migrate;

    impl Migrate {
        pub fn new(table: String, columns: Vec<(String, String)>) -> Self {
            Migrate {
                table,
                columns,
                primary_key: vec!["id".to_string()],
                batch_size: 10,
                dry_run: false,
                verbose: false,
            }
        }
    }
}
