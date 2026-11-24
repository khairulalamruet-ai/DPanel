//! Sending: a mail through the panel's mail server, a webhook to a URL somebody gave.
//!
//! A webhook URL is chosen by an account's owner, so what it may reach is limited: only `https`,
//! no redirect is followed, and the name is looked up by a resolver of our own that throws away
//! every address that is not a public one (the panel's own machine, the private networks, a
//! cloud's metadata address). The check is made on the address that is then connected to, so a
//! name that changes its answer between the check and the connection gets nowhere.

use std::{
    net::{IpAddr, Ipv4Addr},
    time::Duration,
};

use lettre::{
    Message, SmtpTransport, Transport,
    message::{MultiPart, SinglePart},
    transport::smtp::authentication::Credentials,
};
use ureq::{
    Error as UreqError,
    config::Config,
    http::Uri,
    unversioned::{
        resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver},
        transport::{DefaultConnector, NextTimeout},
    },
};

/// How long a mail or a webhook may take.
const TIMEOUT: Duration = Duration::from_secs(20);

/// Whether an address is one of the public internet (and not the machine itself, a private or
/// shared network, a link-local or a documentation address).
#[must_use]
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => public_v4(v4),
        IpAddr::V6(v6) => {
            // An IPv4 address in IPv6 clothes is judged as the IPv4 address it is.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return public_v4(v4);
            }
            let segments = v6.segments();
            !(v6.is_unspecified()
                || v6.is_loopback()
                || v6.is_multicast()
                // fc00::/7 unique local, fe80::/10 link-local, 2001:db8::/32 documentation,
                // 64:ff9b::/96 (NAT64: reaches IPv4 hosts), 100::/64 discard.
                || segments[0] & 0xfe00 == 0xfc00
                || segments[0] & 0xffc0 == 0xfe80
                || (segments[0] == 0x2001 && segments[1] == 0x0db8)
                || (segments[0] == 0x0064 && segments[1] == 0xff9b)
                || segments[0] == 0x0100)
        }
    }
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        // 100.64.0.0/10 shared address space, 192.0.0.0/24, 198.18.0.0/15 benchmarking,
        // 240.0.0.0/4 reserved.
        || (o[0] == 100 && (o[1] & 0xc0) == 64)
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)
        || (o[0] == 198 && (o[1] & 0xfe) == 18)
        || o[0] >= 240)
}

/// A resolver that only ever answers with public addresses.
#[derive(Debug, Default)]
struct PublicOnly(DefaultResolver);

impl Resolver for PublicOnly {
    fn resolve(
        &self,
        uri: &Uri,
        config: &Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, UreqError> {
        let found = self.0.resolve(uri, config, timeout)?;
        let mut kept = self.0.empty();
        for address in found.iter().filter(|a| is_public(a.ip())) {
            kept.push(*address);
        }
        if kept.is_empty() {
            return Err(UreqError::HostNotFound);
        }
        Ok(kept)
    }
}

/// An HTTP client for webhooks: public addresses only, no redirects, a time limit.
fn webhook_agent() -> ureq::Agent {
    let config = Config::builder()
        .timeout_global(Some(TIMEOUT))
        .max_redirects(0)
        .https_only(true)
        .http_status_as_error(false)
        .build();
    ureq::Agent::with_parts(config, DefaultConnector::default(), PublicOnly::default())
}

/// Whether a text is a webhook URL we accept: `https`, a host name or a public address, no
/// login in it.
///
/// # Errors
/// What is wrong with it.
pub fn check_webhook_url(url: &str) -> Result<(), &'static str> {
    let uri: Uri = url.parse().map_err(|_| "this is not a URL")?;
    if uri.scheme_str() != Some("https") {
        return Err("a webhook URL must start with https://");
    }
    let authority = uri.authority().ok_or("the URL has no host")?;
    if authority.as_str().contains('@') {
        return Err("the URL must not hold a login");
    }
    let host = uri.host().ok_or("the URL has no host")?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = host.parse::<IpAddr>() {
        if !is_public(ip) {
            return Err("this address is not on the public internet");
        }
    } else if host.eq_ignore_ascii_case("localhost") || !host.contains('.') {
        return Err("use a public host name");
    }
    Ok(())
}

/// `sha256=<hex>` of the HMAC-SHA256 of `body` under `secret`: what a receiver recomputes to know
/// that the call is ours.
#[must_use]
pub fn signature(secret: &[u8], body: &[u8]) -> String {
    use std::fmt::Write as _;
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret);
    let tag = ring::hmac::sign(&key, body);
    let mut out = String::from("sha256=");
    for byte in tag.as_ref() {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Calls a webhook.
///
/// # Errors
/// A short reason (never the URL's query or the secret).
pub fn post_webhook(url: &str, secret: &[u8], event: &str, body: &str) -> Result<(), String> {
    check_webhook_url(url).map_err(str::to_owned)?;
    let mut response = webhook_agent()
        .post(url)
        .header("Content-Type", "application/json")
        .header("X-BuiltPanel-Event", event)
        .header("X-BuiltPanel-Signature", signature(secret, body.as_bytes()))
        .send(body)
        .map_err(|e| format!("the call failed: {e}"))?;
    let status = response.status().as_u16();
    // Whatever came back is not read: it may be big, and it is not ours.
    let _ = response.body_mut().read_to_string();
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(format!("the receiver answered {status}"))
    }
}

/// The mail server, as it is used to send (the password in plain text).
#[derive(Debug, Clone)]
pub struct MailServer {
    /// Host name.
    pub host: String,
    /// Port.
    pub port: u16,
    /// `starttls`, `tls` or `none`.
    pub security: String,
    /// Login, if any.
    pub username: Option<String>,
    /// Password, if any.
    pub password: Option<String>,
    /// The address mail comes from.
    pub from_address: String,
    /// The name mail comes from.
    pub from_name: Option<String>,
}

/// Sends one mail (plain text and HTML in one message).
///
/// # Errors
/// A short reason.
pub fn send_mail(
    server: &MailServer,
    to: &str,
    subject: &str,
    text: &str,
    html: &str,
) -> Result<(), String> {
    let from = match &server.from_name {
        Some(name) if !name.is_empty() => format!(
            "{} <{}>",
            name.replace(['"', '<', '>', '\r', '\n'], ""),
            server.from_address
        ),
        _ => server.from_address.clone(),
    };
    let message = Message::builder()
        .from(
            from.parse()
                .map_err(|_| "the sender's address is not valid".to_owned())?,
        )
        .to(to
            .parse()
            .map_err(|_| "the recipient's address is not valid".to_owned())?)
        .subject(subject.replace(['\r', '\n'], " "))
        .multipart(
            MultiPart::alternative()
                .singlepart(SinglePart::plain(text.to_owned()))
                .singlepart(SinglePart::html(html.to_owned())),
        )
        .map_err(|e| format!("the message could not be made: {e}"))?;
    let builder = match server.security.as_str() {
        "tls" => SmtpTransport::relay(&server.host),
        "starttls" => SmtpTransport::starttls_relay(&server.host),
        _ => Ok(SmtpTransport::builder_dangerous(&server.host)),
    }
    .map_err(|e| format!("the mail server cannot be used: {e}"))?;
    let mut builder = builder.port(server.port).timeout(Some(TIMEOUT));
    if let (Some(user), Some(password)) = (&server.username, &server.password) {
        builder = builder.credentials(Credentials::new(user.clone(), password.clone()));
    }
    builder
        .build()
        .send(&message)
        .map(|_| ())
        .map_err(|e| format!("the mail server refused: {e}"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn only_public_addresses_are_public() {
        for ip in [
            "8.8.8.8",
            "1.1.1.1",
            "93.184.216.34",
            "2606:4700:4700::1111",
        ] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "127.0.0.1",
            "10.0.0.5",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
            "198.18.0.1",
            "192.0.2.1",
            "240.0.0.1",
            "::1",
            "::",
            "fe80::1",
            "fc00::1",
            "fd12::1",
            "2001:db8::1",
            "64:ff9b::7f00:1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "ff02::1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn a_webhook_url_must_be_https_and_public() {
        assert!(check_webhook_url("https://hooks.example.com/in/abc?x=1").is_ok());
        assert!(check_webhook_url("https://8.8.8.8/x").is_ok());
        for bad in [
            "http://hooks.example.com/",
            "https://localhost/",
            "https://intranet/",
            "https://127.0.0.1/",
            "https://[::1]/",
            "https://10.1.2.3/hook",
            "https://169.254.169.254/latest/meta-data/",
            "https://user:pass@hooks.example.com/",
            "ftp://hooks.example.com/",
            "not a url",
            "https://",
        ] {
            assert!(check_webhook_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_signature_is_the_hmac_receivers_can_check() {
        // RFC 4231 test case 2.
        assert_eq!(
            signature(b"Jefe", b"what do ya want for nothing?"),
            "sha256=5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }
}
