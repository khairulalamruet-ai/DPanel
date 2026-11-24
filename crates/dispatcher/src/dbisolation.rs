//! Plan-driven placement of a tenant's databases (T07.13): a plan with the `db_dedicated`
//! feature gives the tenant its own database server; a tenant whose plan changes has its
//! databases moved, one operation on the node (dump from the one server, load into the other).
//!
//! The work is a loop that can be run any time: it compares what each tenant's plan says with
//! where its databases are (`tenant_db_servers`), starts the move where they differ, and, when
//! a move's operation has succeeded, switches the tenant over and asks the node to remove the
//! old copies. A move that failed for good is tried again after a while.

use builtpanel_database::tenant_db::{self, Candidate};
use builtpanel_node_protocol::{DbEngine, MigrateGrant, MigrateRemote, Operation};
use builtpanel_operations::{Enqueued, Request, Status};
use uuid::Uuid;

use crate::Dispatcher;

/// The feature of a plan that gives its tenants their own database servers.
pub const FEATURE: &str = "db_dedicated";
/// How long a failed move waits before it is tried again.
const RETRY_AFTER_SECS: i64 = 600;

impl Dispatcher {
    /// Starts, finishes or retries the moves of databases between servers. Returns how many
    /// tenants' placements changed or were started.
    pub async fn reconcile_db_isolation(&self) -> usize {
        let db = self.ops.db();
        let candidates = match tenant_db::candidates(db).await {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!(error = %err, "could not look at where databases live");
                return 0;
            }
        };
        let mut changed = 0;
        for c in candidates {
            match self.reconcile_one(&c).await {
                Ok(true) => changed += 1,
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(tenant = %c.tenant_id, engine = %c.engine, %error, "database placement");
                }
            }
        }
        changed
    }

    async fn reconcile_one(&self, c: &Candidate) -> Result<bool, String> {
        let db = self.ops.db();
        let Some(engine) = DbEngine::parse(&c.engine) else {
            return Ok(false);
        };
        let wants = tenant_db::plan_asks_for_dedicated(db, c.plan_id)
            .await
            .map_err(|e| e.to_string())?;
        match (c.state.as_str(), wants) {
            ("shared", true) => self.start_move(c, engine, true).await,
            ("dedicated", false) => self.start_move(c, engine, false).await,
            ("moving_to_dedicated" | "moving_to_shared", _) => {
                let to_dedicated = c.state == "moving_to_dedicated";
                self.settle_move(c, engine, to_dedicated).await
            }
            _ => Ok(false),
        }
    }

    async fn start_move(
        &self,
        c: &Candidate,
        engine: DbEngine,
        to_dedicated: bool,
    ) -> Result<bool, String> {
        let db = self.ops.db();
        if c.age_secs < RETRY_AFTER_SECS {
            return Ok(false);
        }
        let set = tenant_db::move_set(db, c.tenant_id, &c.engine)
            .await
            .map_err(|e| e.to_string())?;
        if set.databases.is_empty() && set.users.is_empty() {
            // Nothing to move: the next one made goes where the plan says.
            tenant_db::set_state(
                db,
                c.tenant_id,
                &c.engine,
                if to_dedicated { "dedicated" } else { "shared" },
                None,
            )
            .await
            .map_err(|e| e.to_string())?;
            if !to_dedicated {
                self.queue_removal(c, engine).await?;
            }
            return Ok(true);
        }
        let target = if to_dedicated { "dedicated" } else { "shared" };
        let request = Request::new(
            c.node_id,
            Operation::MigrateTenantDatabases {
                username: c.username.clone(),
                engine,
                to_dedicated,
                databases: set.databases,
                users: set.users,
                grants: set
                    .grants
                    .into_iter()
                    .map(|(database, user, access)| MigrateGrant {
                        database,
                        user,
                        access,
                    })
                    .collect(),
                remote_hosts: set
                    .remote
                    .into_iter()
                    .map(|(user, cidrs)| MigrateRemote { user, cidrs })
                    .collect(),
            },
            format!(
                "dbmove:{}:{}:{target}:{}",
                c.tenant_id.simple(),
                c.engine,
                Uuid::now_v7().simple()
            ),
        );
        let mut request = request;
        request.tenant_id = Some(c.tenant_id);
        // Many databases take a while; the agent works on it as long as it needs.
        let id = match self
            .ops
            .enqueue(&request)
            .await
            .map_err(|e| e.to_string())?
        {
            Enqueued::Created(op) | Enqueued::Existing(op) => op.id,
        };
        tenant_db::set_state(
            db,
            c.tenant_id,
            &c.engine,
            if to_dedicated {
                "moving_to_dedicated"
            } else {
                "moving_to_shared"
            },
            Some(id),
        )
        .await
        .map_err(|e| e.to_string())?;
        Ok(true)
    }

    async fn settle_move(
        &self,
        c: &Candidate,
        engine: DbEngine,
        to_dedicated: bool,
    ) -> Result<bool, String> {
        let db = self.ops.db();
        let Some(id) = c.operation_id else {
            // A row with no operation cannot finish: start from where it was.
            let back = if to_dedicated { "shared" } else { "dedicated" };
            tenant_db::set_state(db, c.tenant_id, &c.engine, back, None)
                .await
                .map_err(|e| e.to_string())?;
            return Ok(true);
        };
        let status = builtpanel_database::operations::get(db, id)
            .await
            .map_err(|e| e.to_string())?
            .map(|o| o.status);
        match status {
            Some(Status::Succeeded) => {
                // Switched: from now on new work goes to the new server.
                let set = tenant_db::move_set(db, c.tenant_id, &c.engine)
                    .await
                    .map_err(|e| e.to_string())?;
                tenant_db::set_state(
                    db,
                    c.tenant_id,
                    &c.engine,
                    if to_dedicated { "dedicated" } else { "shared" },
                    None,
                )
                .await
                .map_err(|e| e.to_string())?;
                // The old copies go.
                if to_dedicated {
                    for name in &set.databases {
                        self.queue_cleanup(
                            c,
                            Operation::DropDatabase {
                                name: name.clone(),
                                engine,
                                dedicated: None,
                            },
                            &format!("dbmove-drop:{}:{name}", id.simple()),
                        )
                        .await?;
                    }
                    for name in &set.users {
                        self.queue_cleanup(
                            c,
                            Operation::DropDbUser {
                                name: name.clone(),
                                engine,
                                dedicated: None,
                            },
                            &format!("dbmove-dropuser:{}:{name}", id.simple()),
                        )
                        .await?;
                    }
                } else {
                    self.queue_removal(c, engine).await?;
                }
                Ok(true)
            }
            Some(Status::Failed) | None => {
                // It did not work; the databases are where they were. Tried again later.
                let back = if to_dedicated { "shared" } else { "dedicated" };
                tenant_db::set_state(db, c.tenant_id, &c.engine, back, None)
                    .await
                    .map_err(|e| e.to_string())?;
                tracing::warn!(tenant = %c.tenant_id, engine = %c.engine, "a database move failed");
                Ok(true)
            }
            // Still going.
            Some(_) => Ok(false),
        }
    }

    async fn queue_removal(&self, c: &Candidate, engine: DbEngine) -> Result<(), String> {
        self.queue_cleanup(
            c,
            Operation::RemoveDedicatedDb {
                username: c.username.clone(),
                engine,
            },
            &format!(
                "dbmove-remove:{}:{}:{}",
                c.tenant_id.simple(),
                c.engine,
                Uuid::now_v7().simple()
            ),
        )
        .await
    }

    async fn queue_cleanup(&self, c: &Candidate, op: Operation, key: &str) -> Result<(), String> {
        let mut request = Request::new(c.node_id, op, key.to_owned());
        request.tenant_id = Some(c.tenant_id);
        self.ops
            .enqueue(&request)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}
