//! The catalog's applications installed for real (T12.05): the generic engine in a one-off
//! container against a real MariaDB on a tenant-style network. Needs rootless Docker and the
//! network; run one at a time (`--test-threads=1`). Ignored by default.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use builtpanel_docker_manager::{Capability, ContainerSpec, DockerManager, Name, Network};
use builtpanel_domain::TenantUsername;
use builtpanel_node_agent::ops::{appinstall::install_app_as, tenant::TenantContext};
use builtpanel_node_protocol::{AppSite, DbPassword, ErrorCode, Operation, Output, WpDatabase};
use builtpanel_tenant_stack::{Allowlist, Role, files_volume_name, network_name};
use uuid::Uuid;

fn docker_socket() -> PathBuf {
    // The engine the live tests run against: another one (Kiri) than the rootless Docker of this user.
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

static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Rig {
    context: Arc<TenantContext>,
    docker: DockerManager,
    tenant: Uuid,
    www: PathBuf,
    base: PathBuf,
    db: Name,
}

const ROOT_PW: &str = "rootpw-for-the-test";

async fn rig(tag: &str) -> Rig {
    let base = std::env::temp_dir().join(format!("bp-appinstall-{tag}"));
    let _ = std::fs::remove_dir_all(&base);
    let tenants = base.join("tenants");
    let state = base.join("state");
    std::fs::create_dir_all(state.join("vhosts")).unwrap();
    let tenant = Uuid::now_v7();
    let www = tenants.join(tenant.to_string()).join("www");
    std::fs::create_dir_all(&www).unwrap();
    // A root engine (Kiri) has no user-id shift: the tenant's id must be allowed into the folder.
    if std::env::var_os("BUILTPANEL_TEST_UID_IDENTITY").is_some() {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&www, std::fs::Permissions::from_mode(0o777)).unwrap();
    }
    let username = TenantUsername::parse("alice").unwrap();
    let docker = DockerManager::connect(&docker_socket()).unwrap();
    let network = network_name(&username).unwrap();
    let files = files_volume_name(&username).unwrap();
    let db = Name::parse("bp-apit-db").unwrap();
    let _ = docker.remove(&db, true).await;
    let _ = docker.remove_volume(&files).await;
    docker.ensure_network(&network, false).await.unwrap();
    docker
        .create_bind_volume(&files, &www, &tenants)
        .await
        .unwrap();
    let allowlist = Allowlist::embedded().unwrap();
    let image = allowlist.current(Role::Db).unwrap().clone();
    docker.pull_image(&image).await.unwrap();
    let mut spec = ContainerSpec::new(db.clone(), image);
    spec.env
        .insert("MARIADB_ROOT_PASSWORD".into(), ROOT_PW.into());
    spec.writable_root = true;
    spec.memory_bytes = 512 * 1024 * 1024;
    spec.cap_add = vec![
        Capability::Chown,
        Capability::DacOverride,
        Capability::Fowner,
        Capability::Setgid,
        Capability::Setuid,
    ];
    let aux = Name::parse("bp-apit-aux").unwrap();
    docker.ensure_network(&aux, false).await.unwrap();
    spec.network = Network::Named(aux);
    docker.create_container(&spec).await.unwrap();
    docker.connect_network(&db, &network, "db").await.unwrap();
    docker.start(&db).await.unwrap();
    let mut ready = false;
    for _ in 0..60 {
        if let Ok(r) = docker
            .run_user_command(
                &db,
                &format!("mariadb -uroot -p{ROOT_PW} -e 'select 1'"),
                "/",
                Duration::from_secs(10),
            )
            .await
            && r.exit_code == 0
        {
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    assert!(ready, "the database did not come up");
    let context = Arc::new(TenantContext {
        helper_socket: base.join("no-helper.sock"),
        tenants_root: tenants,
        docker: DockerManager::connect(&docker_socket()).unwrap(),
        vhosts_root: state.join("vhosts"),
    });
    Rig {
        context,
        docker,
        tenant,
        www,
        base,
        db,
    }
}

impl Rig {
    async fn sql(&self, sql: &str) -> String {
        let r = self
            .docker
            .run_user_command(
                &self.db,
                &format!("mariadb -uroot -p{ROOT_PW} -N -e \"{sql}\""),
                "/",
                Duration::from_secs(30),
            )
            .await
            .unwrap();
        assert_eq!(r.exit_code, 0, "{}", r.output);
        r.output
    }

    async fn database(&self, name: &str, password: &str) {
        self.sql(&format!("CREATE DATABASE {name}; CREATE USER '{name}'@'%' IDENTIFIED BY '{password}'; GRANT ALL ON {name}.* TO '{name}'@'%';"))
            .await;
    }

    fn op(&self, slug: &str, path: &str, db: Option<&str>, password: &str) -> Operation {
        let definition = builtpanel_catalog::Catalog::embedded()
            .unwrap()
            .get(slug)
            .unwrap()
            .clone();
        Operation::InstallApp {
            app_id: Uuid::now_v7(),
            tenant_id: self.tenant,
            username: "alice".into(),
            uid: 10_000,
            path: path.into(),
            definition: Box::new(definition),
            database: db.map(|d| WpDatabase {
                name: d.into(),
                user: d.into(),
            }),
            database_password: db.map(|_| DbPassword(password.into())),
            site: AppSite {
                url: "https://alice.example/app".into(),
                title: "Alice App".into(),
                admin_user: "boss".into(),
                admin_email: "boss@example.com".into(),
            },
            admin_password: Some(DbPassword("Sup3r-Secret-Admin-Pass-1".into())),
        }
    }

    async fn finish(self) {
        let _ = self.docker.remove(&self.db, true).await;
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

#[tokio::test]
#[ignore = "needs rootless Docker and the network"]
async fn a_static_site_is_made_from_its_steps_alone_and_a_used_folder_is_refused() {
    let _one = ONE_AT_A_TIME.lock().await;
    let rig = rig("static").await;
    let out = install_app_as(
        rig.context.clone(),
        rig.op("static-site", "site", None, ""),
        (0, 0),
    )
    .await
    .unwrap();
    assert!(matches!(out, Output::AppInstalled { .. }), "{out:?}");
    let page = std::fs::read_to_string(rig.www.join("site").join("index.html")).unwrap();
    assert!(page.contains("<h1>Alice App</h1>"), "{page}");
    // A folder with something in it is refused; a default index page does not count.
    // (Under a root engine the folder belongs to the tenant's id and this user cannot write in it.)
    if std::fs::write(rig.www.join("site").join("mine.txt"), "mine").is_ok() {
        let e = install_app_as(
            rig.context.clone(),
            rig.op("static-site", "site", None, ""),
            (0, 0),
        )
        .await
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidArguments, "{e:?}");
    } else {
        eprintln!("skipped the used-folder check: the folder is not writable here");
    }
    // A folder that climbs out and a definition that is not valid are refused before anything runs.
    let e = install_app_as(
        rig.context.clone(),
        rig.op("static-site", "../x", None, ""),
        (0, 0),
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidArguments);
    let mut bad = rig.op("static-site", "other", None, "");
    if let Operation::InstallApp { definition, .. } = &mut bad {
        definition
            .steps
            .push(builtpanel_node_protocol::AppStep::Php {
                args: vec!["{{shell}}".into()],
            });
    }
    let e = install_app_as(rig.context.clone(), bad, (0, 0))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidArguments);
    rig.finish().await;
}

#[tokio::test]
#[ignore = "needs rootless Docker and the network"]
async fn joomla_is_downloaded_checked_and_installed_with_its_own_installer() {
    let _one = ONE_AT_A_TIME.lock().await;
    let rig = rig("joomla").await;
    rig.database("alice_joomla", "dbpass-for-joomla-1").await;
    let started = Instant::now();
    let out = install_app_as(
        rig.context.clone(),
        rig.op(
            "joomla",
            "joomla",
            Some("alice_joomla"),
            "dbpass-for-joomla-1",
        ),
        (0, 0),
    )
    .await;
    eprintln!("joomla in {:?}: {out:?}", started.elapsed());
    out.unwrap();
    assert!(rig.www.join("joomla").join("configuration.php").is_file());
    assert!(
        !rig.www.join("joomla").join("installation").exists(),
        "the installer folder is removed"
    );
    let tables = rig.sql("select count(*) from information_schema.tables where table_schema='alice_joomla' and table_name like 'jml\\\\_%'").await;
    assert!(tables.trim().parse::<u32>().unwrap() > 20, "{tables}");
    // A wrong checksum would have stopped it: the same definition with another one fails.
    let mut tampered = rig.op(
        "joomla",
        "joomla2",
        Some("alice_joomla"),
        "dbpass-for-joomla-1",
    );
    if let Operation::InstallApp { definition, .. } = &mut tampered {
        definition.source.as_mut().unwrap().sha256 = "0".repeat(64);
    }
    let e = install_app_as(rig.context.clone(), tampered, (0, 0))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Internal, "{e:?}");
    assert!(
        !e.message.contains("Sup3r") && !e.message.contains("dbpass"),
        "{e:?}"
    );
    assert!(!rig.www.join("joomla2").join("configuration.php").exists());
    rig.finish().await;
}

#[tokio::test]
#[ignore = "needs rootless Docker and the network"]
async fn laravel_is_made_with_composer_and_configured_for_its_database() {
    let _one = ONE_AT_A_TIME.lock().await;
    let rig = rig("laravel").await;
    rig.database("alice_laravel", "dbpass-for-laravel-1").await;
    let started = Instant::now();
    let out = install_app_as(
        rig.context.clone(),
        rig.op(
            "laravel",
            "app",
            Some("alice_laravel"),
            "dbpass-for-laravel-1",
        ),
        (0, 0),
    )
    .await;
    eprintln!("laravel in {:?}: {out:?}", started.elapsed());
    out.unwrap();
    let env = std::fs::read_to_string(rig.www.join("app").join(".env")).unwrap();
    assert!(
        env.contains("DB_DATABASE=alice_laravel") && env.contains("APP_KEY=base64:"),
        "{env}"
    );
    assert!(rig.www.join("app").join("artisan").is_file());
    assert!(
        rig.www
            .join("app")
            .join("vendor")
            .join("autoload.php")
            .is_file()
    );
    rig.finish().await;
}

#[tokio::test]
#[ignore = "needs rootless Docker and the network"]
async fn drupal_is_made_with_composer_and_its_database_is_set_for_the_installer() {
    let _one = ONE_AT_A_TIME.lock().await;
    let rig = rig("drupal").await;
    rig.database("alice_drupal", "dbpass-for-drupal-1").await;
    let started = Instant::now();
    let out = install_app_as(
        rig.context.clone(),
        rig.op("drupal", "cms", Some("alice_drupal"), "dbpass-for-drupal-1"),
        (0, 0),
    )
    .await;
    eprintln!("drupal in {:?}: {out:?}", started.elapsed());
    out.unwrap();
    let site = rig.www.join("cms");
    let settings = std::fs::read_to_string(site.join("web/sites/default/settings.php")).unwrap();
    assert!(
        settings.contains("'database' => 'alice_drupal'")
            && settings.contains("'password' => 'dbpass-for-drupal-1'")
            && settings.contains(r"Drupal\\mysql\\Driver\\Database\\mysql"),
        "{settings}"
    );
    assert!(site.join("web/index.php").is_file());
    assert!(site.join("web/core/install.php").is_file());
    assert!(site.join("vendor/bin/drush").exists());
    rig.finish().await;
}
