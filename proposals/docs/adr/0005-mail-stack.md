# ADR 0005 — The mail stack

Status: **accepted** (decided before T13.02: Stalwart, Roundcube webmail; SnappyMail may follow as an option)

## Context
The panel must offer mailboxes, forwarders, DKIM/SPF/DMARC, spam filtering, quotas, webmail with
single sign-on and brute-force protection, per hosting account and per domain, managed by typed
operations, and shipped as containers we can pin by digest (as for the other roles).

## Options
| | docker-mailserver | Stalwart Mail Server | Own Postfix + Dovecot + Rspamd |
|---|---|---|---|
| Language / shape | shell + Postfix + Dovecot + Rspamd in one image | one Rust binary (SMTP, IMAP, JMAP, spam filter, DKIM) | three daemons we configure |
| Managing users and domains | files and a `setup` script; no real API | HTTP management API; accounts, domains, aliases, DKIM keys, quotas | our own config files and reloads |
| Fit for typed operations | we would write config files and reload | call an API with typed bodies | same as docker-mailserver |
| Maturity | years in production | younger, quick moving, active security response | the most proven parts, the most work |
| Footprint | large | small | medium |
| Webmail | Roundcube separately | Roundcube separately (IMAP) or JMAP clients | Roundcube separately |
| Licence | MIT | AGPL-3.0 (with a commercial option) | permissive |

## Proposal
Use **Stalwart**, one container per mail node, managed through its API by the node agent, with
Roundcube for webmail (single sign-on by a one-time token, as for phpMyAdmin). Reasons: an API
fits our "typed operation" rule better than editing files; one binary is easier to pin, scan and
update; the mail stack stays in the memory-safe family of the rest of the system.

## Things the human must weigh
- **Licence:** AGPL-3.0. Running unmodified Stalwart as a separate service is allowed; any
  change we make to it must be published. A commercial licence is available.
- **Maturity risk:** if a feature we depend on is missing or unstable, fall back to
  docker-mailserver (same operations, different handler); the typed operation layer hides which
  one is behind it.
- **Deliverability** (reverse DNS, IP reputation, port 25) is a deployment matter, not a stack
  matter, and is the same with every option.

## Consequences
If accepted: T13.02 provisions one Stalwart container (SMTP 25/465/587, IMAP 993) with the
certificate from the panel's Caddy; T13.03 and later call its management API.

## As built (T13.02 - T13.12)
- **One Stalwart container per node** (`bp-mail`, image pinned by digest in `deploy/stack/images.toml`),
  set up and managed by the node agent through Stalwart's JMAP management API with typed operations
  (`EnsureMailServer`, `SetMailDomain`, `UpsertMailbox`, `SetMailForwarders`, `SetMailAutoresponder`,
  `ReportMail`, `MailQueueAction`). The mail administrator's secret is made on first use and stays
  in the agent's secret store; the control plane never sees it.
- **Control plane** (`crates/modules/mail`): the wanted state lives in `mail_domains`, `mailboxes`,
  `mail_forwarders`, each with a version counter; a 15-second tick sends the node what it does not
  have and writes the MX, SPF, DKIM and DMARC records the mail server asks for into the domain's
  zone (an owner's own SPF, DMARC or MX is never replaced; the admin's deliverability page says when
  it differs). Mailbox passwords are sealed with the panel's master key until the node has them.
- **Abuse control** (T13.07): a domain's outgoing limit follows the plan (`mail_per_hour`); a domain
  with more than `abuse_queue_threshold` messages waiting in the outgoing queue is held (one message a
  day), its queued messages are removed, and the owner and the administrators are told. An
  administrator releases it.
- **Spam** (T13.08): Stalwart's built-in filter plus a GTUBE rule; the test spam is filed as junk.
- **Webmail** (T13.09): Roundcube (`bp-webmail`), reached under the name set in the mail settings
  (`webmail_host`). A one-time link (5 minutes, used once) signs the client in through a small plugin;
  the node signs in as the mailbox with the mail administrator's login (`address%administrator`), so
  the mailbox's password is never needed.
- **Known limits:** the mail server's certificate is self-signed until the panel hands it a real one
  (the agent operation accepts a PEM already); Docker without privileged ports cannot bind 25/465/993
  (the agent's ports can be set by environment); rootless Docker hides the client address on port 25,
  so per-address bans on SMTP are not possible there; blacklist (RBL) checks and DMARC report parsing
  are not done yet; a domain removed from the panel is not yet removed from the mail server.
