//! The software marketplace: server software a client can install with one click and remove or
//! update later, in the groups a hosting panel's customers know (databases, web servers, message
//! queues, Java tools, version control, ...).
//!
//! One TOML file per software in `deploy/marketplace/`, compiled into the program like the
//! application catalog. Each is one of:
//!
//! * `container`: run from an open-source image, pinned by digest, in the same hardened way
//!   every client container is run (no capabilities, no privileges, limited memory, its own
//!   network, a data folder). `deploy/marketplace/refresh.sh` finds newer releases and re-pins.
//! * `builtin`: the panel already does it (PHP versions, Node.js, MariaDB, ...); the entry says
//!   where, so the list is complete.
//! * `unavailable`: it needs root on the server or is not open source; the entry says why.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

include!(concat!(env!("OUT_DIR"), "/marketplace_files.rs"));

/// The groups, in the order a customer sees them.
pub const CATEGORIES: &[(&str, &str)] = &[
    ("databases", "Databases"),
    ("web_servers", "Web servers"),
    ("server_side_scripting", "Server side scripting"),
    ("java_tools", "Java tools"),
    ("java_containers", "Java containers"),
    ("message_queue", "Message queues"),
    ("version_control", "Version control"),
    ("security", "Security"),
    ("utilities", "Utilities"),
    ("modules", "Modules"),
    ("stacks", "Stacks"),
    ("statistics", "Statistics"),
    ("libraries", "Libraries"),
];

/// How a piece of software is provided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Run in a container from a pinned image.
    Container,
    /// Part of the panel already.
    Builtin,
    /// Cannot be offered, with the reason.
    Unavailable,
}

/// One piece of software.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Software {
    /// File name without `.toml`: 2 to 32 of a-z, 0-9 and `-`.
    pub slug: String,
    /// Shown name.
    pub name: String,
    /// One of [`CATEGORIES`].
    pub category: String,
    /// What it is, one or two sentences.
    pub description: String,
    /// SPDX identifier of the license; the marketplace lists open-source software only.
    pub license: String,
    /// Project page.
    #[serde(default)]
    pub homepage: String,
    /// How it is provided.
    pub kind: Kind,
    /// The newest version this entry installs (shown to the customer).
    #[serde(default)]
    pub version: String,
    /// The names of the Webuzo software this entry answers to.
    #[serde(default)]
    pub covers: Vec<String>,
    /// `builtin`: where in the panel it is.
    #[serde(default)]
    pub provided_by: String,
    /// `unavailable`: why.
    #[serde(default)]
    pub reason: String,
    /// `container`: the image without a tag (`docker.io/library/redis`).
    #[serde(default)]
    pub image: String,
    /// `container`: the tag the digest was taken from.
    #[serde(default)]
    pub tag: String,
    /// `container`: the version prefix `refresh.py` follows (`8` keeps Redis in 8.x); empty to
    /// follow every release.
    #[serde(default)]
    pub track: String,
    /// `container`: `sha256:` and 64 hex digits.
    #[serde(default)]
    pub digest: String,
    /// `container`: the port it listens on (1024 and up).
    #[serde(default)]
    pub port: i32,
    /// `container`: the memory it starts with, MiB.
    #[serde(default)]
    pub memory_mb: i32,
    /// `container`: its environment (names in capitals).
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// `container`: how to use it once it runs (host name is the container's name).
    #[serde(default)]
    pub usage: String,
}

impl Software {
    /// `repo@sha256:...`, what the node runs.
    #[must_use]
    pub fn reference(&self) -> String {
        format!("{}@{}", self.image, self.digest)
    }

    /// The text kept with the image of this entry, so an installed container can be traced back
    /// to the entry and the version.
    #[must_use]
    pub fn image_note(&self) -> String {
        format!("marketplace:{}:{}", self.slug, self.version)
    }

    fn checked(&self) -> Result<(), String> {
        let slug_ok = (2..=32).contains(&self.slug.len())
            && self.slug.starts_with(|c: char| c.is_ascii_lowercase())
            && !self.slug.ends_with('-')
            && self
                .slug
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !slug_ok {
            return Err("the slug is 2 to 32 of a-z, 0-9 and -".into());
        }
        if self.name.trim().is_empty() || self.description.trim().is_empty() {
            return Err("name and description are needed".into());
        }
        if !CATEGORIES.iter().any(|(c, _)| *c == self.category) {
            return Err(format!("unknown category {}", self.category));
        }
        if self.license.trim().is_empty() {
            return Err("the license is needed (open source only)".into());
        }
        let lower = self.license.to_ascii_lowercase();
        if lower.contains("proprietary") || lower.contains("commercial") {
            return Err("only open-source software is listed".into());
        }
        if !self.homepage.is_empty() && !self.homepage.starts_with("https://") {
            return Err("the homepage must be an https address".into());
        }
        match self.kind {
            Kind::Unavailable if self.reason.trim().is_empty() => {
                Err("an unavailable entry needs a reason".into())
            }
            Kind::Builtin if self.provided_by.trim().is_empty() => {
                Err("a builtin entry says where in the panel it is".into())
            }
            Kind::Container => self.checked_container(),
            _ => Ok(()),
        }
    }

    fn checked_container(&self) -> Result<(), String> {
        let image_ok = !self.image.is_empty()
            && self.image.len() <= 200
            && !self.image.contains("..")
            && !self.image.contains("//")
            && !self.image.starts_with(['/', '.', '-'])
            && !self.image.contains('@')
            && !self.image.contains(':')
            && self
                .image
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"./_-".contains(&b));
        if !image_ok {
            return Err("the image is a repository name without tag or digest".into());
        }
        let digest_ok = self.digest.strip_prefix("sha256:").is_some_and(|h| {
            h.len() == 64 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        });
        if !digest_ok {
            return Err("a container needs a pinned digest (sha256: and 64 hex digits)".into());
        }
        if self.digest.ends_with(&"0".repeat(64)) {
            return Err(
                "the digest is a placeholder: run deploy/marketplace/refresh.py --write".into(),
            );
        }
        if self.version.is_empty() || self.tag.is_empty() {
            return Err("a container needs the version and the tag it was pinned from".into());
        }
        if !(1024..=65_535).contains(&self.port) {
            return Err("the port is 1024 to 65535".into());
        }
        if !(32..=4096).contains(&self.memory_mb) {
            return Err("the memory is 32 to 4096 MiB".into());
        }
        for (k, v) in &self.env {
            let name_ok = !k.is_empty()
                && k.starts_with(|c: char| c.is_ascii_uppercase() || c == '_')
                && k.bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
            if !name_ok || v.contains('\0') {
                return Err(format!("the environment variable {k} is not valid"));
            }
        }
        Ok(())
    }
}

/// Why the marketplace could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketplaceError {
    /// The file.
    pub file: String,
    /// What is wrong with it.
    pub problem: String,
}

impl std::fmt::Display for MarketplaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.file, self.problem)
    }
}

impl std::error::Error for MarketplaceError {}

/// Reads and checks one file.
///
/// # Errors
/// [`MarketplaceError`].
pub fn parse(file: &str, text: &str) -> Result<Software, MarketplaceError> {
    let wrong = |problem: String| MarketplaceError {
        file: file.to_owned(),
        problem,
    };
    if text.len() > 64 * 1024 {
        return Err(wrong("larger than 64 KiB".into()));
    }
    let software: Software =
        toml::from_str(text).map_err(|e| wrong(e.to_string().replace('\n', " ")))?;
    software.checked().map_err(wrong)?;
    if file
        .strip_suffix(".toml")
        .is_some_and(|stem| stem != software.slug)
    {
        return Err(wrong(format!(
            "the file is called {file} but the slug is {}",
            software.slug
        )));
    }
    Ok(software)
}

/// All the software of this build, in category order and then by name.
///
/// # Errors
/// [`MarketplaceError`] (a build mistake; tested).
pub fn embedded() -> Result<Vec<Software>, MarketplaceError> {
    let mut all = Vec::new();
    let mut seen = BTreeSet::new();
    for (file, text) in FILES {
        let software = parse(file, text)?;
        if !seen.insert(software.slug.clone()) {
            return Err(MarketplaceError {
                file: (*file).to_owned(),
                problem: "the slug is used twice".into(),
            });
        }
        all.push(software);
    }
    let rank = |c: &str| {
        CATEGORIES
            .iter()
            .position(|(k, _)| *k == c)
            .unwrap_or(usize::MAX)
    };
    all.sort_by(|a, b| {
        rank(&a.category)
            .cmp(&rank(&b.category))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(all)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const DIGEST: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";

    fn container(extra: &str) -> String {
        format!(
            "slug='x1'\nname='X'\ncategory='databases'\ndescription='d'\nlicense='MIT'\nkind='container'\nversion='1'\nimage='docker.io/library/x'\ntag='1'\ndigest='{DIGEST}'\nport=6379\nmemory_mb=128\n{extra}"
        )
    }

    #[test]
    fn the_shipped_marketplace_is_valid() {
        let all = embedded().unwrap();
        assert!(all.len() > 20, "{} entries", all.len());
        for wanted in ["redis", "memcached", "elasticsearch"] {
            assert!(all.iter().any(|s| s.slug == wanted), "{wanted} is missing");
        }
        // Everything installable is pinned and says how to use it.
        for s in all.iter().filter(|s| s.kind == Kind::Container) {
            assert!(!s.usage.is_empty(), "{} has no usage text", s.slug);
        }
    }

    #[test]
    fn a_container_entry_is_checked() {
        let ok = parse("x1.toml", &container("")).unwrap();
        assert_eq!(ok.reference(), format!("docker.io/library/x@{DIGEST}"));
        assert_eq!(ok.image_note(), "marketplace:x1:1");
        for bad in [
            container("surprise=1\n"),
            container("").replace(DIGEST, "sha256:abc"),
            container("").replace("docker.io/library/x", "docker.io/library/x:latest"),
            container("").replace("docker.io/library/x", "../x"),
            container("").replace("port=6379", "port=80"),
            container("").replace("memory_mb=128", "memory_mb=8"),
            container("").replace("'MIT'", "'Proprietary'"),
            container("[env]\nlower='x'\n"),
        ] {
            assert!(parse("x1.toml", &bad).is_err(), "{bad}");
        }
        assert!(
            parse("other.toml", &container("")).is_err(),
            "file name is the slug"
        );
    }

    #[test]
    fn the_other_kinds_say_why_or_where() {
        let head =
            "slug='y1'\nname='Y'\ncategory='security'\ndescription='d'\nlicense='GPL-2.0-only'\n";
        assert!(parse("y1.toml", &format!("{head}kind='unavailable'\n")).is_err());
        assert!(
            parse(
                "y1.toml",
                &format!("{head}kind='unavailable'\nreason='needs root'\n")
            )
            .is_ok()
        );
        assert!(parse("y1.toml", &format!("{head}kind='builtin'\n")).is_err());
        assert!(
            parse(
                "y1.toml",
                &format!("{head}kind='builtin'\nprovided_by='Websites > PHP'\n")
            )
            .is_ok()
        );
    }
}
