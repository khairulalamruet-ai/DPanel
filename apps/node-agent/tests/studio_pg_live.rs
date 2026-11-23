//! The database studio on PostgreSQL for real: the node's own server in Docker, real tables,
//! and a client's requests done with a temporary role of one database. Ignored by default;
//! needs the login user's rootless Docker (or `BUILTPANEL_TEST_DOCKER_SOCKET`) and the image.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, process::Command, sync::Arc};

use builtpanel_docker_manager::DockerManager;
use builtpanel_node_agent::ops::{
    dbroute::create_database,
    pg::{self, CONTAINER},
    studio::studio_op,
    tenant::TenantContext,
};
use builtpanel_node_protocol::{
    DbEngine, Operation, Output,
    studio::{Assignment, Filter, FilterOp, KeyPart, StudioErrorCode, StudioRequest, StudioResult},
};

fn docker_socket() -> PathBuf {
    if let Some(path) = std::env::var_os("BUILTPANEL_TEST_DOCKER_SOCKET") {
        return PathBuf::from(path);
    }
    let uid = std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find(|l| l.starts_with("Uid:"))
        .and_then(|l| l.split_whitespace().nth(1).map(str::to_owned))
        .unwrap();
    PathBuf::from(format!("/run/user/{uid}/docker.sock"))
}

async fn ask(context: &Arc<TenantContext>, database: &str, request: StudioRequest) -> StudioResult {
    let output = studio_op(
        context.clone(),
        Operation::StudioOp {
            database: database.into(),
            engine: DbEngine::Postgres,
            request,
        },
    )
    .await
    .expect("the studio answered");
    let Output::Studio { result } = output else {
        panic!("unexpected output {output:?}")
    };
    result
}

fn query(sql: &str) -> StudioRequest {
    StudioRequest::Query { sql: sql.into() }
}

fn is_sql_error(result: &StudioResult) -> bool {
    matches!(
        result,
        StudioResult::Error {
            code: StudioErrorCode::Sql,
            ..
        }
    )
}

#[tokio::test]
#[ignore = "needs rootless Docker (see the file header)"]
async fn a_client_browses_edits_and_queries_only_its_own_postgresql_database() {
    let socket = docker_socket();
    if !socket.exists() {
        eprintln!("skipped: no Docker at {}", socket.display());
        return;
    }
    let docker = DockerManager::connect(&socket).unwrap();
    let root = std::env::temp_dir().join(format!("bp-studio-pg-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let context = Arc::new(TenantContext {
        helper_socket: "/nonexistent".into(),
        tenants_root: "/nonexistent".into(),
        docker: docker.clone(),
        vhosts_root: root.join("vhosts"),
    });
    let _ = Command::new("docker")
        .args(["rm", "-f", CONTAINER])
        .output();
    for volume in ["bp-pg-data", "bp-pg-conf"] {
        let _ = Command::new("docker")
            .args(["volume", "rm", "-f", volume])
            .output();
    }
    pg::ensure_pg_server(context.clone(), Operation::EnsurePgServer)
        .await
        .unwrap();
    for name in ["sttest_shop", "sttest_other"] {
        create_database(
            context.clone(),
            Operation::CreateDatabase {
                name: name.into(),
                engine: DbEngine::Postgres,
                dedicated: None,
            },
        )
        .await
        .unwrap();
    }
    let db = "sttest_shop";

    for sql in [
        "CREATE TABLE items (id serial PRIMARY KEY, name varchar(40) NOT NULL, price numeric(8,2), added timestamp, note text)",
        "CREATE INDEX by_name ON items (name)",
        "INSERT INTO items (name, price, added) VALUES ('pen', 1.50, '2026-01-02 03:04:05'), ('book', 12.00, NULL), ('cup', 4.25, '2026-02-03 00:00:00')",
        "CREATE TABLE loose (a int)",
        "CREATE VIEW cheap AS SELECT name FROM items WHERE price < 5",
    ] {
        let result = ask(&context, db, query(sql)).await;
        assert!(
            matches!(result, StudioResult::Query { .. }),
            "{sql}: {result:?}"
        );
    }

    // What one request made, the next one (another role) can use: the objects went to the group.
    let StudioResult::Tables { tables } = ask(&context, db, StudioRequest::Tables).await else {
        panic!("no tables")
    };
    let names: Vec<_> = tables
        .iter()
        .map(|t| (t.name.as_str(), t.kind.as_str()))
        .collect();
    assert_eq!(
        names,
        [("cheap", "view"), ("items", "table"), ("loose", "table")]
    );

    let StudioResult::Described {
        columns,
        indexes,
        create_sql,
    } = ask(
        &context,
        db,
        StudioRequest::Describe {
            table: "items".into(),
        },
    )
    .await
    else {
        panic!("not described")
    };
    assert_eq!(columns.len(), 5);
    assert!(columns[0].primary_key && columns[0].generated && !columns[0].nullable);
    assert!(columns[2].nullable && columns[2].data_type.starts_with("numeric"));
    assert!(indexes.iter().any(|i| i.name == "by_name" && !i.unique));
    assert!(indexes.iter().any(|i| i.unique && i.columns == ["id"]));
    assert!(
        create_sql.starts_with("CREATE TABLE \"items\""),
        "{create_sql}"
    );
    assert!(matches!(
        ask(
            &context,
            db,
            StudioRequest::Describe {
                table: "nope".into()
            }
        )
        .await,
        StudioResult::Error {
            code: StudioErrorCode::NotFound,
            ..
        }
    ));

    let StudioResult::Rows { grid, total } = ask(
        &context,
        db,
        StudioRequest::Rows {
            table: "items".into(),
            offset: 0,
            limit: 10,
            order_by: Some("price".into()),
            descending: true,
            filters: vec![Filter {
                column: "name".into(),
                op: FilterOp::Ne,
                value: Some("cup".into()),
            }],
        },
    )
    .await
    else {
        panic!("no rows")
    };
    assert_eq!(total, 2);
    assert_eq!(grid.columns, ["id", "name", "price", "added", "note"]);
    assert_eq!(grid.rows[0][1].as_deref(), Some("book"));
    assert_eq!(grid.rows[0][3], None);
    assert_eq!(grid.rows[1][3].as_deref(), Some("2026-01-02 03:04:05"));
    assert_eq!(grid.rows[1][2].as_deref(), Some("1.50"));

    // A numeric comparison is numeric; LIKE works on any type; a hostile value is only a value.
    for (filter, expected) in [
        (
            Filter {
                column: "price".into(),
                op: FilterOp::Gt,
                value: Some("4".into()),
            },
            2,
        ),
        (
            Filter {
                column: "id".into(),
                op: FilterOp::Like,
                value: Some("1%".into()),
            },
            1,
        ),
        (
            Filter {
                column: "name".into(),
                op: FilterOp::Eq,
                value: Some("x' OR '1'='1".into()),
            },
            0,
        ),
    ] {
        let StudioResult::Rows { total, .. } = ask(
            &context,
            db,
            StudioRequest::Rows {
                table: "items".into(),
                offset: 0,
                limit: 10,
                order_by: None,
                descending: false,
                filters: vec![filter.clone()],
            },
        )
        .await
        else {
            panic!("{filter:?}")
        };
        assert_eq!(total, expected, "{filter:?}");
    }

    // Add, change and delete one row by its key.
    assert_eq!(
        ask(
            &context,
            db,
            StudioRequest::InsertRow {
                table: "items".into(),
                values: vec![
                    Assignment {
                        column: "name".into(),
                        value: Some("O'Brien's lamp".into())
                    },
                    Assignment {
                        column: "price".into(),
                        value: None
                    },
                ],
            }
        )
        .await,
        StudioResult::Changed { affected: 1 }
    );
    assert_eq!(
        ask(
            &context,
            db,
            StudioRequest::UpdateRow {
                table: "items".into(),
                key: vec![KeyPart {
                    column: "id".into(),
                    value: "4".into()
                }],
                values: vec![
                    Assignment {
                        column: "price".into(),
                        value: Some("9.99".into())
                    },
                    Assignment {
                        column: "note".into(),
                        value: None
                    },
                ],
            }
        )
        .await,
        StudioResult::Changed { affected: 1 }
    );
    let StudioResult::Query {
        grid: Some(grid), ..
    } = ask(
        &context,
        db,
        query("SELECT name, price FROM items WHERE id = 4"),
    )
    .await
    else {
        panic!()
    };
    assert_eq!(
        grid.rows,
        [[Some("O'Brien's lamp".into()), Some("9.99".into())]]
    );
    for (table, key) in [("items", "name"), ("loose", "a")] {
        assert!(
            matches!(
                ask(
                    &context,
                    db,
                    StudioRequest::DeleteRow {
                        table: table.into(),
                        key: vec![KeyPart {
                            column: key.into(),
                            value: "1".into()
                        }],
                    }
                )
                .await,
                StudioResult::Error {
                    code: StudioErrorCode::Invalid,
                    ..
                }
            ),
            "{table}"
        );
    }
    assert_eq!(
        ask(
            &context,
            db,
            StudioRequest::DeleteRow {
                table: "items".into(),
                key: vec![KeyPart {
                    column: "id".into(),
                    value: "4".into()
                }],
            }
        )
        .await,
        StudioResult::Changed { affected: 1 }
    );

    // Statements: rows, a cut result, a count, the server's words.
    let StudioResult::Query {
        grid: Some(grid), ..
    } = ask(&context, db, query("SELECT generate_series(1, 3000)")).await
    else {
        panic!()
    };
    assert_eq!(grid.rows.len(), 1000);
    assert!(grid.truncated);
    let StudioResult::Query { grid, affected, .. } =
        ask(&context, db, query("UPDATE items SET note = 'x'")).await
    else {
        panic!()
    };
    assert!(grid.is_none());
    assert_eq!(affected, 3);
    assert!(is_sql_error(
        &ask(&context, db, query("SELEC nonsense")).await
    ));

    // The walls: another database, other roles and the server itself stay out of reach.
    for sql in [
        "CREATE TABLE sttest_other.public.t (a int)",
        "SELECT rolpassword FROM pg_authid",
        "SELECT passwd FROM pg_shadow",
        "CREATE ROLE evil LOGIN SUPERUSER PASSWORD 'x'",
        "ALTER SYSTEM SET max_connections = 1",
        "COPY items TO PROGRAM 'id > /tmp/studio-leak'",
        "COPY items TO '/tmp/studio-leak'",
        "SELECT pg_read_file('/etc/passwd')",
        "DROP DATABASE sttest_other",
        "SET ROLE postgres",
        "GRANT ALL ON ALL TABLES IN SCHEMA public TO PUBLIC; ALTER USER postgres PASSWORD 'x'",
    ] {
        let result = ask(&context, db, query(sql)).await;
        assert!(is_sql_error(&result), "{sql} was not stopped: {result:?}");
    }
    // A runaway statement is stopped by the time limit.
    let slow = ask(&context, db, query("SELECT pg_sleep(60)")).await;
    assert!(is_sql_error(&slow), "{slow:?}");
    assert!(matches!(
        ask(
            &context,
            db,
            StudioRequest::Describe {
                table: "a\"b".into()
            }
        )
        .await,
        StudioResult::Error { .. }
    ));

    // No temporary role is left behind.
    let password = std::fs::read_to_string(root.join("secrets/postgres-root")).unwrap();
    let (admin, connection) = tokio_postgres::Config::new()
        .host("127.0.0.1")
        .port(pg::port())
        .user("postgres")
        .password(password.trim())
        .dbname("postgres")
        .connect(tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(connection);
    let left: Vec<String> = admin
        .query(
            "SELECT rolname FROM pg_roles WHERE rolname LIKE 'dhst\\_%'",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert!(left.is_empty(), "roles left: {left:?}");
    let _ = Command::new("docker")
        .args(["rm", "-f", CONTAINER])
        .output();
}
