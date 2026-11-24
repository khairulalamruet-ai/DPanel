//! Which database server an operation is for (T07.12, T07.13): the node's MariaDB (what
//! every operation meant before) or its PostgreSQL, the node's shared server or a tenant's own.
//! The operation says; this module runs the code of that server with that server's names (see
//! [`super::dbserver`]).

use std::sync::Arc;

use builtpanel_node_protocol::{AgentError, DbEngine, Operation, Output};

use super::{db, dbserver, pg, sqlexport, sqlimport, tenant::TenantContext};

/// The tenant whose own server an operation is for (none: the shared one).
fn tenant_of(op: &Operation) -> Option<String> {
    op.db_target()
        .and_then(|(_, dedicated)| dedicated.map(str::to_owned))
}

fn engine(op: &Operation) -> DbEngine {
    op.db_target().map_or(DbEngine::Mysql, |(engine, _)| engine)
}

macro_rules! route {
    ($name:ident, $mysql:path, $postgres:path) => {
        /// Hands the operation to the server it is for.
        ///
        /// # Errors
        /// Whatever that server's code answers.
        pub async fn $name(
            context: Arc<TenantContext>,
            op: Operation,
        ) -> Result<Output, AgentError> {
            let tenant = tenant_of(&op);
            match engine(&op) {
                DbEngine::Mysql => dbserver::scope(tenant, $mysql(context, op)).await,
                DbEngine::Postgres => dbserver::scope(tenant, $postgres(context, op)).await,
            }
        }
    };
}

route!(create_database, db::create_database, pg::create_database);
route!(drop_database, db::drop_database, pg::drop_database);
route!(upsert_db_user, db::upsert_db_user, pg::upsert_db_user);
route!(grant_db, db::grant_db, pg::grant_db);
route!(revoke_db, db::revoke_db, pg::revoke_db);
route!(
    set_db_remote_hosts,
    db::set_db_remote_hosts,
    pg::set_db_remote_hosts
);
route!(
    set_db_remote_access,
    db::set_db_remote_access,
    pg::set_db_remote_access
);
route!(drop_db_user, db::drop_db_user, pg::drop_db_user);
route!(
    allow_tenant_databases,
    db::allow_tenant_databases,
    pg::allow_tenant
);
route!(import_sql, sqlimport::import_sql, sqlimport::import_sql);
route!(export_sql, sqlexport::export_sql, sqlexport::export_sql);
