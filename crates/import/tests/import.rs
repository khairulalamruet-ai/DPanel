//! Reading backups of other panels from archives built in memory.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::io::Write as _;

use builtpanel_import::{
    DomainKind, Engine, ImportError, Limits, SourceKind, parse_zone, read_backup,
};

fn tar_gz(files: &[(&str, &str)]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (path, content) in files {
        let mut h = tar::Header::new_gnu();
        h.set_size(content.len() as u64);
        h.set_mode(0o644);
        h.set_entry_type(tar::EntryType::Regular);
        tar.append_data(&mut h, path, content.as_bytes()).unwrap();
    }
    let raw = tar.into_inner().unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&raw).unwrap();
    gz.finish().unwrap()
}

const ZONE: &str = r#"$TTL 14400
@	14400	IN	SOA	ns1.old.example. hostmaster.example.com. (
		2024010101 ; serial
		3600 1800 1209600 86400 )
example.com.	14400	IN	NS	ns1.old.example.
example.com.	14400	IN	NS	ns2.old.example.
example.com.	14400	IN	A	203.0.113.10
www		14400	IN	CNAME	example.com.
mail		IN	A	203.0.113.11
example.com.	14400	IN	MX	10	mail.example.com.
example.com.	14400	IN	TXT	"v=spf1 +a +mx ip4:203.0.113.10 ~all"
_dmarc		IN	TXT	"v=DMARC1; p=none"
default._domainkey	IN	TXT	( "v=DKIM1; k=rsa; "
		"p=MIIBIjANBgkq" )
_sip._tcp	IN	SRV	10 5 5060 sip.example.com.
sub.example.com.	IN	A	203.0.113.12
"#;

fn cpmove() -> Vec<u8> {
    tar_gz(&[
        ("cpmove-alice/version", "pkgacct version 11.110\n"),
        (
            "cpmove-alice/cp/alice",
            "USER=alice\nDNS=example.com\nDNS1=parked.example.org\nCONTACTEMAIL=alice@example.net\nPLAN=gold\nSTARTDATE=1700000000\nLOCALE=en\n",
        ),
        (
            "cpmove-alice/userdata/main",
            "---\nmain_domain: example.com\naddon_domains:\n  shop.example.net: shop.example.com\nparked_domains:\n  - parked.example.org\nsub_domains:\n  - blog.example.com\n",
        ),
        (
            "cpmove-alice/userdata/example.com",
            "---\ndocumentroot: /home/alice/public_html\nhomedir: /home/alice\nphpversion: ea-php81\n",
        ),
        (
            "cpmove-alice/userdata/shop.example.net",
            "documentroot: /home/alice/shop\nphpversion: ea-php74\n",
        ),
        (
            "cpmove-alice/mysql/alice_wp.sql",
            "-- dump\nCREATE TABLE t (id int);\n",
        ),
        ("cpmove-alice/psql/alice_pg.sql", "-- pg dump\n"),
        (
            "cpmove-alice/mysql.sql",
            "GRANT USAGE ON *.* TO 'alice_wpu'@'localhost' IDENTIFIED BY PASSWORD '*2470C0C06DEE42FD1618BB99005ADCA2EC9D1E19';\nGRANT ALL PRIVILEGES ON `alice\\_wp`.* TO 'alice_wpu'@'localhost';\nGRANT SELECT, INSERT ON `alice\\_other`.* TO 'alice_wpu'@'localhost';\n",
        ),
        ("cpmove-alice/dnszones/example.com.db", ZONE),
        (
            "cpmove-alice/cron/alice",
            "MAILTO=alice@example.net\n# comment\n*/5 * * * * /usr/bin/php /home/alice/cron.php\n@daily /home/alice/backup.sh\n",
        ),
        (
            "cpmove-alice/homedir/etc/example.com/shadow",
            "info:$6$salt$hashhash:19000::::::\nsales:$6$salt2$hash2:19000::::::\n",
        ),
        (
            "cpmove-alice/homedir/etc/example.com/quota",
            "info:104857600\n",
        ),
        ("cpmove-alice/homedir/mail/example.com/info/cur/1", "mail"),
        (
            "cpmove-alice/homedir/public_html/index.php",
            "<?php echo 1;",
        ),
        ("cpmove-alice/homedir/public_html/a.txt", "abcd"),
        (
            "cpmove-alice/valiases/example.com",
            "support: bob@example.net, carol@example.org\n*: :fail: no such user\npiper: |/usr/bin/x\n",
        ),
        ("cpmove-alice/sslkeys/x.key", "key"),
    ])
}

#[test]
fn a_cpanel_backup_becomes_the_common_model() {
    let m = read_backup(cpmove().as_slice(), Limits::default()).unwrap();
    assert_eq!(m.source.kind, SourceKind::Cpanel);
    assert_eq!(m.source.version.as_deref(), Some("pkgacct version 11.110"));
    assert_eq!(m.account.username, "alice");
    assert_eq!(m.account.email.as_deref(), Some("alice@example.net"));
    assert_eq!(m.account.plan.as_deref(), Some("gold"));
    assert_eq!(m.account.created_at, Some(1_700_000_000));
    assert_eq!(m.account.main_domain.as_deref(), Some("example.com"));

    let d = |n: &str| {
        m.domains
            .iter()
            .find(|d| d.name == n)
            .unwrap_or_else(|| panic!("no {n}"))
    };
    assert_eq!(d("example.com").kind, DomainKind::Main);
    assert_eq!(d("example.com").docroot.as_deref(), Some("public_html"));
    assert_eq!(d("example.com").php_version.as_deref(), Some("8.1"));
    assert_eq!(d("shop.example.net").kind, DomainKind::Addon);
    assert_eq!(d("shop.example.net").docroot.as_deref(), Some("shop"));
    assert_eq!(d("shop.example.net").php_version.as_deref(), Some("7.4"));
    assert_eq!(d("blog.example.com").kind, DomainKind::Sub);
    assert_eq!(d("parked.example.org").kind, DomainKind::Alias);
    assert_eq!(m.domains.len(), 4, "{:?}", m.domains);

    assert_eq!(m.databases.len(), 2);
    let wp = m.databases.iter().find(|d| d.name == "alice_wp").unwrap();
    assert_eq!(wp.engine, Engine::Mysql);
    assert_eq!(wp.dump.as_ref().unwrap().as_str(), "mysql/alice_wp.sql");
    assert!(wp.dump_bytes.unwrap() > 0);
    assert_eq!(
        m.databases
            .iter()
            .find(|d| d.name == "alice_pg")
            .unwrap()
            .engine,
        Engine::Postgres
    );
    let user = &m.db_users[0];
    assert_eq!(user.name, "alice_wpu");
    assert_eq!(
        user.password_hash.as_deref(),
        Some("*2470C0C06DEE42FD1618BB99005ADCA2EC9D1E19")
    );
    assert_eq!(user.grants.len(), 2);
    assert_eq!(user.grants[0].database, "alice_wp");
    assert!(user.grants[0].all_privileges);
    assert_eq!(user.grants[1].database, "alice_other");
    assert_eq!(user.grants[1].privileges, ["SELECT", "INSERT"]);

    assert_eq!(m.cron_jobs.len(), 2);
    assert_eq!(m.cron_jobs[0].schedule, "*/5 * * * *");
    assert_eq!(m.cron_jobs[0].command, "/usr/bin/php /home/alice/cron.php");
    assert_eq!(m.cron_jobs[1].schedule, "@daily");

    assert_eq!(m.mailboxes.len(), 2);
    let info = m.mailboxes.iter().find(|b| b.local == "info").unwrap();
    assert_eq!(info.domain, "example.com");
    assert_eq!(info.quota_mb, Some(100));
    assert_eq!(info.password_hash.as_deref(), Some("$6$salt$hashhash"));
    assert_eq!(
        info.maildir.as_ref().unwrap().as_str(),
        "homedir/mail/example.com/info"
    );
    assert!(
        m.mailboxes
            .iter()
            .find(|b| b.local == "sales")
            .unwrap()
            .maildir
            .is_none()
    );
    assert_eq!(m.forwarders.len(), 1);
    assert_eq!(
        m.forwarders[0].destinations,
        ["bob@example.net", "carol@example.org"]
    );
    assert!(m.warnings.iter().any(|w| w.code == "forwarder_skipped"));

    let files = m.files.as_ref().unwrap();
    assert_eq!(files.root.as_str(), "homedir");
    assert_eq!(
        files.bytes,
        "<?php echo 1;".len() as u64
            + 4
            + 4
            + "info:$6$salt$hashhash:19000::::::\nsales:$6$salt2$hash2:19000::::::\n".len() as u64
            + "info:104857600\n".len() as u64
    );
    assert!(
        m.warnings
            .iter()
            .any(|w| w.code == "custom_certificate_skipped")
    );

    // Valid, and the JSON form round-trips.
    m.validate().unwrap();
    let json = serde_json::to_string(&m).unwrap();
    let back: builtpanel_import::ImportModel = serde_json::from_str(&json).unwrap();
    assert_eq!(back, m);
}

#[test]
fn the_zone_reader_handles_the_usual_zone_file() {
    let (z, skipped) = parse_zone("example.com", ZONE);
    assert!(skipped.is_empty(), "{skipped:?}");
    assert_eq!(z.name, "example.com");
    let find = |name: &str, t: &str| {
        z.records
            .iter()
            .filter(|r| r.name == name && r.rtype == t)
            .collect::<Vec<_>>()
    };
    // SOA and the apex name servers are not carried over.
    assert!(
        z.records
            .iter()
            .all(|r| r.rtype != "SOA" && !(r.rtype == "NS" && r.name == "@"))
    );
    assert_eq!(find("@", "A")[0].content, "203.0.113.10");
    assert_eq!(find("@", "A")[0].ttl, Some(14400));
    assert_eq!(find("www", "CNAME")[0].content, "example.com");
    assert_eq!(find("mail", "A")[0].ttl, Some(14400), "the $TTL default");
    assert_eq!(find("@", "MX")[0].priority, Some(10));
    assert_eq!(find("@", "MX")[0].content, "mail.example.com");
    assert_eq!(
        find("@", "TXT")[0].content,
        "v=spf1 +a +mx ip4:203.0.113.10 ~all"
    );
    assert_eq!(
        find("default._domainkey", "TXT")[0].content,
        "v=DKIM1; k=rsa; p=MIIBIjANBgkq"
    );
    let srv = &find("_sip._tcp", "SRV")[0];
    assert_eq!(
        (srv.priority, srv.content.as_str()),
        (Some(10), "5 5060 sip.example.com")
    );
    assert_eq!(find("sub", "A")[0].content, "203.0.113.12");
    // A type that is not carried over is named, not lost silently.
    let (_, skipped) = parse_zone("example.com", "@ IN HINFO a b\n@ IN A 192.0.2.1\n");
    assert_eq!(skipped.len(), 1);
    assert!(skipped[0].contains("HINFO"));
}

#[test]
fn a_cyberpanel_backup_becomes_the_common_model() {
    let meta = r#"<?xml version="1.0" ?>
<VirtualHost>
  <metaVersion>2.0</metaVersion>
  <VERSION>2.3</VERSION>
  <domain>Example.com</domain>
  <phpSelection>PHP 8.1</phpSelection>
  <externalApp>examp1</externalApp>
  <adminEmail>owner@example.net</adminEmail>
  <ChildDomains>
    <domain><domain>blog.example.com</domain><phpSelection>PHP 7.4</phpSelection><path>/home/example.com/public_html/blog</path></domain>
  </ChildDomains>
  <Databases>
    <database><dbName>examp1_wp</dbName><dbUser>examp1_u</dbUser><password>*ABCDEF</password></database>
  </Databases>
  <Emails>
    <emailAccount><email>info@example.com</email><password>$6$x$y</password></emailAccount>
  </Emails>
</VirtualHost>"#;
    let archive = tar_gz(&[
        ("meta.xml", meta),
        ("examp1_wp.sql", "-- dump\n"),
        ("example.com/public_html/index.php", "x"),
    ]);
    let m = read_backup(archive.as_slice(), Limits::default()).unwrap();
    assert_eq!(m.source.kind, SourceKind::Cyberpanel);
    assert_eq!(m.account.username, "examp1");
    assert_eq!(m.account.main_domain.as_deref(), Some("example.com"));
    assert_eq!(m.domains[0].php_version.as_deref(), Some("8.1"));
    assert_eq!(m.domains[1].name, "blog.example.com");
    assert_eq!(m.domains[1].docroot.as_deref(), Some("public_html/blog"));
    assert_eq!(
        m.databases[0].dump.as_ref().unwrap().as_str(),
        "examp1_wp.sql"
    );
    assert_eq!(m.db_users[0].grants[0].database, "examp1_wp");
    assert_eq!(m.mailboxes[0].local, "info");
    assert_eq!(m.files.as_ref().unwrap().root.as_str(), "example.com");
    assert_eq!(m.files.as_ref().unwrap().entries, 1);
}

#[test]
fn hostile_or_foreign_archives_are_refused() {
    // Not a backup of a known panel.
    let other = tar_gz(&[("readme.txt", "hello")]);
    assert_eq!(
        read_backup(other.as_slice(), Limits::default()).unwrap_err(),
        ImportError::Unrecognised
    );
    // Not an archive at all.
    assert!(matches!(
        read_backup(
            &b"this is not a tar file at all, just text........"[..],
            Limits::default()
        ),
        Err(ImportError::Archive(_) | ImportError::Unrecognised)
    ));
    // A path that climbs out of the archive.
    let mut tar = tar::Builder::new(Vec::new());
    let mut h = tar::Header::new_gnu();
    h.set_size(1);
    h.set_entry_type(tar::EntryType::Regular);
    let name = b"cpmove-x/../../etc/passwd";
    h.as_old_mut().name[..name.len()].copy_from_slice(name);
    h.set_cksum();
    tar.append(&h, &b"x"[..]).unwrap();
    let raw = tar.into_inner().unwrap();
    assert!(matches!(
        read_backup(raw.as_slice(), Limits::default()),
        Err(ImportError::UnsafePath(_))
    ));
    // Too many entries.
    let many = tar_gz(&[("a/1", "x"), ("a/2", "x"), ("a/3", "x")]);
    let tight = Limits {
        max_entries: 2,
        ..Limits::default()
    };
    assert!(matches!(
        read_backup(many.as_slice(), tight),
        Err(ImportError::TooBig(_))
    ));
    // A model with a hostile name is not accepted.
    let evil = tar_gz(&[
        ("cpmove-x/cp/x", "USER=x\nDNS=bad_domain!.com\n"),
        ("cpmove-x/userdata/main", "main_domain: bad_domain!.com\n"),
        ("cpmove-x/mysql/a;DROP.sql", "x"),
    ]);
    let m = read_backup(evil.as_slice(), Limits::default()).unwrap();
    assert!(m.domains.is_empty() && m.databases.is_empty());
    assert!(m.warnings.iter().any(|w| w.code == "domain_skipped"));
    assert!(m.warnings.iter().any(|w| w.code == "database_skipped"));
}

#[test]
fn archive_paths_cannot_leave_the_archive() {
    for bad in [
        "",
        "/etc/passwd",
        "a/../b",
        "a//b",
        "./a",
        "a\\b",
        "..",
        "a/.",
    ] {
        assert!(builtpanel_import::ArchivePath::new(bad).is_err(), "{bad}");
    }
    assert!(builtpanel_import::ArchivePath::new("homedir/mail/x.com/info").is_ok());
    assert!(serde_json::from_str::<builtpanel_import::ArchivePath>("\"../x\"").is_err());
}

mod dry_run {
    use std::collections::BTreeSet;

    use builtpanel_import::{
        Action, Existing, Limits, OnConflict, Options, Resolution, plan, read_backup,
    };

    fn model() -> builtpanel_import::ImportModel {
        read_backup(super::cpmove().as_slice(), Limits::default()).unwrap()
    }

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn on_an_empty_panel_everything_is_planned_in_order() {
        let p = plan(
            &model(),
            &Existing::default(),
            &Options {
                mail_supported: true,
                ..Options::default()
            },
        );
        assert!(p.ok, "{:?}", p.blockers);
        assert!(p.conflicts.is_empty());
        let actions: Vec<_> = p.steps.iter().map(|s| s.action.clone()).collect();
        assert_eq!(actions[0], Action::CreateAccount);
        assert!(actions.contains(&Action::RestoreFiles));
        assert_eq!(
            actions.iter().filter(|a| **a == Action::AddDomain).count(),
            4
        );
        assert_eq!(
            actions
                .iter()
                .filter(|a| **a == Action::CreateDatabase)
                .count(),
            1
        );
        assert_eq!(
            actions.iter().filter(|a| **a == Action::ImportDump).count(),
            1
        );
        assert_eq!(
            actions
                .iter()
                .filter(|a| **a == Action::CreateDbUser)
                .count(),
            1
        );
        assert_eq!(
            actions
                .iter()
                .filter(|a| **a == Action::CreateDnsZone)
                .count(),
            1
        );
        assert_eq!(
            actions.iter().filter(|a| **a == Action::AddCronJob).count(),
            2
        );
        assert_eq!(
            actions
                .iter()
                .filter(|a| **a == Action::CreateMailbox)
                .count(),
            2
        );
        assert_eq!(
            actions
                .iter()
                .filter(|a| **a == Action::CreateForwarder)
                .count(),
            1
        );
        // The account comes before what belongs to it; databases before their users.
        let pos = |a: Action| actions.iter().position(|x| *x == a).unwrap();
        assert!(pos(Action::CreateDatabase) < pos(Action::CreateDbUser));
        assert!(p.dump_bytes > 0 && p.file_bytes > 0);
        // Database names keep the (same) account prefix.
        assert!(p.model.databases.iter().any(|d| d.name == "alice_wp"));
        assert_eq!(p.model.account.username, "alice");
        // Without a mail service the mail is named as left out, not lost silently.
        let p = plan(&model(), &Existing::default(), &Options::default());
        assert!(p.ok);
        assert!(p.skipped.iter().any(|s| s.kind == "mail"));
        assert!(p.model.mailboxes.is_empty());
    }

    #[test]
    fn a_taken_account_name_stops_the_import_unless_it_may_be_renamed() {
        let existing = Existing {
            usernames: set(&["alice", "alice2"]),
            ..Existing::default()
        };
        let p = plan(&model(), &existing, &Options::default());
        assert!(!p.ok);
        assert_eq!(p.conflicts[0].kind, "username");
        assert_eq!(p.conflicts[0].resolution, Resolution::Blocks);
        let p = plan(
            &model(),
            &existing,
            &Options {
                on_conflict: OnConflict::Rename,
                ..Options::default()
            },
        );
        assert!(p.ok, "{:?}", p.blockers);
        assert_eq!(p.model.account.username, "alice3");
        // The databases follow the new account name.
        assert!(
            p.model.databases.iter().any(|d| d.name == "alice3_wp"),
            "{:?}",
            p.model.databases
        );
        assert!(p.model.db_users[0].name.starts_with("alice3_"));
        assert!(
            p.model.db_users[0]
                .grants
                .iter()
                .all(|g| g.database.starts_with("alice3_"))
        );
        let renamed = p
            .steps
            .iter()
            .find(|s| s.action == Action::CreateAccount)
            .unwrap();
        assert_eq!(renamed.renamed_from.as_deref(), Some("alice"));
        // The operator may also just choose a name.
        let p = plan(
            &model(),
            &existing,
            &Options {
                username: Some("Carol".into()),
                ..Options::default()
            },
        );
        assert!(p.ok);
        assert_eq!(p.model.account.username, "carol");
        // A name that cannot be an account name is a blocker, not a panic.
        let p = plan(
            &model(),
            &Existing::default(),
            &Options {
                username: Some("1".into()),
                ..Options::default()
            },
        );
        assert!(!p.ok);
    }

    #[test]
    fn a_domain_in_the_way_blocks_or_is_left_out_and_takes_its_dns_with_it() {
        let existing = Existing {
            domains: set(&["shop.example.net", "example.com"]),
            ..Existing::default()
        };
        // The main domain cannot be skipped: it is the account.
        let p = plan(
            &model(),
            &existing,
            &Options {
                on_conflict: OnConflict::Skip,
                ..Options::default()
            },
        );
        assert!(!p.ok);
        assert!(p.blockers.iter().any(|b| b.contains("example.com")));
        // Renaming does not help with a domain.
        let p = plan(
            &model(),
            &existing,
            &Options {
                on_conflict: OnConflict::Rename,
                ..Options::default()
            },
        );
        assert!(!p.ok);
        // Another main domain, and the addon is skipped: the rest goes on.
        let p = plan(
            &model(),
            &Existing {
                domains: set(&["shop.example.net"]),
                ..Existing::default()
            },
            &Options {
                on_conflict: OnConflict::Skip,
                main_domain: Some("new.example.org".into()),
                ..Options::default()
            },
        );
        assert!(p.ok, "{:?}", p.blockers);
        assert!(p.skipped.iter().any(|s| s.name == "shop.example.net"));
        assert!(p.model.domains.iter().all(|d| d.name != "shop.example.net"));
        assert!(p.model.domains.iter().any(|d| d.name == "new.example.org"));
        // The zone of example.com is not carried (that domain is no longer there).
        assert!(p.model.dns_zones.is_empty());
        assert!(p.skipped.iter().any(|s| s.kind == "dns_zone"));
    }

    #[test]
    fn a_database_name_in_use_is_settled_by_the_policy() {
        let existing = Existing {
            db_names: set(&["alice_wp", "alice_wpu"]),
            ..Existing::default()
        };
        let p = plan(&model(), &existing, &Options::default());
        assert!(!p.ok);
        assert!(
            p.conflicts
                .iter()
                .any(|c| c.kind == "database" && c.resolution == Resolution::Blocks)
        );
        let p = plan(
            &model(),
            &existing,
            &Options {
                on_conflict: OnConflict::Rename,
                ..Options::default()
            },
        );
        assert!(p.ok);
        let db = p
            .model
            .databases
            .iter()
            .find(|d| {
                d.dump
                    .as_ref()
                    .is_some_and(|x| x.as_str() == "mysql/alice_wp.sql")
            })
            .unwrap();
        assert_eq!(db.name, "alice_wp2");
        // The user's grant follows the database's new name.
        assert!(
            p.model.db_users[0]
                .grants
                .iter()
                .any(|g| g.database == "alice_wp2")
        );
        let p = plan(
            &model(),
            &existing,
            &Options {
                on_conflict: OnConflict::Skip,
                ..Options::default()
            },
        );
        assert!(p.ok);
        assert!(p.model.databases.iter().all(|d| d.name != "alice_wp"));
        // The user was skipped as well, so nothing refers to the missing database.
        assert!(p.model.db_users.is_empty());
    }

    #[test]
    fn what_is_not_wanted_is_not_planned() {
        let p = plan(
            &model(),
            &Existing::default(),
            &Options {
                include: builtpanel_import::Include {
                    files: false,
                    databases: false,
                    dns: false,
                    cron: false,
                    mail: false,
                },
                ..Options::default()
            },
        );
        assert!(p.ok);
        let actions: Vec<_> = p.steps.iter().map(|s| s.action.clone()).collect();
        assert!(
            actions
                .iter()
                .all(|a| matches!(a, Action::CreateAccount | Action::AddDomain))
        );
        assert!(
            p.model.files.is_none() && p.model.databases.is_empty() && p.model.cron_jobs.is_empty()
        );
    }
}

#[test]
fn a_file_and_a_folder_can_be_taken_out_of_the_archive() {
    let archive = cpmove();
    let mut dump = Vec::new();
    let n = builtpanel_import::copy_file(&archive[..], "mysql/alice_wp.sql", &mut dump).unwrap();
    assert_eq!(dump, b"-- dump\nCREATE TABLE t (id int);\n");
    assert_eq!(n, dump.len() as u64);
    assert!(builtpanel_import::copy_file(&archive[..], "mysql/none.sql", &mut Vec::new()).is_err());

    // The files under public_html, packed again without that folder.
    let mut packed = Vec::new();
    let done = builtpanel_import::repack(
        &archive[..],
        |p| p.strip_prefix("homedir/public_html/").map(str::to_owned),
        &mut packed,
    )
    .unwrap();
    assert_eq!(done.entries, 2, "{done:?}");
    assert_eq!(done.bytes, 17);
    let mut names: Vec<String> = tar::Archive::new(flate2::read::GzDecoder::new(&packed[..]))
        .entries()
        .unwrap()
        .map(|e| e.unwrap().path().unwrap().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["a.txt", "index.php"]);
}

#[test]
fn repacking_refuses_a_mapping_that_leaves_the_folder() {
    let archive = cpmove();
    let result = builtpanel_import::repack(
        &archive[..],
        |p| {
            p.strip_prefix("homedir/public_html/")
                .map(|r| format!("../{r}"))
        },
        Vec::new(),
    );
    assert!(result.is_err());
}
