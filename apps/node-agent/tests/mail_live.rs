//! The node's mail server for real (T13.02-T13.08): Stalwart in Docker, set up by the agent,
//! mail sent to it over SMTP and read back over IMAP. Ignored by default; needs the login user's
//! rootless Docker and these (the test's own high ports):
//!
//! ```text
//! BUILTPANEL_AGENT_MAIL_API_PORT=18089 BUILTPANEL_AGENT_MAIL_SMTP_PORT=12525 \
//!   BUILTPANEL_AGENT_MAIL_SUBMISSION_PORT=12465 BUILTPANEL_AGENT_MAIL_IMAP_PORT=12993
//! ```
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use builtpanel_docker_manager::DockerManager;
use builtpanel_node_agent::ops::{
    mail::{
        CONTAINER, drop_mailbox, ensure_mail_server, mail_queue_action, remove_mail_domain,
        report_mail, set_mail_autoresponder, set_mail_domain, set_mail_forwarders, upsert_mailbox,
    },
    tenant::TenantContext,
    webmail,
};
use builtpanel_node_protocol::{DbPassword, ErrorCode, MailForwarder, Operation, Output};

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

const PROBE: &str = r#"
import smtplib, imaplib, ssl, sys, json, time
GTUBE = "XJS*C4JDBQADN1.NSBN3*2IDNEN*GTUBE-STANDARD-ANTI-UBE-TEST-EMAIL*C.34X"
cmd = sys.argv[1]
def send(rcpt, subject, body, sender="someone@external.example", size=0):
    s = smtplib.SMTP("127.0.0.1", 25, timeout=30)
    text = body + ("\r\n" + ("x" * 76 + "\r\n") * (size // 78) if size else "")
    try:
        s.sendmail(sender, [rcpt], f"From: {sender}\r\nTo: {rcpt}\r\nSubject: {subject}\r\n\r\n{text}\r\n")
        return "accepted"
    except smtplib.SMTPResponseException as e:
        return f"refused {e.smtp_code} {e.smtp_error.decode(errors='replace')[:100]}"
    except smtplib.SMTPRecipientsRefused as e:
        return "refused " + str(list(e.recipients.values())[0])[:120]
    except Exception as e:
        return f"error {e}"
    finally:
        try: s.quit()
        except Exception: pass
def imap(user, password):
    m = imaplib.IMAP4_SSL("127.0.0.1", 993, ssl_context=ssl._create_unverified_context())
    m.login(user, password)
    return m
def headers(m):
    out = []
    for box in ["INBOX", "Junk Mail"]:
        st, n = m.select(f'"{box}"')
        if st != "OK": continue
        typ, data = m.search(None, "ALL")
        for num in data[0].split():
            typ, msg = m.fetch(num, "(BODY.PEEK[HEADER])")
            out.append({"box": box, "head": msg[0][1].decode(errors="replace")})
    return out
if cmd == "send":
    print(json.dumps(send(sys.argv[2], sys.argv[3], sys.argv[4], size=int(sys.argv[5]) if len(sys.argv) > 5 else 0)))
elif cmd == "sendauth":
    # user pass rcpt subject body: submitted over TLS as a local user
    s = smtplib.SMTP_SSL("127.0.0.1", 465, timeout=30, context=ssl._create_unverified_context())
    s.login(sys.argv[2], sys.argv[3])
    opts = ["HOLDFOR=3600"] if len(sys.argv) > 7 else []
    s.sendmail(sys.argv[2], [sys.argv[4]], mail_options=opts, msg=f"From: {sys.argv[2]}\r\nTo: {sys.argv[4]}\r\nSubject: {sys.argv[5]}\r\nDate: {__import__('email.utils').utils.formatdate()}\r\nMessage-ID: <{time.time()}@clientmail.org>\r\n\r\n{sys.argv[6]}\r\n")
    s.quit()
    print(json.dumps("accepted"))
elif cmd == "gtube":
    print(json.dumps(send(sys.argv[2], "spam test", GTUBE)))
elif cmd == "read":
    time.sleep(2)
    print(json.dumps(headers(imap(sys.argv[2], sys.argv[3]))))
elif cmd == "webmail":
    # Opens webmail with a one-time link and says where it ended up (a mailbox view when it worked).
    import urllib.request, urllib.error, http.cookiejar
    jar = http.cookiejar.CookieJar()
    web = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
    url = "http://bp-webmail:80/?_dhsignon=" + sys.argv[2]
    deadline = time.time() + 300
    while True:
        try:
            r = web.open(url, timeout=30)
            body = r.read().decode(errors="replace")
            print(json.dumps({"url": r.geturl(), "roundcube": "Roundcube" in body or "rcmail" in body}))
            break
        except urllib.error.HTTPError as e:
            print(json.dumps({"http": e.code, "text": e.read().decode(errors="replace")[:200]}))
            break
        except Exception as e:
            if time.time() > deadline:
                print(json.dumps({"error": str(e)}))
                break
            time.sleep(3)
elif cmd == "login":
    try:
        imap(sys.argv[2], sys.argv[3]); print(json.dumps("ok"))
    except Exception as e:
        print(json.dumps("no"))
"#;

fn probe(script: &Path, args: &[&str]) -> serde_json::Value {
    let out = Command::new("docker")
        .args([
            "run",
            "--rm",
            "--network",
            &format!("container:{CONTAINER}"),
            "-v",
            &format!("{}:/probe.py:ro", script.display()),
            "python:3-alpine",
            "python",
            "/probe.py",
        ])
        .args(args)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    serde_json::from_str(&text).unwrap_or_else(|e| {
        panic!(
            "probe {args:?}: {e}: {text} {}",
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

fn heads(v: &serde_json::Value) -> Vec<(String, String)> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["box"].as_str().unwrap().to_owned(),
                m["head"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[tokio::test]
#[ignore = "needs rootless Docker (see the file header)"]
#[allow(clippy::too_many_lines)]
async fn the_mail_server_is_set_up_and_takes_domains_mailboxes_forwarders_and_replies() {
    let socket = docker_socket();
    if !socket.exists() || std::env::var_os("BUILTPANEL_AGENT_MAIL_API_PORT").is_none() {
        eprintln!("skipped: no Docker, or the ports are not set (see the file header)");
        return;
    }
    let docker = DockerManager::connect(&socket).unwrap();
    let root = std::env::temp_dir().join(format!("bp-mail-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let script = root.join("probe.py");
    std::fs::write(&script, PROBE).unwrap();
    let context = Arc::new(TenantContext {
        helper_socket: "/nonexistent".into(),
        tenants_root: "/nonexistent".into(),
        docker: docker.clone(),
        vhosts_root: root.join("vhosts"),
    });
    let _ = Command::new("docker")
        .args(["rm", "-f", CONTAINER, "bp-webmail"])
        .output();
    for v in [
        "bp-webmail-sessions",
        "bp-webmail-tasks",
        "bp-webmail-config",
        "bp-webmail-data",
    ] {
        let _ = Command::new("docker")
            .args(["volume", "rm", "-f", v])
            .output();
    }
    for v in ["bp-mail-etc", "bp-mail-data"] {
        let _ = Command::new("docker")
            .args(["volume", "rm", "-f", v])
            .output();
    }

    // The server starts, is set up once, and says so the second time.
    let hello = |hostname: &str| Operation::EnsureMailServer {
        hostname: hostname.into(),
        certificate_pem: None,
        private_key_pem: None,
    };
    assert_eq!(
        ensure_mail_server(context.clone(), hello("mail.example.com"))
            .await
            .unwrap(),
        Output::MailServerReady { created: true }
    );
    assert_eq!(
        ensure_mail_server(context.clone(), hello("mail.example.com"))
            .await
            .unwrap(),
        Output::MailServerReady { created: false }
    );
    assert_eq!(
        ensure_mail_server(context.clone(), hello("not a name"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidArguments
    );
    // The administrator's login is kept where only the agent reads it.
    let admin = std::fs::read_to_string(root.join("secrets/stalwart-admin")).unwrap();
    assert!(admin.contains(':') && admin.len() > 12);
    // IMAP over TLS answers on its published port with the certificate made for the name.
    let tls = Command::new("sh")
        .args([
            "-c",
            &format!(
                "echo QUIT | timeout 8 openssl s_client -connect 127.0.0.1:{} -servername mail.example.com 2>/dev/null | grep -E 'subject='",
                std::env::var("BUILTPANEL_AGENT_MAIL_IMAP_PORT").unwrap()
            ),
        ])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&tls.stdout).contains("mail.example.com"),
        "{:?}",
        tls
    );

    // A domain: the records it needs for mail are given back.
    let set_domain =
        |catch_all: Option<&str>, per_hour: Option<u32>, held: bool| Operation::SetMailDomain {
            domain: "clientmail.org".into(),
            catch_all: catch_all.map(str::to_owned),
            per_hour,
            held,
        };
    let Output::MailDomainSet { records } =
        set_mail_domain(context.clone(), set_domain(None, Some(100), false))
            .await
            .unwrap()
    else {
        panic!("domain")
    };
    let mx = records.iter().find(|r| r.rtype == "MX").unwrap();
    assert_eq!(
        (mx.name.as_str(), mx.content.as_str(), mx.priority),
        ("@", "mail.example.com", Some(10))
    );
    assert!(
        records
            .iter()
            .any(|r| r.name == "@" && r.content.starts_with("v=spf1")),
        "{records:?}"
    );
    assert!(
        records
            .iter()
            .any(|r| r.name == "_dmarc" && r.content.starts_with("v=DMARC1"))
    );
    let dkim: Vec<_> = records
        .iter()
        .filter(|r| r.name.ends_with("._domainkey") && r.content.starts_with("v=DKIM1"))
        .collect();
    assert_eq!(dkim.len(), 2, "an Ed25519 and an RSA key: {records:?}");
    // Again: the same records, nothing new made.
    let Output::MailDomainSet { records: again } =
        set_mail_domain(context.clone(), set_domain(None, Some(100), false))
            .await
            .unwrap()
    else {
        panic!("domain")
    };
    assert_eq!(records, again);
    for bad in ["x' || true || '", "no spaces.com", ""] {
        let err = set_mail_domain(
            context.clone(),
            Operation::SetMailDomain {
                domain: bad.into(),
                catch_all: None,
                per_hour: None,
                held: false,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArguments, "{bad}");
    }

    // Mailboxes: made, found again, quota kept, bad ones refused.
    let mailbox = |local: &str, password: Option<&str>, quota: u64, aliases: &[&str]| {
        Operation::UpsertMailbox {
            domain: "clientmail.org".into(),
            local: local.into(),
            password: password.map(|p| DbPassword(p.into())),
            quota_bytes: quota,
            aliases: aliases.iter().map(|a| (*a).to_owned()).collect(),
        }
    };
    assert_eq!(
        upsert_mailbox(
            context.clone(),
            mailbox("alice", Some("Alice-Pass-1234!"), 1 << 20, &["ally"])
        )
        .await
        .unwrap(),
        Output::MailboxSet { created: true }
    );
    assert_eq!(
        upsert_mailbox(context.clone(), mailbox("alice", None, 1 << 20, &["ally"]))
            .await
            .unwrap(),
        Output::MailboxSet { created: false }
    );
    for (op, why) in [
        (
            mailbox("bob", None, 1 << 20, &[]),
            "a new mailbox needs a password",
        ),
        (mailbox("bob", Some("weak"), 1 << 20, &[]), "weak password"),
        (
            mailbox("Bob Smith", Some("Bob-Pass-1234!"), 1 << 20, &[]),
            "bad name",
        ),
        (
            mailbox("bob", Some("Bob-Pass-1234!"), 1 << 20, &["x'y"]),
            "bad alias",
        ),
    ] {
        assert_eq!(
            upsert_mailbox(context.clone(), op).await.unwrap_err().code,
            ErrorCode::InvalidArguments,
            "{why}"
        );
    }
    assert_eq!(
        probe(
            &script,
            &["login", "alice@clientmail.org", "Alice-Pass-1234!"]
        ),
        "ok"
    );
    assert_eq!(
        probe(
            &script,
            &["login", "alice@clientmail.org", "Wrong-Pass-1234!"]
        ),
        "no"
    );
    // A new password is the one that works.
    upsert_mailbox(
        context.clone(),
        mailbox("alice", Some("Alice-New-Pass-5678!"), 1 << 20, &["ally"]),
    )
    .await
    .unwrap();
    assert_eq!(
        probe(
            &script,
            &["login", "alice@clientmail.org", "Alice-Pass-1234!"]
        ),
        "no"
    );
    assert_eq!(
        probe(
            &script,
            &["login", "alice@clientmail.org", "Alice-New-Pass-5678!"]
        ),
        "ok"
    );

    // Signing in as the user with the administrator's login (what a webmail single sign-on
    // uses): the administrator's name after a `%`, with the administrator's secret.
    let (admin_user, admin_secret) = admin.trim().split_once(':').unwrap();
    assert_eq!(
        probe(
            &script,
            &[
                "login",
                &format!("alice@clientmail.org%{admin_user}"),
                admin_secret
            ]
        ),
        "ok",
        "the administrator can sign in as a mailbox"
    );
    assert_eq!(
        probe(
            &script,
            &[
                "login",
                &format!("alice@clientmail.org%{admin_user}"),
                "Wrong-Secret-1234!"
            ]
        ),
        "no"
    );
    // Webmail: one link signs in as the mailbox, once.
    {
        use sha2::{Digest as _, Sha256};
        let token = "5d".repeat(32);
        let hash: String = Sha256::digest(token.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let bad = webmail::open_webmail(
            context.clone(),
            Operation::OpenWebmail {
                address: "alice@clientmail.org".into(),
                token_hash: "nope".into(),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(bad.code, ErrorCode::InvalidArguments);
        let opened = webmail::open_webmail(
            context.clone(),
            Operation::OpenWebmail {
                address: "alice@clientmail.org".into(),
                token_hash: hash,
            },
        )
        .await
        .unwrap();
        assert!(matches!(opened, Output::WebmailOpened { .. }), "{opened:?}");
        let shown = probe(&script, &["webmail", &token]);
        assert!(
            shown["url"]
                .as_str()
                .is_some_and(|u| u.contains("_task=mail"))
                && shown["roundcube"] == true,
            "webmail signs in: {shown}"
        );
        // The link does not work a second time.
        let again = probe(&script, &["webmail", &token]);
        assert_eq!(again["http"], 403, "{again}");
        let _ = Command::new("docker")
            .args(["rm", "-f", "bp-webmail"])
            .output();
    }
    // Mail arrives, also under an alias; a test spam is marked for what it is.
    let pw = "Alice-New-Pass-5678!";
    assert_eq!(
        probe(
            &script,
            &["send", "alice@clientmail.org", "hello", "a normal message"]
        ),
        "accepted"
    );
    assert_eq!(
        probe(
            &script,
            &[
                "send",
                "ally@clientmail.org",
                "by alias",
                "to the other name"
            ]
        ),
        "accepted"
    );
    assert_eq!(
        probe(&script, &["gtube", "alice@clientmail.org"]),
        "accepted"
    );
    // (Only the test spam; a normal message from a sender whose mail is in order is not spam.)
    let read = heads(&probe(&script, &["read", "alice@clientmail.org", pw]));
    let subjects: Vec<&str> = read
        .iter()
        .filter_map(|(_, h)| h.lines().find_map(|l| l.strip_prefix("Subject: ")))
        .collect();
    assert!(
        subjects.contains(&"hello") && subjects.contains(&"by alias"),
        "{subjects:?}"
    );
    let spam = read
        .iter()
        .find(|(_, h)| h.contains("Subject: spam test"))
        .expect("the test spam is kept");
    assert_eq!(spam.0, "Junk Mail", "it is put with the junk");
    assert!(
        spam.1.to_ascii_uppercase().contains("GTUBE"),
        "and says why: {}",
        spam.1.replace("\r\n", " | ")
    );

    // A message bigger than the mailbox may hold is not kept: it is refused, or it waits in the
    // queue to be tried again and, the server's schedule running out, is returned to its sender.
    // It is never silently lost, and it is not put in a mailbox that is full.
    let big = probe(
        &script,
        &["send", "alice@clientmail.org", "too big", "x", "1500000"],
    );
    if !big.as_str().unwrap().starts_with("refused") {
        let mut waiting = false;
        for _ in 0..20 {
            let Output::MailReport { queue, .. } =
                report_mail(context.clone(), Operation::ReportMail)
                    .await
                    .unwrap()
            else {
                panic!("report")
            };
            if queue.iter().any(|m| {
                m.recipients
                    .iter()
                    .any(|r| r.contains("alice@clientmail.org"))
                    && m.size_bytes > 1_000_000
            }) {
                waiting = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        assert!(
            waiting,
            "a message that does not fit waits in the queue: {big}"
        );
    }
    let after = heads(&probe(&script, &["read", "alice@clientmail.org", pw]));
    assert!(
        !after.iter().any(|(_, h)| h.contains("Subject: too big")),
        "it was not kept"
    );
    // What waits to go out is cleared, so that the next check is about the next thing.
    let Output::MailReport { queue, .. } = report_mail(context.clone(), Operation::ReportMail)
        .await
        .unwrap()
    else {
        panic!("report")
    };
    let ids: Vec<String> = queue.iter().map(|m| m.id.clone()).collect();
    if !ids.is_empty() {
        mail_queue_action(
            context.clone(),
            Operation::MailQueueAction {
                action: "delete".into(),
                ids,
            },
        )
        .await
        .unwrap();
    }

    // Forwarders and the catch-all.
    let forward = |list: &[(&str, &[&str])]| Operation::SetMailForwarders {
        domain: "clientmail.org".into(),
        forwarders: list
            .iter()
            .map(|(l, to)| MailForwarder {
                local: (*l).to_owned(),
                to: to.iter().map(|t| (*t).to_owned()).collect(),
            })
            .collect(),
    };
    assert_eq!(
        set_mail_forwarders(
            context.clone(),
            forward(&[("sales", &["alice@clientmail.org"])])
        )
        .await
        .unwrap(),
        Output::MailForwardersSet { count: 1 }
    );
    assert_eq!(
        probe(
            &script,
            &["send", "sales@clientmail.org", "via forwarder", "hi"]
        ),
        "accepted"
    );
    set_mail_domain(
        context.clone(),
        set_domain(Some("alice@clientmail.org"), Some(100), false),
    )
    .await
    .unwrap();
    assert_eq!(
        probe(
            &script,
            &["send", "anything@clientmail.org", "caught", "hi"]
        ),
        "accepted"
    );
    let read = heads(&probe(&script, &["read", "alice@clientmail.org", pw]));
    let all: String = read.iter().map(|(_, h)| h.as_str()).collect();
    assert!(all.contains("Subject: via forwarder"), "{all}");
    assert!(all.contains("Subject: caught"), "{all}");
    // Taken away, the forwarder no longer takes mail (the catch-all would, so it goes too).
    set_mail_domain(context.clone(), set_domain(None, Some(100), false))
        .await
        .unwrap();
    set_mail_forwarders(context.clone(), forward(&[]))
        .await
        .unwrap();
    let gone = probe(&script, &["send", "sales@clientmail.org", "after", "hi"]);
    assert!(gone.as_str().unwrap().starts_with("refused"), "{gone}");

    // An automatic reply is sent to whoever writes, and is on the queue to go out.
    set_mail_autoresponder(
        context.clone(),
        Operation::SetMailAutoresponder {
            domain: "clientmail.org".into(),
            local: "alice".into(),
            enabled: true,
            subject: "Away".into(),
            body: "I am away until Monday.".into(),
            from: None,
            until: None,
        },
    )
    .await
    .unwrap();
    // Somebody who writes gets the reply (here a local user, who is sure to be believed).
    upsert_mailbox(
        context.clone(),
        mailbox("carol", Some("Carol-Pass-1234!"), 1 << 20, &[]),
    )
    .await
    .unwrap();
    assert_eq!(
        probe(
            &script,
            &[
                "sendauth",
                "carol@clientmail.org",
                "Carol-Pass-1234!",
                "alice@clientmail.org",
                "ping",
                "are you there"
            ]
        ),
        "accepted"
    );
    let mut replied = false;
    for _ in 0..20 {
        let carol = heads(&probe(
            &script,
            &["read", "carol@clientmail.org", "Carol-Pass-1234!"],
        ));
        if carol.iter().any(|(_, h)| h.contains("Subject: Away")) {
            replied = true;
            break;
        }
    }
    assert!(replied, "the automatic reply came back");
    // The reply is turned off again.
    set_mail_autoresponder(
        context.clone(),
        Operation::SetMailAutoresponder {
            domain: "clientmail.org".into(),
            local: "alice".into(),
            enabled: false,
            subject: "Away".into(),
            body: "I am away until Monday.".into(),
            from: None,
            until: None,
        },
    )
    .await
    .unwrap();
    // Mailboxes show their size, and a message that is on its way out shows in the queue.
    let Output::MailReport { usage, .. } = report_mail(context.clone(), Operation::ReportMail)
        .await
        .unwrap()
    else {
        panic!("report")
    };
    assert!(
        usage
            .iter()
            .find(|u| u.address == "alice@clientmail.org")
            .unwrap()
            .used_bytes
            > 0
    );
    // (Held for an hour by the sender's own request, so that it is still there to be seen.)
    probe(
        &script,
        &[
            "sendauth",
            "carol@clientmail.org",
            "Carol-Pass-1234!",
            "someone@external.example",
            "outgoing",
            "to the world",
            "hold",
        ],
    );
    let mut queued = Vec::new();
    for _ in 0..20 {
        let Output::MailReport { queue, .. } = report_mail(context.clone(), Operation::ReportMail)
            .await
            .unwrap()
        else {
            panic!("report")
        };
        if queue.iter().any(|m| {
            m.recipients
                .iter()
                .any(|r| r.contains("someone@external.example"))
        }) {
            queued = queue;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    assert!(!queued.is_empty(), "an outgoing message waits in the queue");
    // The queue can be retried and emptied.
    let ids: Vec<String> = queued.iter().map(|m| m.id.clone()).collect();
    let Output::MailQueueChanged { count } = mail_queue_action(
        context.clone(),
        Operation::MailQueueAction {
            action: "flush".into(),
            ids: ids.clone(),
        },
    )
    .await
    .unwrap() else {
        panic!("flush")
    };
    assert!(count <= ids.len());
    assert_eq!(
        mail_queue_action(
            context.clone(),
            Operation::MailQueueAction {
                action: "burn".into(),
                ids: vec![]
            }
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::InvalidArguments
    );
    mail_queue_action(
        context.clone(),
        Operation::MailQueueAction {
            action: "delete".into(),
            ids,
        },
    )
    .await
    .unwrap();
    let Output::MailReport { queue, .. } = report_mail(context.clone(), Operation::ReportMail)
        .await
        .unwrap()
    else {
        panic!("report")
    };
    assert!(
        queue
            .iter()
            .all(|m| !m.sender.contains("alice@clientmail.org")),
        "{queue:?}"
    );

    // The outgoing limit: set, held, lifted.
    let admin_login = admin.trim();
    let limit = |expected: Option<(u64, u64)>| {
        let (user, secret) = admin_login.split_once(':').unwrap();
        let body = serde_json::json!({
            "using": ["urn:ietf:params:jmap:core", "urn:stalwart:jmap"],
            "methodCalls": [["x:MtaOutboundThrottle/get", {"ids": null}, "c1"]],
        });
        let out = Command::new("curl")
            .args([
                "-s",
                "-u",
                &format!("{user}:{secret}"),
                "-H",
                "Content-Type: application/json",
                "-d",
                &body.to_string(),
                &format!(
                    "http://127.0.0.1:{}/jmap/",
                    builtpanel_node_agent::ops::mail::api_port()
                ),
            ])
            .output()
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let found = v["methodResponses"][0][1]["list"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["description"] == "builtpanel:clientmail.org")
            .map(|t| {
                (
                    t["rate"]["count"].as_u64().unwrap(),
                    t["rate"]["period"].as_u64().unwrap(),
                )
            });
        assert_eq!(found, expected);
    };
    limit(Some((100, 3_600_000)));
    set_mail_domain(context.clone(), set_domain(None, Some(20), false))
        .await
        .unwrap();
    limit(Some((20, 3_600_000)));
    set_mail_domain(context.clone(), set_domain(None, Some(20), true))
        .await
        .unwrap();
    limit(Some((1, 86_400_000)));
    set_mail_domain(context.clone(), set_domain(None, None, false))
        .await
        .unwrap();
    limit(None);

    // Removing: the mailbox, then the domain with what is left.
    assert_eq!(
        drop_mailbox(
            context.clone(),
            Operation::DropMailbox {
                domain: "clientmail.org".into(),
                local: "alice".into()
            }
        )
        .await
        .unwrap(),
        Output::MailboxDropped { existed: true }
    );
    assert_eq!(
        drop_mailbox(
            context.clone(),
            Operation::DropMailbox {
                domain: "clientmail.org".into(),
                local: "alice".into()
            }
        )
        .await
        .unwrap(),
        Output::MailboxDropped { existed: false }
    );
    upsert_mailbox(
        context.clone(),
        mailbox("carol", Some("Carol-Pass-1234!"), 1 << 20, &[]),
    )
    .await
    .unwrap();
    assert_eq!(
        remove_mail_domain(
            context.clone(),
            Operation::RemoveMailDomain {
                domain: "clientmail.org".into()
            }
        )
        .await
        .unwrap(),
        Output::MailDomainRemoved { existed: true }
    );
    assert_eq!(
        remove_mail_domain(
            context.clone(),
            Operation::RemoveMailDomain {
                domain: "clientmail.org".into()
            }
        )
        .await
        .unwrap(),
        Output::MailDomainRemoved { existed: false }
    );
    assert_eq!(
        probe(
            &script,
            &["login", "carol@clientmail.org", "Carol-Pass-1234!"]
        ),
        "no"
    );

    // Clean up.
    let _ = Command::new("docker")
        .args(["rm", "-f", CONTAINER])
        .output();
    for v in ["bp-mail-etc", "bp-mail-data"] {
        let _ = Command::new("docker")
            .args(["volume", "rm", "-f", v])
            .output();
    }
    let _ = Command::new("docker")
        .args(["network", "rm", "bp-mail-net"])
        .output();
    let _ = std::fs::remove_dir_all(&root);
}
