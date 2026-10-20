//! One test per matrix cell, so the test runner's pass/fail list *is* the
//! matrix. Test names read `<topology>_<mode>_<check>` inside a module per pooler.
//!
//! Cells share pooled server connections by design, so they run serially.

use super::{
    checks, run_cell, Cell, Check, PoolMode, Pooler, Topology, PGBOUNCER_1_19, PGBOUNCER_1_24,
    PGBOUNCER_1_24_PS, PGCAT_1_2, PGCAT_1_2_PS,
};

macro_rules! cell_test {
    ($name:ident, $pooler:expr, $topology:ident, $mode:ident, $check:ident, $run:path) => {
        #[tokio::test]
        #[serial_test::serial(poolers)]
        async fn $name() {
            let pooler: Option<Pooler> = $pooler;
            run_cell(
                Cell {
                    pooler,
                    topology: Topology::$topology,
                    mode: PoolMode::$mode,
                    check: Check::$check,
                },
                super::super::CELL_TIMEOUT,
                $run,
            )
            .await;
        }
    };
}

// A rejected startup cannot exercise CRUD. Record the actual protocol rejection
// separately instead of accepting arbitrary CRUD connection failures.
macro_rules! proxy_in_front_tests {
    ($pooler:expr, supported) => {
        // app -> Proxy -> pooler -> PostgreSQL. Proxy's state lives on the
        // client's own Proxy connection here, so only protocol behaviour is
        // at stake.
        cell_test!(
            proxy_in_front_session_crud_simple,
            Some($pooler),
            ProxyInFrontOfPooler,
            Session,
            CrudSimple,
            checks::crud_simple
        );
        cell_test!(
            proxy_in_front_session_crud_extended_unnamed,
            Some($pooler),
            ProxyInFrontOfPooler,
            Session,
            CrudExtendedUnnamed,
            checks::crud_extended_unnamed
        );
        cell_test!(
            proxy_in_front_session_crud_extended_named,
            Some($pooler),
            ProxyInFrontOfPooler,
            Session,
            CrudExtendedNamed,
            checks::crud_extended_named
        );
        cell_test!(
            proxy_in_front_txn_crud_simple,
            Some($pooler),
            ProxyInFrontOfPooler,
            Transaction,
            CrudSimple,
            checks::crud_simple
        );
        cell_test!(
            proxy_in_front_txn_crud_extended_unnamed,
            Some($pooler),
            ProxyInFrontOfPooler,
            Transaction,
            CrudExtendedUnnamed,
            checks::crud_extended_unnamed
        );
        cell_test!(
            proxy_in_front_txn_crud_extended_named,
            Some($pooler),
            ProxyInFrontOfPooler,
            Transaction,
            CrudExtendedNamed,
            checks::crud_extended_named
        );
        cell_test!(
            proxy_in_front_txn_prepared_statement_split,
            Some($pooler),
            ProxyInFrontOfPooler,
            Transaction,
            PreparedStatementSplit,
            checks::prepared_statement_split
        );
    };
    ($pooler:expr, rejected) => {
        cell_test!(
            proxy_in_front_session_startup,
            Some($pooler),
            ProxyInFrontOfPooler,
            Session,
            StartupProtocol,
            checks::startup_protocol
        );
        cell_test!(
            proxy_in_front_txn_startup,
            Some($pooler),
            ProxyInFrontOfPooler,
            Transaction,
            StartupProtocol,
            checks::startup_protocol
        );
    };
}

macro_rules! pooler_tests {
    ($module:ident, $pooler:expr, $startup:ident) => {
        mod $module {
            use super::*;

            // app -> pooler -> Proxy -> PostgreSQL, session mode
            cell_test!(
                pooler_in_front_session_crud_simple,
                Some($pooler),
                PoolerInFrontOfProxy,
                Session,
                CrudSimple,
                checks::crud_simple
            );
            cell_test!(
                pooler_in_front_session_crud_extended_unnamed,
                Some($pooler),
                PoolerInFrontOfProxy,
                Session,
                CrudExtendedUnnamed,
                checks::crud_extended_unnamed
            );
            cell_test!(
                pooler_in_front_session_crud_extended_named,
                Some($pooler),
                PoolerInFrontOfProxy,
                Session,
                CrudExtendedNamed,
                checks::crud_extended_named
            );
            cell_test!(
                pooler_in_front_session_keyset_isolation,
                Some($pooler),
                PoolerInFrontOfProxy,
                Session,
                KeysetIsolation,
                checks::keyset_isolation
            );
            cell_test!(
                pooler_in_front_session_disable_mapping_isolation,
                Some($pooler),
                PoolerInFrontOfProxy,
                Session,
                DisableMappingIsolation,
                checks::disable_mapping_isolation
            );

            // app -> pooler -> Proxy -> PostgreSQL, transaction mode
            cell_test!(
                pooler_in_front_txn_crud_simple,
                Some($pooler),
                PoolerInFrontOfProxy,
                Transaction,
                CrudSimple,
                checks::crud_simple
            );
            cell_test!(
                pooler_in_front_txn_crud_extended_unnamed,
                Some($pooler),
                PoolerInFrontOfProxy,
                Transaction,
                CrudExtendedUnnamed,
                checks::crud_extended_unnamed
            );
            cell_test!(
                pooler_in_front_txn_crud_extended_named,
                Some($pooler),
                PoolerInFrontOfProxy,
                Transaction,
                CrudExtendedNamed,
                checks::crud_extended_named
            );
            cell_test!(
                pooler_in_front_txn_prepared_statement_split,
                Some($pooler),
                PoolerInFrontOfProxy,
                Transaction,
                PreparedStatementSplit,
                checks::prepared_statement_split
            );
            cell_test!(
                pooler_in_front_txn_keyset_isolation,
                Some($pooler),
                PoolerInFrontOfProxy,
                Transaction,
                KeysetIsolation,
                checks::keyset_isolation
            );
            cell_test!(
                pooler_in_front_txn_set_local_keyset_isolation,
                Some($pooler),
                PoolerInFrontOfProxy,
                Transaction,
                SetLocalKeysetIsolation,
                checks::set_local_keyset_isolation
            );
            cell_test!(
                pooler_in_front_txn_disable_mapping_isolation,
                Some($pooler),
                PoolerInFrontOfProxy,
                Transaction,
                DisableMappingIsolation,
                checks::disable_mapping_isolation
            );

            proxy_in_front_tests!($pooler, $startup);
        }
    };
}

/// app -> Proxy -> PostgreSQL with no pooler: the control row.
mod direct {
    use super::*;

    cell_test!(
        crud_simple,
        None,
        Direct,
        Session,
        CrudSimple,
        checks::crud_simple
    );
    cell_test!(
        crud_extended_unnamed,
        None,
        Direct,
        Session,
        CrudExtendedUnnamed,
        checks::crud_extended_unnamed
    );
    cell_test!(
        crud_extended_named,
        None,
        Direct,
        Session,
        CrudExtendedNamed,
        checks::crud_extended_named
    );
    cell_test!(
        keyset_isolation,
        None,
        Direct,
        Session,
        KeysetIsolation,
        checks::keyset_isolation
    );
    cell_test!(
        set_local_keyset_isolation,
        None,
        Direct,
        Session,
        SetLocalKeysetIsolation,
        checks::set_local_keyset_isolation
    );
    cell_test!(
        disable_mapping_isolation,
        None,
        Direct,
        Session,
        DisableMappingIsolation,
        checks::disable_mapping_isolation
    );
}

pooler_tests!(pgbouncer_1_19, PGBOUNCER_1_19, rejected);
pooler_tests!(pgbouncer_1_24, PGBOUNCER_1_24, supported);
pooler_tests!(pgbouncer_1_24_ps, PGBOUNCER_1_24_PS, supported);
pooler_tests!(pgcat_1_2, PGCAT_1_2, rejected);
pooler_tests!(pgcat_1_2_ps, PGCAT_1_2_PS, rejected);

// PgCat only marks SET outside an explicit transaction for RESET ALL cleanup.
// Verify that supported path independently of the within-transaction leak cells.
macro_rules! pgcat_cleanup_tests {
    ($module:ident, $pooler:expr) => {
        mod $module {
            use super::*;
            cell_test!(
                session_standalone_keyset_cleanup,
                Some($pooler),
                PoolerInFrontOfProxy,
                Session,
                StandaloneKeysetCleanup,
                checks::keyset_isolation
            );
            cell_test!(
                transaction_standalone_keyset_cleanup,
                Some($pooler),
                PoolerInFrontOfProxy,
                Transaction,
                StandaloneKeysetCleanup,
                checks::keyset_isolation
            );
            cell_test!(
                session_standalone_mapping_cleanup,
                Some($pooler),
                PoolerInFrontOfProxy,
                Session,
                StandaloneMappingCleanup,
                checks::disable_mapping_isolation
            );
            cell_test!(
                transaction_standalone_mapping_cleanup,
                Some($pooler),
                PoolerInFrontOfProxy,
                Transaction,
                StandaloneMappingCleanup,
                checks::disable_mapping_isolation
            );
        }
    };
}
pgcat_cleanup_tests!(pgcat_1_2_cleanup, PGCAT_1_2);
pgcat_cleanup_tests!(pgcat_1_2_ps_cleanup, PGCAT_1_2_PS);
