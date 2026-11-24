//! A tenant's own cache for real (T12.10): Redis and Memcached containers on a tenant-style
//! network, and WordPress's object cache working through the Redis. Needs rootless Docker and
//! the network; ignored by default.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, sync::Arc, time::Duration};

use builtpanel_docker_manager::{ContainerSpec, DockerManager, Name, Network, VolumeMount};
use builtpanel_domain::TenantUsername;
use builtpanel_node_agent::ops::{
    cacheaddon::{remove_cache_instance, set_cache_instance},
    tenant::TenantContext,
};
use builtpanel_node_protocol::{DbPassword, ErrorCode, Operation, Output};
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

#[tokio::test]
#[ignore = "needs rootless Docker and the network"]
async fn a_tenants_cache_runs_on_its_network_with_a_password_and_wordpress_uses_it() {
    let base = std::env::temp_dir().join("bp-cache-live");
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
    for n in [
        "bp-alice-redis",
        "bp-alice-memcached",
        "bp-cachit-db",
        "bp-cachit-wp",
    ] {
        let _ = docker.remove(&Name::parse(n).unwrap(), true).await;
    }
    let _ = docker.remove_volume(&files).await;
    docker.ensure_network(&network, false).await.unwrap();
    docker
        .create_bind_volume(&files, &www, &tenants)
        .await
        .unwrap();
    let context = Arc::new(TenantContext {
        helper_socket: base.join("no-helper.sock"),
        tenants_root: tenants,
        docker: DockerManager::connect(&docker_socket()).unwrap(),
        vhosts_root: state.join("vhosts"),
    });
    let set = |kind: &str, memory_mb: u32, password: Option<&str>| Operation::SetCacheInstance {
        instance_id: Uuid::now_v7(),
        tenant_id: tenant,
        username: "alice".into(),
        // (As root of the user namespace: the files of the test belong to the test's user.)
        uid: 10_000,
        kind: kind.into(),
        memory_mb,
        password: password.map(|p| DbPassword(p.into())),
    };
    // Bad input first: a short password, an unknown kind, memory out of range.
    for bad in [
        set("redis", 64, Some("short")),
        set("redis", 64, None),
        set("etcd", 64, None),
        set("redis", 0, Some("a-long-password-1")),
        set("redis", 9999, Some("a-long-password-1")),
    ] {
        let e = set_cache_instance(context.clone(), bad).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidArguments, "{e:?}");
    }
    let redis_pw = "Redis-pass-for-the-test-1";
    let out = set_cache_instance(context.clone(), set("redis", 32, Some(redis_pw)))
        .await
        .unwrap();
    assert!(
        matches!(out, Output::CacheInstanceSet { ref host, port: 6379 } if host == "bp-alice-redis"),
        "{out:?}"
    );
    let out = set_cache_instance(context.clone(), set("memcached", 16, None))
        .await
        .unwrap();
    assert!(
        matches!(out, Output::CacheInstanceSet { ref host, port: 11211 } if host == "bp-alice-memcached"),
        "{out:?}"
    );
    // The password is not in the container's command line or environment.
    let info = docker
        .inspect(&Name::parse("bp-alice-redis").unwrap())
        .await
        .unwrap();
    assert!(
        info.running && info.limits.memory_bytes == (32 + 48) * 1024 * 1024,
        "{info:?}"
    );
    let report = format!("{info:?}");
    assert!(!report.contains(redis_pw), "{report}");
    // WordPress's object cache, through the Redis: a throwaway install, the Redis Object
    // Cache plugin, switched on. (Raw WP-CLI here: the panel's own vocabulary has no such command.)
    let allowlist = Allowlist::embedded().unwrap();
    let wpcli = allowlist.current(Role::Wpcli).unwrap().clone();
    docker.pull_image(&wpcli).await.unwrap();
    let db_image = allowlist.current(Role::Db).unwrap().clone();
    docker.pull_image(&db_image).await.unwrap();
    let mut dbs = ContainerSpec::new(Name::parse("bp-cachit-db").unwrap(), db_image);
    dbs.env
        .insert("MARIADB_ROOT_PASSWORD".into(), "rootpw-for-the-test".into());
    dbs.env.insert("MARIADB_DATABASE".into(), "wp".into());
    dbs.env.insert("MARIADB_USER".into(), "wp".into());
    dbs.env
        .insert("MARIADB_PASSWORD".into(), "wp-pass-for-the-test-1".into());
    dbs.writable_root = true;
    dbs.memory_bytes = 512 * 1024 * 1024;
    dbs.cap_add = vec![
        builtpanel_docker_manager::Capability::Chown,
        builtpanel_docker_manager::Capability::DacOverride,
        builtpanel_docker_manager::Capability::Fowner,
        builtpanel_docker_manager::Capability::Setgid,
        builtpanel_docker_manager::Capability::Setuid,
    ];
    let aux = Name::parse("bp-cachit-aux").unwrap();
    docker.ensure_network(&aux, false).await.unwrap();
    dbs.network = Network::Named(aux);
    docker.create_container(&dbs).await.unwrap();
    docker
        .connect_network(&dbs.name, &network, "db")
        .await
        .unwrap();
    docker.start(&dbs.name).await.unwrap();
    for _ in 0..60 {
        if docker
            .run_user_command(
                &dbs.name,
                "mariadb -uwp -pwp-pass-for-the-test-1 -e 'select 1' wp",
                "/",
                Duration::from_secs(10),
            )
            .await
            .is_ok_and(|r| r.exit_code == 0)
        {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let script = format!(
        "set -e; W='php -d memory_limit=512M /usr/local/bin/wp --allow-root --path=/var/www/html'; \
         $W core download >/dev/null; \
         $W config create --dbname=wp --dbuser=wp --dbpass=wp-pass-for-the-test-1 --dbhost=db --skip-salts >/dev/null; \
         $W core install --url=http://x.example --title=T --admin_user=boss --admin_password=Sup3r-Secret-Admin-Pass! --admin_email=a@example.com --skip-email >/dev/null; \
         $W config set WP_REDIS_HOST bp-alice-redis >/dev/null; $W config set WP_REDIS_PASSWORD '{redis_pw}' >/dev/null; \
         $W plugin install redis-cache --activate >/dev/null; $W redis enable 2>&1; $W redis status 2>&1"
    );
    let mut wp = ContainerSpec::new(Name::parse("bp-cachit-wp").unwrap(), wpcli);
    wp.entrypoint = vec!["sh".into(), "-c".into(), script];
    wp.user = Some((0, 0));
    wp.memory_bytes = 1024 * 1024 * 1024;
    wp.writable_root = true;
    wp.network = Network::Named(network.clone());
    wp.volumes = vec![VolumeMount {
        volume: files.clone(),
        target: "/var/www/html".into(),
        read_only: false,
    }];
    let ran = docker
        .run_to_end(&wp, Duration::from_secs(300))
        .await
        .unwrap();
    eprintln!("{}", ran.output);
    assert_eq!(ran.exit_code, 0, "{}", ran.output);
    assert!(ran.output.contains("Status: Connected"), "{}", ran.output);
    // Remove: gone, and safe to repeat.
    let gone = Operation::RemoveCacheInstance {
        tenant_id: tenant,
        username: "alice".into(),
        kind: "redis".into(),
    };
    assert!(matches!(
        remove_cache_instance(context.clone(), gone.clone())
            .await
            .unwrap(),
        Output::CacheInstanceRemoved { existed: true }
    ));
    assert!(matches!(
        remove_cache_instance(context.clone(), gone).await.unwrap(),
        Output::CacheInstanceRemoved { existed: false }
    ));
    assert!(
        docker
            .inspect(&Name::parse("bp-alice-redis").unwrap())
            .await
            .is_err()
    );
    for n in ["bp-alice-memcached", "bp-cachit-db"] {
        let _ = docker.remove(&Name::parse(n).unwrap(), true).await;
    }
    let _ = docker.remove_volume(&files).await;
    let _ = std::fs::remove_dir_all(&base);
}
