//! The database studio for real: a real MariaDB, real tables, and a client's requests done
//! with the temporary account of one database. Ignored by default; needs the login user's
//! rootless Docker (or `BUILTPANEL_TEST_DOCKER_SOCKET`).
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, sync::Arc};

use builtpanel_docker_manager::{DockerManager, Name};
use builtpanel_node_agent::ops::{
    db::{CONTAINER, create_database, ensure_db_server},
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
            engine: DbEngine::Mysql,
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

#[tokio::test]
#[ignore = "needs rootless Docker (see the file header)"]
async fn a_client_browses_edits_and_queries_only_its_own_database() {
    let socket = docker_socket();
    if !socket.exists() {
        eprintln!("skipped: no Docker at {}", socket.display());
        return;
    }
    let docker = DockerManager::connect(&socket).unwrap();
    let _ = docker.remove(&Name::parse(CONTAINER).unwrap(), true).await;
    let _ = docker
        .remove_volume(&Name::parse("bp-db-data").unwrap())
        .await;
    let root_dir = std::env::temp_dir().join(format!("bp-studio-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root_dir);
    std::fs::create_dir_all(&root_dir).unwrap();
    let context = Arc::new(TenantContext {
        helper_socket: "/nonexistent".into(),
        tenants_root: "/nonexistent".into(),
        docker: docker.clone(),
        vhosts_root: root_dir.join("vhosts"),
    });
    ensure_db_server(context.clone(), Operation::EnsureDbServer)
        .await
        .unwrap();
    for name in ["sttest_shop", "sttest_other"] {
        create_database(
            context.clone(),
            Operation::CreateDatabase {
                name: name.into(),
                engine: DbEngine::Mysql,
                dedicated: None,
            },
        )
        .await
        .unwrap();
    }
    let db = "sttest_shop";

    // The client builds a table with the SQL editor.
    for sql in [
        "CREATE TABLE items (id int NOT NULL AUTO_INCREMENT PRIMARY KEY, name varchar(40) NOT NULL, price decimal(8,2), added datetime, note text, KEY by_name (name))",
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

    // The tables.
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

    // The structure.
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
    assert!(columns[2].nullable && columns[2].data_type.starts_with("decimal"));
    assert!(indexes.iter().any(|i| i.name == "by_name" && !i.unique));
    assert!(indexes.iter().any(|i| i.name == "PRIMARY" && i.unique));
    assert!(create_sql.starts_with("CREATE TABLE `items`"));
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

    // A page: sorted, filtered, with NULLs and dates as text.
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

    // A hostile filter value is a value, nothing else.
    let StudioResult::Rows { total, .. } = ask(
        &context,
        db,
        StudioRequest::Rows {
            table: "items".into(),
            offset: 0,
            limit: 10,
            order_by: None,
            descending: false,
            filters: vec![Filter {
                column: "name".into(),
                op: FilterOp::Eq,
                value: Some("x' OR '1'='1".into()),
            }],
        },
    )
    .await
    else {
        panic!()
    };
    assert_eq!(total, 0);

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
                        value: Some("lamp".into())
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
                    }
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
    assert_eq!(grid.rows, [[Some("lamp".into()), Some("9.99".into())]]);
    // A key that is not the whole primary key, or a table without one, is refused.
    assert!(matches!(
        ask(
            &context,
            db,
            StudioRequest::DeleteRow {
                table: "items".into(),
                key: vec![KeyPart {
                    column: "name".into(),
                    value: "pen".into()
                }],
            }
        )
        .await,
        StudioResult::Error {
            code: StudioErrorCode::Invalid,
            ..
        }
    ));
    assert!(matches!(
        ask(
            &context,
            db,
            StudioRequest::DeleteRow {
                table: "loose".into(),
                key: vec![KeyPart {
                    column: "a".into(),
                    value: "1".into()
                }],
            }
        )
        .await,
        StudioResult::Error {
            code: StudioErrorCode::Invalid,
            ..
        }
    ));
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

    // Statements: a result set, a count, a server error in the server's words.
    let StudioResult::Query {
        grid: Some(grid), ..
    } = ask(&context, db, query("SELECT id FROM items ORDER BY id")).await
    else {
        panic!()
    };
    assert_eq!(grid.rows.len(), 3);
    assert!(!grid.truncated);
    let StudioResult::Query {
        grid: Some(grid), ..
    } = ask(&context, db, query("SELECT seq FROM seq_1_to_3000")).await
    else {
        panic!()
    };
    assert_eq!(grid.rows.len(), 1000);
    assert!(grid.truncated, "more than 1000 rows are cut");
    let StudioResult::Query { grid, affected, .. } =
        ask(&context, db, query("UPDATE items SET note = 'x'")).await
    else {
        panic!()
    };
    assert!(grid.is_none());
    assert_eq!(affected, 3);
    assert!(matches!(
        ask(&context, db, query("SELEC nonsense")).await,
        StudioResult::Error {
            code: StudioErrorCode::Sql,
            ..
        }
    ));

    // The walls: another database, the server's accounts and its settings stay out of reach.
    let other = ask(&context, db, query("CREATE TABLE sttest_other.t (a int)")).await;
    assert!(
        matches!(
            other,
            StudioResult::Error {
                code: StudioErrorCode::Sql,
                ..
            }
        ),
        "{other:?}"
    );
    for sql in [
        "SELECT user FROM mysql.user",
        "CREATE USER evil IDENTIFIED BY 'x'",
        "GRANT ALL ON *.* TO CURRENT_USER",
        "SET GLOBAL max_connections = 1",
        "SELECT LOAD_FILE('/etc/passwd')",
        "SELECT 1 INTO OUTFILE '/tmp/studio-leak'",
        "DROP DATABASE sttest_other",
    ] {
        let result = ask(&context, db, query(sql)).await;
        let blocked = matches!(result, StudioResult::Error { .. })
            || matches!(&result, StudioResult::Query { grid: Some(g), .. } if g.rows.iter().all(|r| r[0].is_none()));
        assert!(blocked, "{sql} was not stopped: {result:?}");
    }
    // A runaway statement is stopped by the server's time limit.
    let slow = ask(&context, db, query("SELECT SLEEP(30)")).await;
    match slow {
        StudioResult::Query { grid: Some(g), .. } => {
            assert_eq!(g.rows[0][0].as_deref(), Some("1"), "SLEEP is cut short");
        }
        StudioResult::Error { .. } => {}
        other => panic!("{other:?}"),
    }
    // Bad names never reach the server.
    assert!(matches!(
        ask(
            &context,
            db,
            StudioRequest::Describe {
                table: "a`b".into()
            }
        )
        .await,
        StudioResult::Error { .. }
    ));

    // No temporary account is left behind.
    let leftovers: Vec<_> = std::fs::read_dir(root_dir.join("pma/accounts"))
        .map(|d| d.flatten().collect())
        .unwrap_or_default();
    assert!(leftovers.is_empty(), "accounts left: {leftovers:?}");
    let _ = docker.remove(&Name::parse(CONTAINER).unwrap(), true).await;
    let _ = docker
        .remove_volume(&Name::parse("bp-db-data").unwrap())
        .await;
}
