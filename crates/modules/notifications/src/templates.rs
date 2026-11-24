//! What a notification says (T18.04): each event in English and in Bangla, with the panel's name
//! (its branding) in the subject and at the foot of a mail.

use builtpanel_kernel::Event;
use serde_json::{Value, json};

/// A notification as it is worded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    /// `info`, `warning` or `critical`.
    pub severity: &'static str,
    /// One line.
    pub subject: String,
    /// The text.
    pub body: String,
    /// The ids the notification is about.
    pub data: Value,
}

/// Who an event concerns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
    /// The owner of the hosting account it is about.
    TenantOwner(uuid::Uuid),
    /// Every administrator.
    Admins,
}

/// What the wording needs to know besides the event.
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    /// `en` or `bn`.
    pub locale: &'a str,
    /// The panel's name.
    pub panel: &'a str,
    /// The name of the node the event is about, when it has one.
    pub node: Option<&'a str>,
    /// The time of the wording (Unix seconds), to tell "ends soon" from "has ended".
    pub now: i64,
}

/// The event types that can be sent to a channel, and who may ask for them.
pub const CLIENT_EVENTS: &[&str] = &["certificate_expiring", "backup_failed"];
/// Everything an administrator may ask for.
pub const ADMIN_EVENTS: &[&str] = &[
    "backup_failed",
    "node_offline",
    "node_service_down",
    "node_service_recovered",
    "panel_update",
];

/// Who is told about an event and how it is worded, or `None` for an event that is nobody's
/// business here.
#[must_use]
pub fn render(event: &Event, ctx: &Context<'_>) -> Option<(Audience, String, Rendered)> {
    let bn = ctx.locale == "bn";
    let panel = ctx.panel;
    let node = ctx.node.unwrap_or("?");
    match event {
        Event::CertificateExpiring {
            tenant_id,
            domain_id,
            name,
            not_after,
        } => {
            let ended = *not_after <= ctx.now;
            let (subject, body) = match (bn, ended) {
                (false, false) => (
                    format!("[{panel}] The certificate of {name} ends soon"),
                    format!(
                        "The SSL certificate of {name} ends soon. It is renewed on its own; if it cannot be, the site will show a warning to visitors. Open the domain's SSL page to see why."
                    ),
                ),
                (false, true) => (
                    format!("[{panel}] The certificate of {name} has ended"),
                    format!(
                        "The SSL certificate of {name} has ended: visitors see a warning. Open the domain's SSL page to renew it."
                    ),
                ),
                (true, false) => (
                    format!("[{panel}] {name}-এর সার্টিফিকেটের মেয়াদ শেষ হতে চলেছে"),
                    format!(
                        "{name}-এর SSL সার্টিফিকেটের মেয়াদ শীঘ্রই শেষ হবে। এটি নিজে থেকেই নবায়ন হয়; না হলে দর্শকেরা সাইটে সতর্কবার্তা দেখবেন। কারণ দেখতে ডোমেইনের SSL পাতা খুলুন।"
                    ),
                ),
                (true, true) => (
                    format!("[{panel}] {name}-এর সার্টিফিকেটের মেয়াদ শেষ হয়ে গেছে"),
                    format!(
                        "{name}-এর SSL সার্টিফিকেটের মেয়াদ শেষ: দর্শকেরা সতর্কবার্তা দেখছেন। নবায়ন করতে ডোমেইনের SSL পাতা খুলুন।"
                    ),
                ),
            };
            Some((
                Audience::TenantOwner(*tenant_id),
                format!("certificate:{domain_id}:{not_after}"),
                Rendered {
                    severity: if ended { "critical" } else { "warning" },
                    subject,
                    body,
                    data: json!({"domain_id": domain_id, "name": name, "not_after": not_after}),
                },
            ))
        }
        Event::BackupFailed {
            backup_id,
            tenant_id,
            error,
        } => {
            let (subject, body) = if bn {
                (
                    format!("[{panel}] ব্যাকআপ ব্যর্থ হয়েছে"),
                    format!("একটি ব্যাকআপ সম্পন্ন হয়নি। কারণ: {error}\nব্যাকআপের পাতা খুলে আবার চেষ্টা করুন।"),
                )
            } else {
                (
                    format!("[{panel}] A backup failed"),
                    format!(
                        "A backup did not finish. Reason: {error}\nOpen the backups page and try again."
                    ),
                )
            };
            let audience = tenant_id.map_or(Audience::Admins, Audience::TenantOwner);
            Some((
                audience,
                format!("backup:{backup_id}"),
                Rendered {
                    severity: "critical",
                    subject,
                    body,
                    data: json!({"backup_id": backup_id, "tenant_id": tenant_id}),
                },
            ))
        }
        Event::NodeOffline { node_id, name } => {
            let (subject, body) = if bn {
                (
                    format!("[{panel}] নোড {name} অফলাইন"),
                    format!(
                        "নোড {name} আর প্যানেলের সঙ্গে যুক্ত নেই। এর অ্যাকাউন্টগুলোর কাজ আটকে থাকতে পারে। সার্ভার ও agent চালু আছে কি না দেখুন।"
                    ),
                )
            } else {
                (
                    format!("[{panel}] Node {name} is offline"),
                    format!(
                        "Node {name} is no longer connected to the panel. Changes to its accounts wait until it is back. Check that the server and its agent are running."
                    ),
                )
            };
            Some((
                Audience::Admins,
                format!("node_offline:{node_id}:{}", ctx.now / 300),
                Rendered {
                    severity: "critical",
                    subject,
                    body,
                    data: json!({"node_id": node_id, "name": name}),
                },
            ))
        }
        Event::NodeServiceDown {
            node_id,
            service,
            state,
        } => {
            let (subject, body) = if bn {
                (
                    format!("[{panel}] {node}-এ {service} চলছে না"),
                    format!("নোড {node}-এর সেবা {service} চলছে না ({state})।"),
                )
            } else {
                (
                    format!("[{panel}] {service} is down on {node}"),
                    format!("The service {service} of node {node} is not running ({state})."),
                )
            };
            Some((
                Audience::Admins,
                format!("service_down:{node_id}:{service}:{}", ctx.now / 300),
                Rendered {
                    severity: "warning",
                    subject,
                    body,
                    data: json!({"node_id": node_id, "service": service, "state": state}),
                },
            ))
        }
        Event::NodeServiceRecovered { node_id, service } => {
            let (subject, body) = if bn {
                (
                    format!("[{panel}] {node}-এ {service} আবার চলছে"),
                    format!("নোড {node}-এর সেবা {service} আবার চালু হয়েছে।"),
                )
            } else {
                (
                    format!("[{panel}] {service} is back on {node}"),
                    format!("The service {service} of node {node} is running again."),
                )
            };
            Some((
                Audience::Admins,
                format!("service_up:{node_id}:{service}:{}", ctx.now / 300),
                Rendered {
                    severity: "info",
                    subject,
                    body,
                    data: json!({"node_id": node_id, "service": service}),
                },
            ))
        }
        Event::PanelUpdate {
            version,
            stage,
            auto_in_days,
        } => {
            let (severity, subject, body) = match (stage.as_str(), bn) {
                ("available", false) => {
                    let auto = match auto_in_days {
                        Some(0) => " It will be installed automatically at the next update hour unless you switch automatic updates off in Settings.".to_owned(),
                        Some(d) => format!(" It will be installed automatically in about {d} day(s) unless you do it sooner or switch automatic updates off in Settings."),
                        None => String::new(),
                    };
                    (
                        "info",
                        format!("[{panel}] Version {version} is available"),
                        format!(
                            "A new version of the panel, {version}, is ready. Open Updates in the admin area and press Update now.{auto}"
                        ),
                    )
                }
                ("available", true) => {
                    let auto = match auto_in_days {
                        Some(0) => " আপডেটের পরের নির্ধারিত ঘণ্টায় এটি নিজে থেকেই বসানো হবে; না চাইলে সেটিংসে স্বয়ংক্রিয় আপডেট বন্ধ করুন।".to_owned(),
                        Some(d) => format!(" আপনি আগে না করলে প্রায় {d} দিনের মধ্যে এটি নিজে থেকেই বসানো হবে; না চাইলে সেটিংসে স্বয়ংক্রিয় আপডেট বন্ধ করুন।"),
                        None => String::new(),
                    };
                    (
                        "info",
                        format!("[{panel}] {version} সংস্করণ এসেছে"),
                        format!(
                            "প্যানেলের নতুন সংস্করণ {version} তৈরি। অ্যাডমিন এলাকার আপডেট পেজে গিয়ে \"এখনই আপডেট\" চাপুন।{auto}"
                        ),
                    )
                }
                ("started", false) => (
                    "info",
                    format!("[{panel}] Updating to {version}"),
                    format!(
                        "The automatic update to {version} has started. The panel restarts for a moment."
                    ),
                ),
                ("started", true) => (
                    "info",
                    format!("[{panel}] {version}-এ আপডেট শুরু"),
                    format!(
                        "{version}-এ স্বয়ংক্রিয় আপডেট শুরু হয়েছে। প্যানেল কয়েক মুহূর্তের জন্য রিস্টার্ট হবে।"
                    ),
                ),
                ("failed", false) => (
                    "critical",
                    format!("[{panel}] The update to {version} did not work"),
                    format!(
                        "The update to {version} failed, so the old version is still running. Open Updates in the admin area to see why."
                    ),
                ),
                ("failed", true) => (
                    "critical",
                    format!("[{panel}] {version}-এ আপডেট হয়নি"),
                    format!(
                        "{version}-এ আপডেট ব্যর্থ হয়েছে, তাই পুরনো সংস্করণই চলছে। কারণ দেখতে অ্যাডমিন এলাকার আপডেট পেজ খুলুন।"
                    ),
                ),
                _ => return None,
            };
            Some((
                Audience::Admins,
                format!("panel_update:{version}:{stage}"),
                Rendered {
                    severity,
                    subject,
                    body,
                    data: json!({"version": version, "stage": stage}),
                },
            ))
        }
        _ => None,
    }
}

/// The mail of a notification: a plain text and a small HTML page, both with the panel's name.
#[must_use]
pub fn mail(subject: &str, body: &str, panel: &str, bn: bool) -> (String, String) {
    let foot = if bn {
        format!("— {panel}। এই বার্তাটি আপনার বিজ্ঞপ্তির সেটিং অনুযায়ী পাঠানো হয়েছে।")
    } else {
        format!("— {panel}. You get this message because of your notification settings.")
    };
    let text = format!("{body}\n\n{foot}\n");
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let html = format!(
        "<!doctype html><html><body style=\"font-family:system-ui,sans-serif;max-width:560px;margin:24px auto;color:#222\">\
         <h2 style=\"margin:0 0 12px\">{}</h2><p style=\"white-space:pre-line;line-height:1.5\">{}</p>\
         <hr style=\"border:0;border-top:1px solid #ddd;margin:20px 0\"><p style=\"color:#666;font-size:13px\">{}</p></body></html>",
        esc(subject),
        esc(body),
        esc(&foot)
    );
    (text, html)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use uuid::Uuid;

    fn ctx<'a>(locale: &'a str) -> Context<'a> {
        Context {
            locale,
            panel: "Acme Host",
            node: Some("node1"),
            now: 1_000_000,
        }
    }

    #[test]
    fn every_event_has_both_languages_and_carries_the_panels_name() {
        let tenant = Uuid::from_u128(1);
        let events = [
            Event::CertificateExpiring {
                tenant_id: tenant,
                domain_id: Uuid::from_u128(2),
                name: "a.example".into(),
                not_after: 2_000_000,
            },
            Event::CertificateExpiring {
                tenant_id: tenant,
                domain_id: Uuid::from_u128(2),
                name: "a.example".into(),
                not_after: 10,
            },
            Event::BackupFailed {
                backup_id: Uuid::from_u128(3),
                tenant_id: Some(tenant),
                error: "no space".into(),
            },
            Event::BackupFailed {
                backup_id: Uuid::from_u128(3),
                tenant_id: None,
                error: "no space".into(),
            },
            Event::NodeOffline {
                node_id: Uuid::from_u128(4),
                name: "node1".into(),
            },
            Event::NodeServiceDown {
                node_id: Uuid::from_u128(4),
                service: "database".into(),
                state: "stopped".into(),
            },
            Event::NodeServiceRecovered {
                node_id: Uuid::from_u128(4),
                service: "database".into(),
            },
            Event::PanelUpdate {
                version: "0.1.15".into(),
                stage: "available".into(),
                auto_in_days: Some(3),
            },
            Event::PanelUpdate {
                version: "0.1.15".into(),
                stage: "started".into(),
                auto_in_days: None,
            },
            Event::PanelUpdate {
                version: "0.1.15".into(),
                stage: "failed".into(),
                auto_in_days: None,
            },
        ];
        for event in &events {
            let en = render(event, &ctx("en")).unwrap();
            let bn = render(event, &ctx("bn")).unwrap();
            assert!(
                en.2.subject.starts_with("[Acme Host]") && bn.2.subject.starts_with("[Acme Host]")
            );
            assert_ne!(en.2.subject, bn.2.subject);
            assert_ne!(en.2.body, bn.2.body);
            // The same event is the same notification in either language.
            assert_eq!((en.0, &en.1, en.2.severity), (bn.0, &bn.1, bn.2.severity));
        }
        // Ended is not "ends soon".
        let soon = render(&events[0], &ctx("en")).unwrap().2;
        let ended = render(&events[1], &ctx("en")).unwrap().2;
        assert_eq!((soon.severity, ended.severity), ("warning", "critical"));
        assert!(ended.subject.contains("has ended"));
        // A backup of the panel itself is for the administrators, an account's for its owner.
        assert_eq!(
            render(&events[2], &ctx("en")).unwrap().0,
            Audience::TenantOwner(tenant)
        );
        assert_eq!(render(&events[3], &ctx("en")).unwrap().0, Audience::Admins);
        // Events nobody is told about make nothing.
        assert!(render(&Event::TenantCreated { tenant_id: tenant }, &ctx("en")).is_none());
    }

    #[test]
    fn a_mail_has_the_panels_name_and_cannot_carry_markup_of_its_own() {
        let (text, html) = mail("A <b>subject</b>", "Body & more", "Acme Host", false);
        assert!(text.contains("Acme Host") && text.contains("Body & more"));
        assert!(html.contains("A &lt;b&gt;subject&lt;/b&gt;") && html.contains("Body &amp; more"));
        assert!(!html.contains("<b>subject"));
        let (bn_text, _) = mail("বিষয়", "লেখা", "Acme Host", true);
        assert!(bn_text.contains("বিজ্ঞপ্তির সেটিং"));
    }
}
