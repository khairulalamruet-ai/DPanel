//! Web server configuration for a site (T06.03): the template an administrator
//! keeps, and what is made of it for one domain.
//!
//! The control plane renders; the node only writes what it is given. A template
//! is plain nginx configuration with these variables: `{{domain}}`,
//! `{{server_names}}`, `{{docroot}}`, `{{php_upstream}}`, and the two places a
//! domain's [`Policy`] goes, `{{policy_http}}` (before the `server` block) and
//! `{{policy}}` (inside it), and `{{custom}}` for the directives a person adds to
//! their own site (see [`check_custom_block`]). Every value that goes in comes from a checked type,
//! so no domain, path, header or address can carry a directive of its own into
//! the file.

use crate::{Docroot, Fqdn};

/// Where a tenant's web container sees its files.
pub const WEB_ROOT: &str = "/var/www/html";
/// Longest template.
pub const MAX_TEMPLATE_BYTES: usize = 32 * 1024;
/// Most names (the domain and its aliases) in one `server_name`.
pub const MAX_SERVER_NAMES: usize = 50;
/// Variables a template may use.
pub const VARIABLES: [&str; 7] = [
    "domain",
    "server_names",
    "docroot",
    "php_upstream",
    "policy_http",
    "policy",
    "custom",
];
/// Variables a template must use: without them it does not make a site.
const REQUIRED: [&str; 2] = ["server_names", "docroot"];

/// The template new installations start with.
pub const DEFAULT_NGINX_TEMPLATE: &str = r"# Managed by Built Panel for {{domain}}. Changes made here are overwritten.
{{policy_http}}
server {
    listen 8080;
    server_name {{server_names}};
    root {{docroot}};
    index index.php index.html index.htm;
    {{policy}}
    {{custom}}

    location / {
        try_files $uri $uri/ /index.php?$query_string;
    }

    location ~ \.php$ {
        try_files $uri =404;
        include fastcgi_params;
        fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;
        fastcgi_pass {{php_upstream}};
    }

    location ~ /\.(?!well-known) {
        deny all;
    }
}
";

/// The site configuration of a domain that shows one of the tenant's own applications
/// (Node.js, Python): the web server passes every request on to the application's container.
/// `{{php_upstream}}` is the application's `name:port` here.
pub const PROXY_NGINX_TEMPLATE: &str = r"# Managed by Built Panel for {{domain}} (application). Changes made here are overwritten.
{{policy_http}}
server {
    listen 8080;
    server_name {{server_names}};
    # The account's files stay in {{docroot}}; the application answers instead.
    client_max_body_size 64m;
    {{policy}}
    {{custom}}

    location / {
        proxy_pass http://{{php_upstream}};
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $http_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $http_x_forwarded_proto;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection $http_connection;
        proxy_read_timeout 300s;
    }
}
";

/// What is wrong with a template or with a value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct VhostError {
    /// Stable, for programs.
    pub code: &'static str,
    /// For people.
    pub message: String,
}

fn err(code: &'static str, message: impl Into<String>) -> VhostError {
    VhostError {
        code,
        message: message.into(),
    }
}

/// Directives no template may use: they read other files, load code or reach
/// outside the tenant's directory.
const FORBIDDEN: [&str; 8] = [
    "load_module",
    "alias",
    "perl",
    "perl_modules",
    "perl_require",
    "perl_set",
    "env",
    "ssi",
];

/// The statements of nginx configuration text, each as its words, read the way
/// nginx reads it: words end at white space, `;`, `{` and `}` unless quoted
/// (single or double quotes, backslash escapes inside and outside), and `#` starts
/// a comment only at the start of a word. Quotes must close and braces must match.
///
/// What a person writes is judged on these words and not on the raw text, so a
/// `#` or `;` inside a string cannot hide a directive from the checks.
fn statements(text: &str) -> Result<Vec<Vec<String>>, VhostError> {
    let bad = |what: &str| err("template_syntax", what.to_owned());
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut depth: i32 = 0;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if c == '\\' {
                word.push(c);
                if let Some(next) = chars.next() {
                    word.push(next);
                }
            } else if c == q {
                quote = None;
            } else {
                word.push(c);
            }
            continue;
        }
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            '"' | '\'' => {
                quote = Some(c);
                in_word = true;
            }
            '#' if !in_word => {
                for rest in chars.by_ref() {
                    if rest == '\n' {
                        break;
                    }
                }
            }
            '\\' => {
                in_word = true;
                word.push(c);
                if let Some(next) = chars.next() {
                    word.push(next);
                }
            }
            ';' | '{' | '}' => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
                if !words.is_empty() {
                    out.push(std::mem::take(&mut words));
                }
                match c {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
                if depth < 0 {
                    return Err(bad("A } has no {."));
                }
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if quote.is_some() {
        return Err(bad("A quote is never closed."));
    }
    if in_word {
        words.push(word);
    }
    if !words.is_empty() {
        out.push(words);
    }
    if depth != 0 {
        return Err(err("template_braces", "The braces { } do not match."));
    }
    Ok(out)
}

fn forbidden(name: &str, argument: Option<&str>) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower == "include" {
        // The one include a PHP site needs; any other file is out of reach.
        return argument != Some("fastcgi_params");
    }
    FORBIDDEN.contains(&lower.as_str())
        || lower.starts_with("js_")
        || lower.contains("_by_lua")
        || lower.starts_with("lua_")
}

/// Checks a template as an administrator saves it: well-formed, only known
/// variables, the required ones present, no forbidden directive, and the result
/// for a sample site is balanced.
///
/// # Errors
/// [`VhostError`] for the first problem.
pub fn check_template(body: &str) -> Result<(), VhostError> {
    if body.len() > MAX_TEMPLATE_BYTES {
        return Err(err("template_too_large", "The template is too large."));
    }
    if body.trim().is_empty() {
        return Err(err("template_empty", "The template is empty."));
    }
    if body
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
    {
        return Err(err(
            "template_characters",
            "The template holds control characters.",
        ));
    }
    let mut used = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            return Err(err("template_variable", "A {{ is never closed."));
        };
        let name = &after[..end];
        if !VARIABLES.contains(&name) {
            return Err(err(
                "unknown_variable",
                format!("{{{{{}}}}} is not a variable of a template.", name.trim()),
            ));
        }
        used.push(name);
        rest = &after[end + 2..];
    }
    if rest.contains("}}") {
        return Err(err("template_variable", "A }} has no {{."));
    }
    for required in REQUIRED {
        if !used.contains(&required) {
            return Err(err(
                "missing_variable",
                format!("The template must use {{{{{required}}}}}."),
            ));
        }
    }
    // Looked at with the variables filled in, so braces of the values count too.
    let sample = render_with(
        body,
        &Values::plain(
            "example.com".into(),
            "example.com www.example.com".into(),
            format!("{WEB_ROOT}/public_html"),
            "bp-sample-php:9000".into(),
        ),
    );
    for words in statements(&sample)? {
        if forbidden(&words[0], words.get(1).map(String::as_str)) {
            return Err(err(
                "forbidden_directive",
                format!("The directive `{}` is not allowed.", words[0].trim()),
            ));
        }
    }
    Ok(())
}

struct Values {
    domain: String,
    server_names: String,
    docroot: String,
    php_upstream: String,
    /// Lines for `{{policy_http}}` and `{{policy}}`.
    policy_http: Vec<String>,
    policy: Vec<String>,
    /// The lines of the person's own block, for `{{custom}}`.
    custom: Vec<String>,
}

impl Values {
    fn plain(domain: String, server_names: String, docroot: String, php_upstream: String) -> Self {
        Self {
            domain,
            server_names,
            docroot,
            php_upstream,
            policy_http: Vec::new(),
            policy: Vec::new(),
            custom: Vec::new(),
        }
    }
}

/// A line that is only a policy variable becomes the policy's lines (same
/// indentation), or disappears when there are none; anywhere else the lines are
/// joined with spaces.
fn put_lines(body: &str, name: &str, lines: &[String]) -> String {
    let token = format!("{{{{{name}}}}}");
    let mut out = String::with_capacity(body.len());
    for line in body.split_inclusive('\n') {
        if line.trim() == token {
            let indent = &line[..line.len() - line.trim_start().len()];
            for l in lines {
                out.push_str(indent);
                out.push_str(l);
                out.push('\n');
            }
        } else {
            out.push_str(&line.replace(&token, &lines.join(" ")));
        }
    }
    out
}

fn render_with(body: &str, values: &Values) -> String {
    let body = put_lines(body, "policy_http", &values.policy_http);
    let body = put_lines(&body, "policy", &values.policy);
    let body = put_lines(&body, "custom", &values.custom);
    body.replace("{{domain}}", &values.domain)
        .replace("{{server_names}}", &values.server_names)
        .replace("{{docroot}}", &values.docroot)
        .replace("{{php_upstream}}", &values.php_upstream)
}

/// A web container address, `name:port` (the PHP service of a tenant).
fn check_upstream(upstream: &str) -> Result<(), VhostError> {
    let ok = upstream.split_once(':').is_some_and(|(host, port)| {
        !host.is_empty()
            && host.len() <= 100
            && host
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && port.parse::<u16>().is_ok_and(|p| p > 0)
    });
    if ok {
        Ok(())
    } else {
        Err(err("invalid_upstream", "The PHP address is not name:port."))
    }
}

/// One site, as far as its web server configuration goes.
#[derive(Debug, Clone)]
pub struct Site<'a> {
    /// The domain the file is named for.
    pub domain: &'a Fqdn,
    /// Other names that show the same site.
    pub aliases: &'a [Fqdn],
    /// Where its files are.
    pub docroot: &'a Docroot,
    /// The PHP service, `name:port`.
    pub php_upstream: &'a str,
    /// Rate limit and response headers.
    pub policy: &'a Policy,
    /// The person's own directives (empty for none), already checked with
    /// [`check_custom_block`].
    pub custom: &'a str,
    /// An application of the tenant that answers for the site, `name:port`: when given the
    /// site is passed on to it ([`PROXY_NGINX_TEMPLATE`]) instead of served from files.
    pub proxy: Option<&'a str>,
}

/// Makes the configuration of one site from a template.
///
/// # Errors
/// [`VhostError`]: a bad template, too many names or a bad PHP address.
pub fn render(template: &str, site: &Site<'_>) -> Result<String, VhostError> {
    let (template, upstream) = match site.proxy {
        Some(app) => (PROXY_NGINX_TEMPLATE, app),
        None => (template, site.php_upstream),
    };
    check_template(template)?;
    check_upstream(upstream)?;
    let mut names: Vec<&str> = vec![site.domain.as_str()];
    for alias in site.aliases {
        if !names.contains(&alias.as_str()) {
            names.push(alias.as_str());
        }
    }
    if names.len() > MAX_SERVER_NAMES {
        return Err(err("too_many_names", "A site has too many names."));
    }
    let docroot = if site.docroot.as_str().is_empty() {
        WEB_ROOT.to_owned()
    } else {
        format!("{WEB_ROOT}/{}", site.docroot.as_str())
    };
    let mut values = Values::plain(
        site.domain.as_str().to_owned(),
        names.join(" "),
        docroot,
        upstream.to_owned(),
    );
    values.policy_http = site.policy.http_lines(site.domain);
    values.policy = site.policy.server_lines(site.domain);
    if !site.custom.trim().is_empty() {
        check_custom_block(site.custom)?;
        values.custom = site.custom.lines().map(str::to_owned).collect();
    }
    Ok(render_with(template, &values))
}

/// Longest block of a person's own directives.
pub const MAX_CUSTOM_BYTES: usize = 8 * 1024;
/// Most directives (statements) in one.
const MAX_CUSTOM_STATEMENTS: usize = 200;
/// The directives a person may use in their own block: what shapes the answers
/// of their own site. Nothing that names a file outside the site, another
/// address to send requests to, a log, a module or a script language.
const CUSTOM_ALLOWED: [&str; 24] = [
    "location",
    "if",
    "set",
    "rewrite",
    "return",
    "break",
    "try_files",
    "index",
    "error_page",
    "expires",
    "add_header",
    "deny",
    "allow",
    "gzip",
    "gzip_types",
    "gzip_min_length",
    "gzip_comp_level",
    "gzip_vary",
    "client_max_body_size",
    "autoindex",
    "charset",
    "default_type",
    "limit_except",
    "etag",
];

/// Checks the block of directives a person adds to their site: only the
/// directives in a short list, quotes and braces that match, and a size limit.
/// What is not on the list is refused, so a directive nobody thought of cannot be
/// used.
///
/// # Errors
/// [`VhostError`] for the first problem (`forbidden_directive` names the directive).
pub fn check_custom_block(text: &str) -> Result<(), VhostError> {
    if text.len() > MAX_CUSTOM_BYTES {
        return Err(err(
            "custom_too_large",
            format!("The block is at most {MAX_CUSTOM_BYTES} bytes."),
        ));
    }
    if text
        .chars()
        .any(|c| !(c == '\n' || c == '\t' || (' '..='~').contains(&c)))
    {
        return Err(err(
            "custom_characters",
            "The block holds characters other than printable ASCII.",
        ));
    }
    if text.contains("{{") || text.contains("}}") {
        return Err(err("custom_characters", "The block may not hold {{ or }}."));
    }
    let all = statements(text)?;
    if all.len() > MAX_CUSTOM_STATEMENTS {
        return Err(err(
            "custom_too_large",
            "The block has too many directives.",
        ));
    }
    for words in &all {
        let name = words[0].to_ascii_lowercase();
        if !CUSTOM_ALLOWED.contains(&name.as_str()) {
            return Err(err(
                "forbidden_directive",
                format!("The directive `{}` is not allowed here.", words[0]),
            ));
        }
        // Headers the panel sets itself stay its own.
        if name == "add_header"
            && words
                .get(1)
                .is_some_and(|h| RESERVED_HEADERS.contains(&h.to_ascii_lowercase().as_str()))
        {
            return Err(err(
                "reserved_header",
                format!("The header {} is set by the panel.", words[1]),
            ));
        }
    }
    Ok(())
}

/// Most response headers one domain may add.
pub const MAX_HEADERS: usize = 10;
/// Highest rate limit that can be set (requests per second per visitor).
pub const MAX_RATE_LIMIT: u16 = 1000;

/// Headers a person may not set: the panel sets them itself, or they would break
/// the response.
const RESERVED_HEADERS: [&str; 12] = [
    "strict-transport-security",
    "content-length",
    "content-type",
    "content-encoding",
    "transfer-encoding",
    "connection",
    "host",
    "set-cookie",
    "server",
    "date",
    "location",
    "x-accel-redirect",
];

/// One response header a domain adds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// The name, `X-Frame-Options`.
    pub name: String,
    /// The value, printable ASCII without `"`, `\` and `$`.
    pub value: String,
}

/// What a domain's web server does besides serving files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policy {
    /// Requests per second one visitor may make (`None`: no limit). Over the limit
    /// the answer is 429.
    pub rate_limit_per_second: Option<u16>,
    /// Response headers to add.
    pub headers: Vec<Header>,
    /// The page cache of the site, when it is on (T12.11).
    pub page_cache: Option<PageCache>,
}

/// Longest time a cached page is kept, in minutes (one day).
pub const MAX_CACHE_MINUTES: u16 = 1440;

/// A domain's page cache: pages the PHP site answers to visitors who are not
/// signed in are kept by the web server for a while and given again.
///
/// Never cached: requests that are not `GET` or `HEAD`, requests with an
/// `Authorization` header or a login/session cookie of the usual PHP
/// applications, the sign-in and admin paths of WordPress, and every answer that
/// sets a cookie or says `Cache-Control: private`/`no-cache` (nginx's own rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageCache {
    /// Minutes a page stays fresh, 1 to [`MAX_CACHE_MINUTES`].
    pub ttl_minutes: u16,
    /// Goes up each time the cache is purged: it is part of every key, so old
    /// pages are never found again and leave the cache by themselves.
    pub generation: u32,
}

impl PageCache {
    /// Checks the time.
    ///
    /// # Errors
    /// [`VhostError`] when the time is out of range.
    pub fn parse(ttl_minutes: u16, generation: u32) -> Result<Self, VhostError> {
        if ttl_minutes == 0 || ttl_minutes > MAX_CACHE_MINUTES {
            return Err(err(
                "invalid_cache_ttl",
                format!("A page stays cached for 1 to {MAX_CACHE_MINUTES} minutes."),
            ));
        }
        Ok(Self {
            ttl_minutes,
            generation,
        })
    }
}

impl Policy {
    /// Checks what a person gave.
    ///
    /// # Errors
    /// [`VhostError`] naming the first problem (`field` is in the message).
    pub fn parse(rate_limit: Option<u16>, headers: Vec<Header>) -> Result<Self, VhostError> {
        if rate_limit.is_some_and(|r| r == 0 || r > MAX_RATE_LIMIT) {
            return Err(err(
                "invalid_rate_limit",
                format!("The rate limit is 1 to {MAX_RATE_LIMIT} requests per second."),
            ));
        }
        if headers.len() > MAX_HEADERS {
            return Err(err(
                "too_many_headers",
                format!("A domain may add at most {MAX_HEADERS} headers."),
            ));
        }
        let mut seen: Vec<String> = Vec::new();
        for h in &headers {
            let plain_name = !h.name.is_empty()
                && h.name.len() <= 64
                && h.name.starts_with(|c: char| c.is_ascii_alphabetic())
                && h.name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-');
            if !plain_name {
                return Err(err(
                    "invalid_header_name",
                    "A header name is letters, digits and dashes, starting with a letter.",
                ));
            }
            let lower = h.name.to_ascii_lowercase();
            if RESERVED_HEADERS.contains(&lower.as_str()) {
                return Err(err(
                    "reserved_header",
                    format!(
                        "The header {} is set by the panel and cannot be changed.",
                        h.name
                    ),
                ));
            }
            if seen.contains(&lower) {
                return Err(err(
                    "duplicate_header",
                    format!("The header {} is given twice.", h.name),
                ));
            }
            seen.push(lower);
            let plain_value = !h.value.is_empty()
                && h.value.len() <= 512
                && h.value
                    .chars()
                    .all(|c| (' '..='~').contains(&c) && !matches!(c, '"' | '\\' | '$'));
            if !plain_value {
                return Err(err(
                    "invalid_header_value",
                    "A header value is printable ASCII without quotes, backslash or $.",
                ));
            }
        }
        Ok(Self {
            rate_limit_per_second: rate_limit,
            headers,
            page_cache: None,
        })
    }

    /// The same policy with the page cache on or off.
    #[must_use]
    pub fn with_page_cache(mut self, page_cache: Option<PageCache>) -> Self {
        self.page_cache = page_cache;
        self
    }

    /// Whether it changes nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rate_limit_per_second.is_none() && self.headers.is_empty() && self.page_cache.is_none()
    }

    /// The name of the rate limit zone: letters, digits and `_`, one per domain.
    fn zone(domain: &Fqdn) -> String {
        let mut hash: u32 = 0x811c_9dc5;
        for b in domain.as_str().bytes() {
            hash = (hash ^ u32::from(b)).wrapping_mul(0x0100_0193);
        }
        let plain: String = domain
            .as_str()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .take(40)
            .collect();
        format!("rl_{plain}_{hash:08x}")
    }

    /// Lines for before the `server` block (the zone of the rate limit).
    fn http_lines(&self, domain: &Fqdn) -> Vec<String> {
        // Behind the panel's own front web server the visitor's address is in
        // X-Forwarded-For, which that server sets itself.
        let mut lines: Vec<String> = self
            .rate_limit_per_second
            .map(|r| {
                vec![format!(
                    "limit_req_zone $http_x_forwarded_for zone={}:1m rate={r}r/s;",
                    Self::zone(domain)
                )]
            })
            .unwrap_or_default();
        if self.page_cache.is_some() {
            let z = Self::cache_zone(domain);
            // Small, in the web container's own memory disk: eight megabytes a site.
            lines.push(format!(
                "fastcgi_cache_path /var/cache/nginx/{z} levels=1:2 keys_zone={z}:1m max_size=8m inactive=60m use_temp_path=off;"
            ));
            lines.push(format!(
                "map $http_cookie $skip_cookie_{z} {{ default 0; ~*(wordpress_logged_in|wordpress_sec|wp-postpass|comment_author|woocommerce_items_in_cart|PHPSESSID|laravel_session|session) 1; }}"
            ));
            lines.push(format!(
                "map $http_authorization $skip_auth_{z} {{ default 1; \"\" 0; }}"
            ));
            lines.push(format!(
                "map $request_uri $skip_uri_{z} {{ default 0; ~*^/(wp-admin|wp-login\\.php|xmlrpc\\.php) 1; }}"
            ));
        }
        lines
    }

    /// The name of the cache zone and of its directory.
    fn cache_zone(domain: &Fqdn) -> String {
        Self::zone(domain).replacen("rl_", "pc_", 1)
    }

    /// Lines for inside the `server` block.
    fn server_lines(&self, domain: &Fqdn) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(r) = self.rate_limit_per_second {
            lines.push(format!(
                "limit_req zone={} burst={} nodelay;",
                Self::zone(domain),
                u32::from(r) * 2
            ));
            lines.push("limit_req_status 429;".to_owned());
        }
        for h in &self.headers {
            lines.push(format!("add_header {} \"{}\" always;", h.name, h.value));
        }
        if let Some(cache) = self.page_cache {
            let z = Self::cache_zone(domain);
            lines.push(format!("fastcgi_cache {z};"));
            lines.push(format!(
                "fastcgi_cache_key \"{}:$scheme$host$request_uri\";",
                cache.generation
            ));
            lines.push(format!(
                "fastcgi_cache_valid 200 301 {}m;",
                cache.ttl_minutes
            ));
            lines.push("fastcgi_cache_methods GET HEAD;".to_owned());
            lines.push("fastcgi_cache_lock on;".to_owned());
            lines.push(format!(
                "fastcgi_cache_bypass $skip_cookie_{z} $skip_auth_{z} $skip_uri_{z};"
            ));
            lines.push(format!(
                "fastcgi_no_cache $skip_cookie_{z} $skip_auth_{z} $skip_uri_{z};"
            ));
            lines.push("add_header X-Cache-Status $upstream_cache_status always;".to_owned());
        }
        lines
    }
}

/// Checks the configuration of a site as a node is about to use it: no forbidden
/// directive, and every `root` inside the tenant's web directory. The control
/// plane makes these files from checked values; this is for the node not to rely
/// on it.
///
/// # Errors
/// [`VhostError`] for the first problem.
pub fn check_site_config(text: &str) -> Result<(), VhostError> {
    for words in statements(text)? {
        if forbidden(&words[0], words.get(1).map(String::as_str)) {
            return Err(err(
                "forbidden_directive",
                format!("The directive `{}` is not allowed.", words[0].trim()),
            ));
        }
        if words[0] == "root" {
            let inside = words.get(1).is_some_and(|path| {
                (path == WEB_ROOT || path.starts_with(&format!("{WEB_ROOT}/")))
                    && !path.split('/').any(|part| part == "..")
            });
            if !inside {
                return Err(err(
                    "root_outside",
                    "A root must lie inside the tenant's web directory.",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn fqdn(s: &str) -> Fqdn {
        Fqdn::parse(s).unwrap()
    }

    #[test]
    fn a_domain_with_an_application_is_passed_on_to_it() {
        check_template(PROXY_NGINX_TEMPLATE).unwrap();
        let domain = fqdn("app.example.com");
        let docroot = Docroot::parse("").unwrap();
        let policy = Policy::default();
        let site = |proxy| Site {
            domain: &domain,
            aliases: &[],
            docroot: &docroot,
            php_upstream: "bp-alice-php:9000",
            policy: &policy,
            custom: "",
            proxy,
        };
        let text = render(DEFAULT_NGINX_TEMPLATE, &site(Some("bp-alice-app-api:3000"))).unwrap();
        assert!(
            text.contains("proxy_pass http://bp-alice-app-api:3000;"),
            "{text}"
        );
        assert!(!text.contains("fastcgi_pass"));
        assert!(text.contains("server_name app.example.com;"));
        // A bad application address is refused like a bad PHP address.
        assert!(
            render(
                DEFAULT_NGINX_TEMPLATE,
                &site(Some("evil; return 200;:3000"))
            )
            .is_err()
        );
    }

    #[test]
    fn a_page_cache_adds_its_zone_keys_and_bypass_rules() {
        let domain = fqdn("example.com");
        let docroot = Docroot::parse("public_html").unwrap();
        let render_with_cache = |generation: u32| {
            let policy =
                Policy::default().with_page_cache(Some(PageCache::parse(15, generation).unwrap()));
            render(
                DEFAULT_NGINX_TEMPLATE,
                &Site {
                    domain: &domain,
                    aliases: &[],
                    docroot: &docroot,
                    php_upstream: "bp-alice-php:9000",
                    policy: &policy,
                    custom: "",
                    proxy: None,
                },
            )
            .unwrap()
        };
        let text = render_with_cache(1);
        assert!(
            text.contains("fastcgi_cache_path /var/cache/nginx/pc_example_com_"),
            "{text}"
        );
        assert!(text.contains("fastcgi_cache_valid 200 301 15m;"));
        assert!(text.contains("fastcgi_cache_key \"1:$scheme$host$request_uri\";"));
        assert!(text.contains("fastcgi_cache_methods GET HEAD;"));
        assert!(text.contains("wordpress_logged_in"));
        assert!(text.contains("fastcgi_no_cache"));
        // The node accepts what the panel makes.
        check_site_config(&text).unwrap();
        // A purge changes the key and nothing else.
        let again = render_with_cache(2);
        assert_ne!(text, again);
        assert_eq!(
            text.replace("\"1:$scheme", "\"2:$scheme"),
            again,
            "only the generation differs"
        );
        // Without a cache there is nothing of it.
        assert!(!Policy::default().page_cache.is_some());
        assert!(PageCache::parse(0, 1).is_err());
        assert!(PageCache::parse(MAX_CACHE_MINUTES + 1, 1).is_err());
    }

    #[test]
    fn the_default_template_is_valid_and_makes_the_expected_file() {
        check_template(DEFAULT_NGINX_TEMPLATE).unwrap();
        let domain = fqdn("example.com");
        let aliases = [fqdn("www.example.com"), fqdn("example.net")];
        let docroot = Docroot::parse("public_html/shop").unwrap();
        let text = render(
            DEFAULT_NGINX_TEMPLATE,
            &Site {
                domain: &domain,
                aliases: &aliases,
                docroot: &docroot,
                php_upstream: "bp-alice-php:9000",
                policy: &Policy::default(),
                custom: "",
                proxy: None,
            },
        )
        .unwrap();
        // The whole file, so a change of the template shows up in review.
        assert_eq!(
            text,
            r"# Managed by Built Panel for example.com. Changes made here are overwritten.
server {
    listen 8080;
    server_name example.com www.example.com example.net;
    root /var/www/html/public_html/shop;
    index index.php index.html index.htm;

    location / {
        try_files $uri $uri/ /index.php?$query_string;
    }

    location ~ \.php$ {
        try_files $uri =404;
        include fastcgi_params;
        fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;
        fastcgi_pass bp-alice-php:9000;
    }

    location ~ /\.(?!well-known) {
        deny all;
    }
}
"
        );
    }

    #[test]
    fn the_web_directory_itself_and_repeated_names() {
        let domain = fqdn("example.com");
        let aliases = [
            fqdn("example.com"),
            fqdn("a.example.com"),
            fqdn("a.example.com"),
        ];
        let text = render(
            DEFAULT_NGINX_TEMPLATE,
            &Site {
                domain: &domain,
                aliases: &aliases,
                docroot: &Docroot::default(),
                php_upstream: "bp-alice-php:9000",
                policy: &Policy::default(),
                custom: "",
                proxy: None,
            },
        )
        .unwrap();
        assert!(
            text.contains("server_name example.com a.example.com;"),
            "{text}"
        );
        assert!(text.contains("root /var/www/html;"), "{text}");
    }

    #[test]
    fn a_template_that_cannot_make_a_site_is_refused() {
        let ok = "server { server_name {{server_names}}; root {{docroot}}; }";
        check_template(ok).unwrap();
        let cases = [
            ("", "template_empty"),
            ("server { root {{docroot}}; }", "missing_variable"),
            (
                "server { server_name {{server_names}}; }",
                "missing_variable",
            ),
            (
                "server { server_name {{server_names}}; root {{docroot}}; x {{nope}}; }",
                "unknown_variable",
            ),
            (
                "server { server_name {{server_names}}; root {{docroot}};",
                "template_braces",
            ),
            (
                "server { server_name {{server_names}}; root {{docroot",
                "template_variable",
            ),
            (
                "server { server_name {{server_names}}; root {{docroot}}; alias /etc; }",
                "forbidden_directive",
            ),
            (
                "server { server_name {{server_names}}; root {{docroot}}; include /etc/passwd; }",
                "forbidden_directive",
            ),
            (
                "server { server_name {{server_names}}; root {{docroot}}; content_by_lua_block { } }",
                "forbidden_directive",
            ),
            (
                "load_module modules/x.so; server { server_name {{server_names}}; root {{docroot}}; }",
                "forbidden_directive",
            ),
            (
                "server { server_name {{server_names}}; root {{docroot}}; \u{0} }",
                "template_characters",
            ),
        ];
        for (body, code) in cases {
            assert_eq!(check_template(body).unwrap_err().code, code, "{body}");
        }
        let big = format!("{ok}{}", "#".repeat(MAX_TEMPLATE_BYTES));
        assert_eq!(check_template(&big).unwrap_err().code, "template_too_large");
        // A commented-out directive is not a directive.
        check_template("server { server_name {{server_names}}; root {{docroot}}; # alias /x;\n}")
            .unwrap();
    }

    #[test]
    fn a_value_cannot_bring_a_directive_along() {
        let domain = fqdn("example.com");
        for bad in [
            "bp-a-php",
            "bp-a-php:0",
            "bp-a-php:9000; alias /etc",
            "DH:9000",
            ":9000",
            "a b:1",
        ] {
            let e = render(
                DEFAULT_NGINX_TEMPLATE,
                &Site {
                    domain: &domain,
                    aliases: &[],
                    docroot: &Docroot::default(),
                    php_upstream: bad,
                    policy: &Policy::default(),
                    custom: "",
                    proxy: None,
                },
            )
            .unwrap_err();
            assert_eq!(e.code, "invalid_upstream", "{bad}");
        }
        let many: Vec<Fqdn> = (0..MAX_SERVER_NAMES)
            .map(|i| fqdn(&format!("a{i}.example.com")))
            .collect();
        let e = render(
            DEFAULT_NGINX_TEMPLATE,
            &Site {
                domain: &domain,
                aliases: &many,
                docroot: &Docroot::default(),
                php_upstream: "bp-a-php:9000",
                policy: &Policy::default(),
                custom: "",
                proxy: None,
            },
        )
        .unwrap_err();
        assert_eq!(e.code, "too_many_names");
    }

    fn header(name: &str, value: &str) -> Header {
        Header {
            name: name.into(),
            value: value.into(),
        }
    }

    #[test]
    fn a_policy_adds_a_rate_limit_and_headers_to_the_file() {
        let policy = Policy::parse(
            Some(10),
            vec![
                header("X-Frame-Options", "SAMEORIGIN"),
                header("Content-Security-Policy", "default-src 'self'"),
            ],
        )
        .unwrap();
        let domain = fqdn("example.com");
        let text = render(
            DEFAULT_NGINX_TEMPLATE,
            &Site {
                domain: &domain,
                aliases: &[],
                docroot: &Docroot::default(),
                php_upstream: "bp-alice-php:9000",
                policy: &policy,
                custom: "",
                proxy: None,
            },
        )
        .unwrap();
        let zone = Policy::zone(&domain);
        assert!(zone.starts_with("rl_example_com_"), "{zone}");
        let expected_top = format!(
            "# Managed by Built Panel for example.com. Changes made here are overwritten.\nlimit_req_zone $http_x_forwarded_for zone={zone}:1m rate=10r/s;\nserver {{\n"
        );
        assert!(text.starts_with(&expected_top), "{text}");
        let inside = format!(
            "    index index.php index.html index.htm;\n    limit_req zone={zone} burst=20 nodelay;\n    limit_req_status 429;\n    add_header X-Frame-Options \"SAMEORIGIN\" always;\n    add_header Content-Security-Policy \"default-src 'self'\" always;\n\n    location / {{"
        );
        assert!(text.contains(&inside), "{text}");
        check_site_config(&text).unwrap();
        // An empty policy leaves the file as it was without one.
        let plain = render(
            DEFAULT_NGINX_TEMPLATE,
            &Site {
                domain: &domain,
                aliases: &[],
                docroot: &Docroot::default(),
                php_upstream: "bp-alice-php:9000",
                policy: &Policy::default(),
                custom: "",
                proxy: None,
            },
        )
        .unwrap();
        assert!(!plain.contains("limit_req") && !plain.contains("add_header"));
        assert!(Policy::default().is_empty() && !policy.is_empty());
    }

    #[test]
    fn a_policy_that_could_break_or_escape_the_file_is_refused() {
        let ok = |h: Vec<Header>| Policy::parse(None, h);
        assert!(ok(vec![header("X-Robots-Tag", "noindex, nofollow")]).is_ok());
        let cases: Vec<(Vec<Header>, &str)> = vec![
            (vec![header("", "x")], "invalid_header_name"),
            (vec![header("X Foo", "x")], "invalid_header_name"),
            (vec![header("1Foo", "x")], "invalid_header_name"),
            (vec![header("X-Foo", "")], "invalid_header_value"),
            (
                vec![header("X-Foo", "a\"; deny all; \"b")],
                "invalid_header_value",
            ),
            (vec![header("X-Foo", "a\nb")], "invalid_header_value"),
            (vec![header("X-Foo", "$host")], "invalid_header_value"),
            (vec![header("X-Foo", "a\\b")], "invalid_header_value"),
            (
                vec![header("X-Foo", &"v".repeat(513))],
                "invalid_header_value",
            ),
            (
                vec![header("Strict-Transport-Security", "x")],
                "reserved_header",
            ),
            (vec![header("set-cookie", "x")], "reserved_header"),
            (
                vec![header("X-Foo", "1"), header("x-foo", "2")],
                "duplicate_header",
            ),
            (
                (0..=MAX_HEADERS)
                    .map(|i| header(&format!("X-H{i}"), "1"))
                    .collect(),
                "too_many_headers",
            ),
        ];
        for (headers, code) in cases {
            assert_eq!(ok(headers).unwrap_err().code, code, "{code}");
        }
        for bad in [0, MAX_RATE_LIMIT + 1] {
            assert_eq!(
                Policy::parse(Some(bad), vec![]).unwrap_err().code,
                "invalid_rate_limit"
            );
        }
        assert!(Policy::parse(Some(MAX_RATE_LIMIT), vec![]).is_ok());
    }

    #[test]
    fn a_site_file_the_node_is_given_must_keep_to_the_tenants_directory() {
        check_site_config("server { root /var/www/html; }").unwrap();
        check_site_config("server { root /var/www/html/a/b; }").unwrap();
        for (text, code) in [
            ("server { root /etc; }", "root_outside"),
            ("server { root /var/www/html/../../etc; }", "root_outside"),
            ("server { root /var/www/htmlx; }", "root_outside"),
            ("server { root; }", "root_outside"),
            ("server { alias /etc/passwd; }", "forbidden_directive"),
            (
                "server { include /etc/nginx/x.conf; }",
                "forbidden_directive",
            ),
            ("load_module x.so;", "forbidden_directive"),
        ] {
            assert_eq!(check_site_config(text).unwrap_err().code, code, "{text}");
        }
        // The files the control plane makes pass.
        check_site_config(
            DEFAULT_NGINX_TEMPLATE
                .replace("{{docroot}}", "/var/www/html")
                .as_str(),
        )
        .unwrap();
    }

    #[test]
    fn a_persons_own_block_may_shape_the_site_and_nothing_else() {
        let good = "# old pages\nlocation /old { return 301 /new; }\nlocation ~* \\.(png|jpg)$ {\n    expires 30d;\n    add_header Cache-Control \"public\";\n}\nrewrite ^/a/(.*)$ /b/$1 permanent;\nerror_page 404 /404.html;\nclient_max_body_size 64m;\nif ($host = 'www.example.com') { return 301 https://example.com$request_uri; }\n";
        check_custom_block(good).unwrap();
        check_custom_block("").unwrap();
        // Text inside a string is text, not a directive.
        check_custom_block("set $a \"; alias /etc;\";").unwrap();
        let cases = [
            ("alias /etc/;", "forbidden_directive"),
            ("root /etc;", "forbidden_directive"),
            ("include /etc/passwd;", "forbidden_directive"),
            ("proxy_pass http://10.0.2.2:5432;", "forbidden_directive"),
            ("fastcgi_pass 127.0.0.1:9000;", "forbidden_directive"),
            ("access_log /tmp/x;", "forbidden_directive"),
            ("content_by_lua_block { }", "forbidden_directive"),
            ("perl_set $x 'sub { }';", "forbidden_directive"),
            ("load_module x.so;", "forbidden_directive"),
            ("server { }", "forbidden_directive"),
            ("location / { alias /etc; }", "forbidden_directive"),
            (
                "add_header Strict-Transport-Security \"x\";",
                "reserved_header",
            ),
            // Nothing hides a directive from the check: not a quote, a comment mark or a semicolon.
            ("add_header X \"a#\"; alias /etc;", "forbidden_directive"),
            ("add_header X 'a'; alias /etc;", "forbidden_directive"),
            ("return 200 x;alias /etc;", "forbidden_directive"),
            ("location /x {\n}\nalias /etc;", "forbidden_directive"),
            ("add_header X \"unclosed;", "template_syntax"),
            ("location / { return 200 x;", "template_braces"),
            ("return 200 x; }", "template_syntax"),
            ("return 200 \"{{domain}}\";", "custom_characters"),
            ("return 200 \"\u{e9}\";", "custom_characters"),
            ("return 200 \"\u{0}\";", "custom_characters"),
        ];
        for (text, code) in cases {
            assert_eq!(
                check_custom_block(text).err().map(|e| e.code),
                Some(code),
                "{text:?}"
            );
        }
        let big = "return 200 x;\n".repeat(MAX_CUSTOM_BYTES);
        assert_eq!(
            check_custom_block(&big).unwrap_err().code,
            "custom_too_large"
        );
        let many = "break;".repeat(MAX_CUSTOM_STATEMENTS + 1);
        assert_eq!(
            check_custom_block(&many).unwrap_err().code,
            "custom_too_large"
        );
    }

    #[test]
    fn a_comment_mark_inside_a_word_or_string_hides_nothing_from_the_node_either() {
        for text in [
            "server { add_header X \"a#\"; alias /etc; }",
            "server { set $a b#c; alias /etc; }",
            "server { set $a \"x;y\"; alias /etc; }",
        ] {
            assert_eq!(
                check_site_config(text).unwrap_err().code,
                "forbidden_directive",
                "{text}"
            );
        }
        // A real comment hides its own line only.
        check_site_config("server { # alias /etc;\n root /var/www/html; }").unwrap();
        assert!(check_site_config("server { # alias /etc;\n alias /etc; }").is_err());
    }

    #[test]
    fn the_block_lands_in_the_file_where_the_template_says() {
        let domain = fqdn("example.com");
        let custom = "location /old { return 301 /new; }\nclient_max_body_size 64m;";
        let text = render(
            DEFAULT_NGINX_TEMPLATE,
            &Site {
                domain: &domain,
                aliases: &[],
                docroot: &Docroot::default(),
                php_upstream: "bp-alice-php:9000",
                policy: &Policy::default(),
                custom,
                proxy: None,
            },
        )
        .unwrap();
        assert!(
            text.contains("    index index.php index.html index.htm;\n    location /old { return 301 /new; }\n    client_max_body_size 64m;\n\n    location / {"),
            "{text}"
        );
        check_site_config(&text).unwrap();
        // A block that is not allowed is not rendered, whoever asks.
        let e = render(
            DEFAULT_NGINX_TEMPLATE,
            &Site {
                domain: &domain,
                aliases: &[],
                docroot: &Docroot::default(),
                php_upstream: "bp-alice-php:9000",
                policy: &Policy::default(),
                custom: "alias /etc;",
                proxy: None,
            },
        )
        .unwrap_err();
        assert_eq!(e.code, "forbidden_directive");
    }
}
