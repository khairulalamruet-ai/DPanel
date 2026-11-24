//! A tenant's own Node.js and Python applications for real (T12.07, T12.08): containers on a
//! tenant-style network that run the tenant's files, reached by name from another container
//! of the network. Needs rootless Docker and the network; ignored by default.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{os::unix::fs::PermissionsExt as _, path::PathBuf, sync::Arc, time::Duration};

use builtpanel_docker_manager::{ContainerSpec, DockerManager, Name, Network, VolumeMount};
use builtpanel_domain::TenantUsername;
use builtpanel_node_agent::ops::{
    runtimeapp::{remove_runtime_app, set_runtime_app},
    tenant::TenantContext,
};
use builtpanel_node_protocol::{ErrorCode, Operation, Output, RuntimeEnv};
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

fn open_dir(path: &std::path::Path) {
    std::fs::create_dir_all(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o777)).unwrap();
}

#[tokio::test]
#[ignore = "needs rootless Docker and the network"]
async fn node_and_python_applications_run_on_the_tenants_network_and_answer_by_name() {
    let base = std::env::temp_dir().join("bp-runtime-live");
    let _ = std::fs::remove_dir_all(&base);
    let tenants = base.join("tenants");
    let state = base.join("state");
    std::fs::create_dir_all(state.join("vhosts")).unwrap();
    let tenant = Uuid::now_v7();
    let www = tenants.join(tenant.to_string()).join("www");
    open_dir(&www);
    // A Node.js application that answers with a value of its environment, and a Python one.
    open_dir(&www.join("api"));
    std::fs::write(
        www.join("api/server.js"),
        "const http = require('http');\n\
         http.createServer((req, res) => res.end('node:' + process.env.GREETING + ':' + process.env.PORT + ':' + process.cwd()))\n\
           .listen(Number(process.env.PORT), '0.0.0.0');\n",
    )
    .unwrap();
    std::fs::write(
        www.join("api/package.json"),
        "{\"name\":\"api\",\"version\":\"1.0.0\",\"private\":true}\n",
    )
    .unwrap();
    open_dir(&www.join("py"));
    std::fs::write(
        www.join("py/app.py"),
        "import http.server, os\n\
         class H(http.server.BaseHTTPRequestHandler):\n\
         \x20   def do_GET(self):\n\
         \x20       self.send_response(200); self.end_headers()\n\
         \x20       self.wfile.write(('python:' + os.environ['PORT'] + ':' + os.getcwd()).encode())\n\
         http.server.HTTPServer(('0.0.0.0', int(os.environ['PORT'])), H).serve_forever()\n",
    )
    .unwrap();
    std::fs::write(www.join("py/requirements.txt"), "# no dependencies\n").unwrap();

    let username = TenantUsername::parse("alice").unwrap();
    let docker = DockerManager::connect(&docker_socket()).unwrap();
    let network = network_name(&username).unwrap();
    let files = files_volume_name(&username).unwrap();
    for n in [
        "bp-alice-app-api",
        "bp-alice-app-py",
        "bp-alice-app-broken",
        "bp-rtprobe",
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
    let set = |kind: &str, version: &str, name: &str, dir: &str, command: &[&str], port: u16| {
        Operation::SetRuntimeApp {
            app_id: Uuid::now_v7(),
            tenant_id: tenant,
            username: "alice".into(),
            uid: 10_000,
            kind: kind.into(),
            version: version.into(),
            name: name.into(),
            dir: dir.into(),
            command: command.iter().map(|w| (*w).to_owned()).collect(),
            install: true,
            port,
            memory_mb: 256,
            env: Some(RuntimeEnv(vec![(
                "GREETING".into(),
                "it's \"quoted\" $(whoami) `id`".into(),
            )])),
            image: None,
        }
    };

    // Bad input first: an unknown version, a folder that climbs out, a name with a capital, a
    // reserved variable, an empty command.
    let mut bad = vec![
        set("node", "99", "api", "api", &["node", "server.js"], 3000),
        set("node", "24", "api", "../x", &["node", "server.js"], 3000),
        set("node", "24", "Api", "api", &["node", "server.js"], 3000),
        set("node", "24", "api", "api", &[], 3000),
        set("ruby", "3", "api", "api", &["ruby"], 3000),
        set("node", "24", "api", "api", &["node", "server.js"], 80),
    ];
    if let Operation::SetRuntimeApp { env, .. } = &mut bad[0] {
        *env = Some(RuntimeEnv(vec![("PATH".into(), "/tmp".into())]));
    }
    bad.push({
        let mut op = set("node", "24", "api", "api", &["node", "server.js"], 3000);
        if let Operation::SetRuntimeApp { env, .. } = &mut op {
            *env = Some(RuntimeEnv(vec![("PATH".into(), "/tmp".into())]));
        }
        op
    });
    for op in bad {
        let err = set_runtime_app(context.clone(), op).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArguments, "{err:?}");
    }

    // The probe: a container on the tenant's network that asks an application by name.
    let allowlist = Allowlist::embedded().unwrap();
    let node_image = allowlist.runtime_image(Role::Node, "24").unwrap().clone();
    let ask = |url: String| {
        let docker = DockerManager::connect(&docker_socket()).unwrap();
        let (network, files, node_image) = (network.clone(), files.clone(), node_image.clone());
        async move {
            let mut spec = ContainerSpec::new(Name::parse("bp-rtprobe").unwrap(), node_image);
            spec.entrypoint = vec!["node".into(), "-e".into()];
            spec.command = vec![format!(
                "fetch('{url}').then(r => r.text()).then(t => console.log('ANSWER=' + t)).catch(e => console.log('ERROR=' + e))"
            )];
            spec.network = Network::Named(network);
            spec.volumes = vec![VolumeMount {
                volume: files,
                target: "/app".into(),
                read_only: true,
            }];
            let result = docker
                .run_to_end(&spec, Duration::from_secs(60))
                .await
                .unwrap();
            result.output
        }
    };

    let ask_with_host = |url: String, host: &'static str| {
        let docker = DockerManager::connect(&docker_socket()).unwrap();
        let (network, files, node_image) = (network.clone(), files.clone(), node_image.clone());
        async move {
            let mut spec = ContainerSpec::new(Name::parse("bp-rtprobe").unwrap(), node_image);
            spec.entrypoint = vec!["node".into(), "-e".into()];
            spec.command = vec![format!(
                "const u = new URL('{url}'); require('http').get({{host: u.hostname, port: u.port, path: '/', headers: {{host: '{host}'}}}}, r => {{ let t = ''; r.on('data', d => t += d); r.on('end', () => console.log('ANSWER=' + t)); }}).on('error', e => console.log('ERROR=' + e));"
            )];
            spec.network = Network::Named(network);
            spec.volumes = vec![VolumeMount {
                volume: files,
                target: "/app".into(),
                read_only: true,
            }];
            docker
                .run_to_end(&spec, Duration::from_secs(60))
                .await
                .unwrap()
                .output
        }
    };

    // Node.js.
    let out = set_runtime_app(
        context.clone(),
        set("node", "24", "api", "api", &["node", "server.js"], 3000),
    )
    .await
    .unwrap();
    assert_eq!(
        out,
        Output::RuntimeAppSet {
            host: "bp-alice-app-api:3000".into()
        }
    );
    let said = ask("http://bp-alice-app-api:3000/".into()).await;
    // The value reached the program intact (quotes, `$(…)` and backticks are only characters),
    // it listens on the port it was given, and it runs in its own folder.
    assert!(
        said.contains("ANSWER=node:it's \"quoted\" $(whoami) `id`:3000:/app/api"),
        "{said}"
    );
    // Its dependencies were installed (a lock file appears when `npm install` has run).
    assert!(www.join("api/package-lock.json").exists());
    // Changed and run again: the same container name, the new command and environment.
    set_runtime_app(
        context.clone(),
        set("node", "24", "api", "api", &["node", "server.js"], 3000),
    )
    .await
    .unwrap();

    // Python, on a port of its own, with a command written with $PORT already worked out.
    let out = set_runtime_app(
        context.clone(),
        set("python", "3.13", "py", "py", &["python", "app.py"], 5000),
    )
    .await
    .unwrap();
    assert_eq!(
        out,
        Output::RuntimeAppSet {
            host: "bp-alice-app-py:5000".into()
        }
    );
    let said = ask("http://bp-alice-app-py:5000/".into()).await;
    assert!(said.contains("ANSWER=python:5000:/app/py"), "{said}");

    // The web server of the account passes a domain on to the application: the site file made
    // for a domain with a running application, in a real nginx on the same network.
    let domain = builtpanel_domain::Fqdn::parse("app.example.com").unwrap();
    let rendered = builtpanel_domain::vhost::render(
        builtpanel_domain::vhost::DEFAULT_NGINX_TEMPLATE,
        &builtpanel_domain::vhost::Site {
            domain: &domain,
            aliases: &[],
            docroot: &builtpanel_domain::Docroot::default(),
            php_upstream: "bp-alice-php:9000",
            policy: &builtpanel_domain::vhost::Policy::default(),
            custom: "",
            proxy: Some("bp-alice-app-api:3000"),
        },
    )
    .unwrap();
    let conf = base.join("conf");
    open_dir(&conf);
    std::fs::write(conf.join("app.example.com.conf"), rendered).unwrap();
    std::fs::set_permissions(
        conf.join("app.example.com.conf"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let conf_volume = Name::parse("bp-alice-testconf").unwrap();
    let _ = docker
        .remove(&Name::parse("bp-alice-testweb").unwrap(), true)
        .await;
    let _ = docker.remove_volume(&conf_volume).await;
    docker
        .create_bind_volume(&conf_volume, &conf, &base)
        .await
        .unwrap();
    let mut web = ContainerSpec::new(
        Name::parse("bp-alice-testweb").unwrap(),
        allowlist.current(Role::Web).unwrap().clone(),
    );
    web.writable_root = true;
    web.network = Network::Named(network.clone());
    web.volumes = vec![VolumeMount {
        volume: conf_volume.clone(),
        target: "/etc/nginx/conf.d".into(),
        read_only: true,
    }];
    docker.pull_image(&web.image).await.unwrap();
    docker.create_container(&web).await.unwrap();
    docker
        .start(&Name::parse("bp-alice-testweb").unwrap())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;
    let said = ask_with_host("http://bp-alice-testweb:8080/".into(), "app.example.com").await;
    assert!(
        said.contains("ANSWER=node:"),
        "through the web server: {said}"
    );
    let _ = docker
        .remove(&Name::parse("bp-alice-testweb").unwrap(), true)
        .await;
    let _ = docker.remove_volume(&conf_volume).await;

    // Another tenant's network does not reach it: nothing is published on the host either.
    let published = docker
        .inspect(&Name::parse("bp-alice-app-api").unwrap())
        .await
        .unwrap();
    assert!(published.published.is_empty(), "{published:?}");

    // A program that stops right away is reported, with its exit code.
    let err = set_runtime_app(
        context.clone(),
        set(
            "node",
            "24",
            "broken",
            "api",
            &["node", "no-such-file.js"],
            3001,
        ),
    )
    .await
    .unwrap_err();
    assert!(
        err.message.contains("stopped right after starting"),
        "{err:?}"
    );
    let _ = remove_runtime_app(
        context.clone(),
        Operation::RemoveRuntimeApp {
            tenant_id: tenant,
            username: "alice".into(),
            name: "broken".into(),
        },
    )
    .await;

    // Removed: the container is gone, the files stay.
    for name in ["api", "py"] {
        let out = remove_runtime_app(
            context.clone(),
            Operation::RemoveRuntimeApp {
                tenant_id: tenant,
                username: "alice".into(),
                name: name.into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(out, Output::RuntimeAppRemoved { existed: true });
    }
    let again = remove_runtime_app(
        context.clone(),
        Operation::RemoveRuntimeApp {
            tenant_id: tenant,
            username: "alice".into(),
            name: "api".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(again, Output::RuntimeAppRemoved { existed: false });
    assert!(www.join("api/server.js").exists());
    let _ = Uuid::nil();
}
