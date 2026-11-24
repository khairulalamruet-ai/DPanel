//! The `php.ini` options an account may set (T06.10).
//!
//! Only a short list of options can be set, each with its kind and range, and
//! the plan puts a ceiling on the ones that cost the node something (memory,
//! upload size, run time). What is stored is the account's choice; what goes to
//! the node is the choice kept within the plan's ceilings as they are now.

use std::collections::BTreeMap;

use serde_json::Value;

/// What an option holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A size: digits and `K`, `M` or `G`.
    Size,
    /// A whole number in a range.
    Int { min: i64, max: i64 },
    /// On or off.
    Flag,
    /// One of a few fixed texts.
    Choice(&'static [&'static str]),
    /// A time zone name such as `Asia/Dhaka`.
    TimeZone,
}

/// Which ceiling of the plan applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cap {
    None,
    Memory,
    Upload,
    Time,
}

struct Option_ {
    key: &'static str,
    kind: Kind,
    cap: Cap,
    /// What PHP uses when nothing is set (shown to people).
    default: &'static str,
}

const ERROR_LEVELS: &[&str] = &[
    "E_ALL",
    "E_ALL & ~E_DEPRECATED",
    "E_ALL & ~E_NOTICE & ~E_STRICT & ~E_DEPRECATED",
    "E_ERROR | E_WARNING | E_PARSE",
];

const OPTIONS: &[Option_] = &[
    Option_ {
        key: "memory_limit",
        kind: Kind::Size,
        cap: Cap::Memory,
        default: "128M",
    },
    Option_ {
        key: "upload_max_filesize",
        kind: Kind::Size,
        cap: Cap::Upload,
        default: "2M",
    },
    Option_ {
        key: "post_max_size",
        kind: Kind::Size,
        cap: Cap::Upload,
        default: "8M",
    },
    Option_ {
        key: "max_execution_time",
        kind: Kind::Int { min: 1, max: 3600 },
        cap: Cap::Time,
        default: "30",
    },
    Option_ {
        key: "max_input_time",
        kind: Kind::Int { min: 1, max: 3600 },
        cap: Cap::Time,
        default: "60",
    },
    Option_ {
        key: "max_input_vars",
        kind: Kind::Int {
            min: 100,
            max: 100_000,
        },
        cap: Cap::None,
        default: "1000",
    },
    Option_ {
        key: "session.gc_maxlifetime",
        kind: Kind::Int {
            min: 60,
            max: 86_400,
        },
        cap: Cap::None,
        default: "1440",
    },
    Option_ {
        key: "display_errors",
        kind: Kind::Flag,
        cap: Cap::None,
        default: "Off",
    },
    Option_ {
        key: "short_open_tag",
        kind: Kind::Flag,
        cap: Cap::None,
        default: "Off",
    },
    Option_ {
        key: "error_reporting",
        kind: Kind::Choice(ERROR_LEVELS),
        cap: Cap::None,
        default: "E_ALL",
    },
    Option_ {
        key: "date.timezone",
        kind: Kind::TimeZone,
        cap: Cap::None,
        default: "UTC",
    },
];

/// The options, for showing: key, what it holds, PHP's default.
#[must_use]
pub fn options() -> Vec<(&'static str, &'static str, &'static str)> {
    OPTIONS
        .iter()
        .map(|o| {
            let kind = match o.kind {
                Kind::Size => "size",
                Kind::Int { .. } => "int",
                Kind::Flag => "flag",
                Kind::Choice(_) => "choice",
                Kind::TimeZone => "timezone",
            };
            (o.key, kind, o.default)
        })
        .collect()
}

/// The choices of a `choice` option.
#[must_use]
pub fn choices(key: &str) -> &'static [&'static str] {
    OPTIONS
        .iter()
        .find(|o| o.key == key)
        .map_or(&[], |o| match o.kind {
            Kind::Choice(list) => list,
            _ => &[],
        })
}

/// The ceilings of a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    /// Most `memory_limit`, in bytes.
    pub memory_bytes: u64,
    /// Most `upload_max_filesize` and `post_max_size`, in bytes.
    pub upload_bytes: u64,
    /// Most `max_execution_time` and `max_input_time`, in seconds.
    pub seconds: i64,
}

impl Caps {
    /// What a plan without its own numbers gets.
    pub const DEFAULT: Self = Self {
        memory_bytes: 256 << 20,
        upload_bytes: 64 << 20,
        seconds: 300,
    };

    /// The ceilings a plan's module limits give: `php_memory_limit_mb`,
    /// `php_upload_max_mb` and `php_max_execution_time`; -1 is as much as an
    /// option may be at all.
    #[must_use]
    pub fn from_limits(limits: &Value) -> Self {
        let get =
            |key: &str, default: i64| limits.get(key).and_then(Value::as_i64).unwrap_or(default);
        let mb = |key: &str, default_mb: i64, most_mb: i64| {
            let v = get(key, default_mb);
            let v = if v < 0 { most_mb } else { v.min(most_mb) };
            u64::try_from(v).unwrap_or(0) << 20
        };
        let seconds = get("php_max_execution_time", 300);
        Self {
            memory_bytes: mb("php_memory_limit_mb", 256, 4096),
            upload_bytes: mb("php_upload_max_mb", 64, 1024),
            seconds: if seconds < 0 { 3600 } else { seconds.min(3600) },
        }
    }
}

/// What is wrong with an option.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct IniError {
    /// The option (or empty for the whole).
    pub key: String,
    /// Stable, for programs.
    pub code: &'static str,
    /// For people.
    pub message: String,
}

fn err(key: &str, code: &'static str, message: impl Into<String>) -> IniError {
    IniError {
        key: key.to_owned(),
        code,
        message: message.into(),
    }
}

/// Bytes in a size like `64M` (`K`, `M`, `G`; no unit means bytes).
fn size_bytes(text: &str) -> Option<u64> {
    let (digits, unit) = match text.char_indices().last()? {
        (i, c) if matches!(c, 'K' | 'M' | 'G') => (&text[..i], c),
        _ => (text, ' '),
    };
    if digits.is_empty() || digits.len() > 7 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u64 = digits.parse().ok()?;
    n.checked_mul(match unit {
        'K' => 1 << 10,
        'M' => 1 << 20,
        'G' => 1 << 30,
        _ => 1,
    })
}

/// What an account chose, checked: option to normalised text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IniSettings(BTreeMap<String, String>);

impl IniSettings {
    /// Checks what a person gave (a JSON object of option to value) against the
    /// ceilings of their plan.
    ///
    /// # Errors
    /// [`IniError`] for the first option that is unknown, of the wrong kind, out
    /// of range or above the plan's ceiling.
    pub fn parse(given: &serde_json::Map<String, Value>, caps: &Caps) -> Result<Self, IniError> {
        let mut out = BTreeMap::new();
        for (key, value) in given {
            let Some(option) = OPTIONS.iter().find(|o| o.key == key) else {
                return Err(err(key, "unknown_option", "This option cannot be set."));
            };
            let text = Self::one(option, value, caps)?;
            out.insert(key.clone(), text);
        }
        // A post body must be able to hold an upload.
        let post = out
            .get("post_max_size")
            .map(String::as_str)
            .and_then(size_bytes);
        let upload = out
            .get("upload_max_filesize")
            .map(String::as_str)
            .and_then(size_bytes);
        if let (Some(post), Some(upload)) = (post, upload)
            && post < upload
        {
            return Err(err(
                "post_max_size",
                "post_smaller_than_upload",
                "post_max_size must be at least upload_max_filesize.",
            ));
        }
        Ok(Self(out))
    }

    fn one(option: &Option_, value: &Value, caps: &Caps) -> Result<String, IniError> {
        let key = option.key;
        let wrong = |what: &str| err(key, "invalid_value", format!("{key} must be {what}."));
        match option.kind {
            Kind::Size => {
                let text = value.as_str().ok_or_else(|| wrong("a size like 64M"))?;
                let upper = text.to_ascii_uppercase();
                let bytes = size_bytes(&upper).ok_or_else(|| wrong("a size like 64M"))?;
                let cap = match option.cap {
                    Cap::Memory => caps.memory_bytes,
                    Cap::Upload => caps.upload_bytes,
                    _ => u64::MAX,
                };
                if bytes < (1 << 20) {
                    return Err(wrong("at least 1M"));
                }
                if bytes > cap {
                    return Err(err(
                        key,
                        "above_plan_limit",
                        format!("{key} may be at most {}M on this plan.", cap >> 20),
                    ));
                }
                Ok(upper)
            }
            Kind::Int { min, max } => {
                let n = value.as_i64().ok_or_else(|| wrong("a whole number"))?;
                let max = match option.cap {
                    Cap::Time => max.min(caps.seconds),
                    _ => max,
                };
                if n < min {
                    return Err(wrong(&format!("at least {min}")));
                }
                if n > max {
                    let code = if option.cap == Cap::Time && max < 3600 {
                        "above_plan_limit"
                    } else {
                        "out_of_range"
                    };
                    return Err(err(key, code, format!("{key} may be at most {max}.")));
                }
                Ok(n.to_string())
            }
            Kind::Flag => match value.as_bool() {
                Some(true) => Ok("On".to_owned()),
                Some(false) => Ok("Off".to_owned()),
                None => Err(wrong("true or false")),
            },
            Kind::Choice(list) => {
                let text = value
                    .as_str()
                    .ok_or_else(|| wrong("one of the offered levels"))?;
                list.contains(&text)
                    .then(|| text.to_owned())
                    .ok_or_else(|| wrong("one of the offered levels"))
            }
            Kind::TimeZone => {
                let text = value
                    .as_str()
                    .ok_or_else(|| wrong("a time zone like Asia/Dhaka"))?;
                let plain = text.len() <= 40
                    && text.starts_with(|c: char| c.is_ascii_alphabetic())
                    && text.split('/').all(|p| {
                        !p.is_empty()
                            && p.bytes().all(|b| {
                                b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'-')
                            })
                    })
                    && text.split('/').count() <= 3;
                plain
                    .then(|| text.to_owned())
                    .ok_or_else(|| wrong("a time zone like Asia/Dhaka"))
            }
        }
    }

    /// Reads what was stored (a JSON object of option to normalised text), with no
    /// plan ceiling: for [`IniSettings::render`], which applies the ceilings.
    #[must_use]
    pub fn from_stored(stored: &serde_json::Map<String, Value>) -> Self {
        Self(
            stored
                .iter()
                .filter(|(k, _)| OPTIONS.iter().any(|o| o.key == k.as_str()))
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                .collect(),
        )
    }

    /// As JSON, for storing and showing: option to normalised text.
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::Object(
            self.0
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                .collect(),
        )
    }

    /// Whether nothing is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The options as they are set, normalised.
    #[must_use]
    pub fn values(&self) -> &BTreeMap<String, String> {
        &self.0
    }

    /// The text of the ini file: each option within the plan's ceilings as they
    /// are now (a plan that was lowered after the choice lowers what is used).
    #[must_use]
    pub fn render(&self, caps: &Caps) -> String {
        let mut out =
            String::from("; Managed by Built Panel. Changes made here are overwritten.\n");
        for option in OPTIONS {
            let Some(text) = self.0.get(option.key) else {
                continue;
            };
            let value = match (option.kind, option.cap) {
                (Kind::Size, Cap::Memory | Cap::Upload) => {
                    let cap = if option.cap == Cap::Memory {
                        caps.memory_bytes
                    } else {
                        caps.upload_bytes
                    };
                    match size_bytes(text) {
                        Some(b) if b > cap => format!("{}M", cap >> 20),
                        _ => text.clone(),
                    }
                }
                (Kind::Int { .. }, Cap::Time) => match text.parse::<i64>() {
                    Ok(n) if n > caps.seconds => caps.seconds.to_string(),
                    _ => text.clone(),
                },
                _ => text.clone(),
            };
            out.push_str(&format!("{} = {}\n", option.key, ini_value(&value)));
        }
        out
    }
}

/// A value as the ini file writes it: plain words as they are, anything with
/// spaces or operators in quotes.
fn ini_value(value: &str) -> String {
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_/+-.".contains(c))
    {
        value.to_owned()
    } else {
        format!("\"{value}\"")
    }
}

/// Checks the text of an ini file as a node is about to use it: only the options
/// of the list, one `key = value` per line (comments and blank lines allowed),
/// and values of the shapes above. The control plane makes these files from
/// checked values; this is for the node not to rely on it.
///
/// # Errors
/// [`IniError`] for the first bad line.
pub fn check_text(text: &str) -> Result<(), IniError> {
    if text.len() > 4096 {
        return Err(err("", "too_large", "The file is too large."));
    }
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        let Some((key, value)) = line.split_once(" = ") else {
            return Err(err("", "bad_line", "A line is `option = value`."));
        };
        let Some(option) = OPTIONS.iter().find(|o| o.key == key) else {
            return Err(err(key, "unknown_option", "This option cannot be set."));
        };
        let unquoted = value.trim_matches('"');
        // Shapes only: ranges and the plan's ceilings were applied by the control plane.
        let ok = match option.kind {
            Kind::Size => size_bytes(unquoted).is_some(),
            Kind::Int { min, max } => unquoted
                .parse::<i64>()
                .is_ok_and(|n| (min..=max).contains(&n)),
            Kind::Flag => matches!(unquoted, "On" | "Off"),
            Kind::Choice(list) => list.contains(&unquoted),
            Kind::TimeZone => {
                !unquoted.is_empty()
                    && unquoted.len() <= 40
                    && unquoted.bytes().all(|b| {
                        b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'-' | b'/')
                    })
            }
        };
        if !ok {
            return Err(err(
                key,
                "invalid_value",
                "The value does not fit the option.",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;

    fn parse(v: Value) -> Result<IniSettings, IniError> {
        IniSettings::parse(v.as_object().unwrap(), &Caps::DEFAULT)
    }

    #[test]
    fn good_options_are_kept_in_a_normal_form_and_written_in_a_fixed_order() {
        let s = parse(json!({
            "upload_max_filesize": "32m",
            "post_max_size": "40M",
            "memory_limit": "256M",
            "max_execution_time": 120,
            "display_errors": true,
            "error_reporting": "E_ALL & ~E_DEPRECATED",
            "date.timezone": "Asia/Dhaka",
            "short_open_tag": false
        }))
        .unwrap();
        assert_eq!(s.values()["upload_max_filesize"], "32M");
        assert_eq!(
            s.render(&Caps::DEFAULT),
            "; Managed by Built Panel. Changes made here are overwritten.\n\
             memory_limit = 256M\n\
             upload_max_filesize = 32M\n\
             post_max_size = 40M\n\
             max_execution_time = 120\n\
             display_errors = On\n\
             short_open_tag = Off\n\
             error_reporting = \"E_ALL & ~E_DEPRECATED\"\n\
             date.timezone = Asia/Dhaka\n"
        );
        check_text(&s.render(&Caps::DEFAULT)).unwrap();
        assert!(IniSettings::default().is_empty());
        assert!(parse(json!({})).unwrap().is_empty());
    }

    #[test]
    fn what_is_not_on_the_list_or_not_of_the_kind_is_refused() {
        let cases = [
            (
                json!({"disable_functions": ""}),
                "disable_functions",
                "unknown_option",
            ),
            (
                json!({"open_basedir": "/"}),
                "open_basedir",
                "unknown_option",
            ),
            (
                json!({"auto_prepend_file": "/etc/passwd"}),
                "auto_prepend_file",
                "unknown_option",
            ),
            (
                json!({"memory_limit": 256}),
                "memory_limit",
                "invalid_value",
            ),
            (
                json!({"memory_limit": "lots"}),
                "memory_limit",
                "invalid_value",
            ),
            (
                json!({"memory_limit": "-1"}),
                "memory_limit",
                "invalid_value",
            ),
            (
                json!({"memory_limit": "64K"}),
                "memory_limit",
                "invalid_value",
            ),
            (
                json!({"memory_limit": "256M\nauto_prepend_file=/x"}),
                "memory_limit",
                "invalid_value",
            ),
            (
                json!({"max_execution_time": "30"}),
                "max_execution_time",
                "invalid_value",
            ),
            (
                json!({"max_execution_time": 0}),
                "max_execution_time",
                "invalid_value",
            ),
            (
                json!({"max_input_vars": 5}),
                "max_input_vars",
                "invalid_value",
            ),
            (
                json!({"max_input_vars": 1_000_000}),
                "max_input_vars",
                "out_of_range",
            ),
            (
                json!({"display_errors": "yes"}),
                "display_errors",
                "invalid_value",
            ),
            (
                json!({"error_reporting": "E_ALL | E_STRICT"}),
                "error_reporting",
                "invalid_value",
            ),
            (
                json!({"date.timezone": "Asia/Dhaka\nx=1"}),
                "date.timezone",
                "invalid_value",
            ),
            (
                json!({"date.timezone": "../etc"}),
                "date.timezone",
                "invalid_value",
            ),
        ];
        for (given, key, code) in cases {
            let e = parse(given.clone()).unwrap_err();
            assert_eq!((e.key.as_str(), e.code), (key, code), "{given}");
        }
    }

    #[test]
    fn the_plan_puts_a_ceiling_on_what_costs_the_node() {
        let caps = Caps {
            memory_bytes: 128 << 20,
            upload_bytes: 16 << 20,
            seconds: 60,
        };
        let p = |v: Value| IniSettings::parse(v.as_object().unwrap(), &caps);
        assert!(p(json!({"memory_limit": "128M"})).is_ok());
        for (given, key) in [
            (json!({"memory_limit": "129M"}), "memory_limit"),
            (json!({"upload_max_filesize": "17M"}), "upload_max_filesize"),
            (json!({"post_max_size": "1G"}), "post_max_size"),
            (json!({"max_execution_time": 61}), "max_execution_time"),
            (json!({"max_input_time": 600}), "max_input_time"),
        ] {
            let e = p(given.clone()).unwrap_err();
            assert_eq!(
                (e.key.as_str(), e.code),
                (key, "above_plan_limit"),
                "{given}"
            );
        }
        // A post body must be able to hold an upload.
        let e = parse(json!({"upload_max_filesize": "32M", "post_max_size": "16M"})).unwrap_err();
        assert_eq!(e.code, "post_smaller_than_upload");
        // A choice made under a bigger plan is lowered when the plan shrinks.
        let chosen = parse(json!({"memory_limit": "256M", "max_execution_time": 250})).unwrap();
        let text = chosen.render(&caps);
        assert!(text.contains("memory_limit = 128M"), "{text}");
        assert!(text.contains("max_execution_time = 60"), "{text}");
    }

    #[test]
    fn a_plans_limits_give_the_ceilings() {
        assert_eq!(Caps::from_limits(&json!({})), Caps::DEFAULT);
        let caps = Caps::from_limits(&json!({
            "php_memory_limit_mb": 512, "php_upload_max_mb": -1, "php_max_execution_time": 90
        }));
        assert_eq!(
            caps,
            Caps {
                memory_bytes: 512 << 20,
                upload_bytes: 1024 << 20,
                seconds: 90
            }
        );
        let wild = Caps::from_limits(
            &json!({"php_memory_limit_mb": 999_999, "php_max_execution_time": 99_999}),
        );
        assert_eq!((wild.memory_bytes, wild.seconds), (4096 << 20, 3600));
    }

    #[test]
    fn the_node_accepts_only_what_the_control_plane_writes() {
        check_text("; hello\n\nmemory_limit = 64M\ndisplay_errors = Off\n").unwrap();
        for bad in [
            "auto_prepend_file = /etc/passwd\n",
            "memory_limit=64M\n",
            "memory_limit = lots\n",
            "display_errors = maybe\n",
            "max_execution_time = 99999\n",
            "error_reporting = \"E_ALL | E_STRICT\"\n",
            "extension = evil.so\n",
            "[PHP]\nmemory_limit = 64M\n",
        ] {
            assert!(check_text(bad).is_err(), "{bad:?}");
        }
        assert!(check_text(&"; x\n".repeat(2000)).is_err());
    }
}
