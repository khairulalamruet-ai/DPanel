//! Typed settings (T03.08).
//!
//! Every module that has settings describes them with a JSON Schema (see
//! `Module::settings_schema`). This crate
//!
//! * checks the schemas at startup ([`SettingsRegistry::build`]): an object,
//!   every property one of `string` / `integer` / `number` / `boolean`, every
//!   property with a `default` that satisfies its own schema, and a known
//!   `x-scope`;
//! * validates what is written, **before** anything is stored
//!   ([`SettingsRegistry::validate`]): unknown settings, settings of another
//!   scope, and values that break the schema are refused, all problems at once;
//! * stores and reads them ([`set`], [`get`]): a setting without a stored
//!   value has its default, and a value equal to the default is not stored
//!   (resetting a setting is just writing its default).
//!
//! Display texts: `title` and `description` are plain English strings, as JSON
//! Schema requires; translations go in `x-title` / `x-description` as
//! `{"en": ..., "bn": ...}`, which the interface prefers.
//!
//! Scopes: `global` (the whole panel), `node` and `tenant` (one of each).
//! A setting belongs to one scope, declared by `x-scope`.

use std::collections::BTreeMap;

use builtpanel_database::{settings as db, sqlx::PgConnection};
use jsonschema::Validator;
use serde_json::{Map, Value};
use uuid::Uuid;

/// What a setting is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    /// The whole panel.
    Global,
    /// One node.
    Node(Uuid),
    /// One tenant.
    Tenant(Uuid),
}

impl Scope {
    fn kind(self) -> ScopeKind {
        match self {
            Self::Global => ScopeKind::Global,
            Self::Node(_) => ScopeKind::Node,
            Self::Tenant(_) => ScopeKind::Tenant,
        }
    }

    fn id(self) -> Option<Uuid> {
        match self {
            Self::Global => None,
            Self::Node(id) | Self::Tenant(id) => Some(id),
        }
    }
}

/// A scope without the id: what a schema declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    /// `global`.
    Global,
    /// `node`.
    Node,
    /// `tenant`.
    Tenant,
}

impl ScopeKind {
    /// The name used in schemas and in the database.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Node => "node",
            Self::Tenant => "tenant",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        match name {
            "global" => Some(Self::Global),
            "node" => Some(Self::Node),
            "tenant" => Some(Self::Tenant),
            _ => None,
        }
    }
}

/// One setting of a module.
pub struct Property {
    /// The property's schema, as the interface should show it.
    pub schema: Value,
    /// Its default.
    pub default: Value,
    /// Where it applies.
    pub scope: ScopeKind,
    validator: Validator,
}

/// The settings of one module.
pub struct ModuleSettings {
    /// Module name.
    pub module: String,
    /// The whole schema, for the interface (title, description, properties).
    pub schema: Value,
    /// By setting key.
    pub properties: BTreeMap<String, Property>,
}

/// A schema that cannot be used. The text names the module and the property.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("settings schema of module `{module}`: {problem}")]
pub struct SchemaError {
    /// The module.
    pub module: String,
    /// What is wrong.
    pub problem: String,
}

/// One thing wrong with a value that was sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The setting's key.
    pub key: String,
    /// Machine-readable reason: `unknown_setting`, `wrong_scope` or `invalid_value`.
    pub code: &'static str,
    /// Human-readable detail (short; never a stored secret).
    pub message: String,
}

/// Why a read or write failed.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// No module with settings has this name.
    #[error("no settings for module `{0}`")]
    UnknownModule(String),
    /// The values do not fit the schema. All problems are listed.
    #[error("{} invalid setting(s)", .0.len())]
    Invalid(Vec<Problem>),
    /// The database failed.
    #[error(transparent)]
    Database(#[from] builtpanel_database::sqlx::Error),
}

/// What a write changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Effective values of the module in this scope before.
    pub before: BTreeMap<String, Value>,
    /// ... and after.
    pub after: BTreeMap<String, Value>,
    /// Keys whose effective value differs.
    pub changed: Vec<String>,
}

/// The checked settings schemas of all modules.
#[derive(Default)]
pub struct SettingsRegistry {
    modules: BTreeMap<String, ModuleSettings>,
}

fn clip(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(200).collect()
}

impl SettingsRegistry {
    /// Checks and compiles the schemas. `schemas` are `(module, schema)`.
    ///
    /// # Errors
    /// Every schema that cannot be used, each naming the module and property.
    pub fn build(schemas: Vec<(String, Value)>) -> Result<Self, Vec<SchemaError>> {
        let mut modules = BTreeMap::new();
        let mut errors = Vec::new();
        for (module, schema) in schemas {
            match compile(&module, schema) {
                Ok(settings) => {
                    modules.insert(module, settings);
                }
                Err(problems) => errors.extend(problems.into_iter().map(|problem| SchemaError {
                    module: module.clone(),
                    problem,
                })),
            }
        }
        if errors.is_empty() {
            Ok(Self { modules })
        } else {
            Err(errors)
        }
    }

    /// The modules that have settings, by name.
    pub fn modules(&self) -> impl Iterator<Item = &ModuleSettings> {
        self.modules.values()
    }

    /// One module's settings.
    #[must_use]
    pub fn module(&self, name: &str) -> Option<&ModuleSettings> {
        self.modules.get(name)
    }

    /// Checks `values` for writing in `scope`. Returns them if every one is
    /// acceptable.
    ///
    /// # Errors
    /// [`SettingsError::UnknownModule`] or [`SettingsError::Invalid`] with
    /// every problem.
    pub fn validate(
        &self,
        module: &str,
        scope: Scope,
        values: &Map<String, Value>,
    ) -> Result<(), SettingsError> {
        let settings = self
            .module(module)
            .ok_or_else(|| SettingsError::UnknownModule(module.to_owned()))?;
        let mut problems = Vec::new();
        for (key, value) in values {
            let Some(property) = settings.properties.get(key) else {
                problems.push(Problem {
                    key: clip(key),
                    code: "unknown_setting",
                    message: "this module has no such setting".into(),
                });
                continue;
            };
            if property.scope != scope.kind() {
                problems.push(Problem {
                    key: key.clone(),
                    code: "wrong_scope",
                    message: format!(
                        "this setting is for the {} scope, not the {} scope",
                        property.scope.as_str(),
                        scope.kind().as_str()
                    ),
                });
                continue;
            }
            let mut found: Vec<String> = property
                .validator
                .iter_errors(value)
                .map(|e| clip(&e.to_string()))
                .collect();
            found.sort();
            found.dedup();
            problems.extend(found.into_iter().map(|message| Problem {
                key: key.clone(),
                code: "invalid_value",
                message,
            }));
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(SettingsError::Invalid(problems))
        }
    }
}

/// Reads the effective values of a module in a scope: the default of every
/// setting for that scope, overridden by what is stored.
///
/// # Errors
/// [`SettingsError::UnknownModule`] or database errors.
pub async fn get(
    registry: &SettingsRegistry,
    conn: &mut PgConnection,
    module: &str,
    scope: Scope,
) -> Result<BTreeMap<String, Value>, SettingsError> {
    let settings = registry
        .module(module)
        .ok_or_else(|| SettingsError::UnknownModule(module.to_owned()))?;
    let mut values: BTreeMap<String, Value> = settings
        .properties
        .iter()
        .filter(|(_, p)| p.scope == scope.kind())
        .map(|(key, p)| (key.clone(), p.default.clone()))
        .collect();
    for (key, stored) in db::load(conn, scope.kind().as_str(), scope.id(), module).await? {
        // A stored value for a setting the schema no longer has is ignored.
        if let Some(slot) = values.get_mut(&key) {
            *slot = stored;
        }
    }
    Ok(values)
}

/// Validates and stores `values`, all or nothing, and reports what changed.
/// Call it on the connection of a transaction: the caller writes the audit
/// entry (and any event) in the same transaction and commits.
///
/// # Errors
/// [`SettingsError::Invalid`] (nothing is stored), [`SettingsError::UnknownModule`],
/// or database errors.
pub async fn set(
    registry: &SettingsRegistry,
    conn: &mut PgConnection,
    module: &str,
    scope: Scope,
    values: &Map<String, Value>,
    updated_by: Option<Uuid>,
) -> Result<Change, SettingsError> {
    registry.validate(module, scope, values)?;
    db::lock(conn).await?;
    let before = get(registry, conn, module, scope).await?;
    let settings = registry
        .module(module)
        .ok_or_else(|| SettingsError::UnknownModule(module.to_owned()))?;
    for (key, value) in values {
        let default = settings.properties.get(key).map(|p| &p.default);
        if default == Some(value) {
            // Back to the default: no row needed.
            db::remove(conn, scope.kind().as_str(), scope.id(), module, key).await?;
        } else {
            db::upsert(
                conn,
                scope.kind().as_str(),
                scope.id(),
                module,
                key,
                value,
                updated_by,
            )
            .await?;
        }
    }
    let after = get(registry, conn, module, scope).await?;
    let changed = after
        .iter()
        .filter(|(key, value)| before.get(*key) != Some(*value))
        .map(|(key, _)| key.clone())
        .collect();
    Ok(Change {
        before,
        after,
        changed,
    })
}

fn compile(module: &str, schema: Value) -> Result<ModuleSettings, Vec<String>> {
    let mut problems = Vec::new();
    if schema.get("type").and_then(Value::as_str) != Some("object") {
        problems.push("the schema must have `\"type\": \"object\"`".to_owned());
    }
    if schema.get("additionalProperties") != Some(&Value::Bool(false)) {
        problems.push("the schema must have `\"additionalProperties\": false`".to_owned());
    }
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        problems.push("the schema needs a `properties` object".to_owned());
        return Err(problems);
    };
    let mut compiled = BTreeMap::new();
    for (key, property) in properties {
        let name = |text: &str| format!("property `{}`: {text}", clip(key));
        if !(1..=64).contains(&key.len())
            || !key.starts_with(|c: char| c.is_ascii_lowercase())
            || !key
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            problems.push(name(
                "the key must be 1-64 characters of a-z, 0-9 and `_`, starting with a letter",
            ));
            continue;
        }
        let kind = property.get("type").and_then(Value::as_str);
        if !matches!(kind, Some("string" | "integer" | "number" | "boolean")) {
            problems.push(name("`type` must be string, integer, number or boolean"));
            continue;
        }
        let scope = match property.get("x-scope") {
            None => ScopeKind::Global,
            Some(Value::String(text)) => match ScopeKind::parse(text) {
                Some(scope) => scope,
                None => {
                    problems.push(name("`x-scope` must be global, node or tenant"));
                    continue;
                }
            },
            Some(_) => {
                problems.push(name("`x-scope` must be a string"));
                continue;
            }
        };
        let Some(default) = property.get("default") else {
            problems.push(name("a `default` is required"));
            continue;
        };
        let validator = match jsonschema::validator_for(property) {
            Ok(validator) => validator,
            Err(err) => {
                problems.push(name(&format!(
                    "not a valid schema: {}",
                    clip(&err.to_string())
                )));
                continue;
            }
        };
        if let Some(first) = validator.iter_errors(default).next() {
            problems.push(name(&format!(
                "the default breaks the schema: {}",
                clip(&first.to_string())
            )));
            continue;
        }
        compiled.insert(
            key.clone(),
            Property {
                schema: property.clone(),
                default: default.clone(),
                scope,
                validator,
            },
        );
    }
    if problems.is_empty() {
        Ok(ModuleSettings {
            module: module.to_owned(),
            schema,
            properties: compiled,
        })
    } else {
        Err(problems)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;

    fn registry() -> SettingsRegistry {
        SettingsRegistry::build(vec![(
            "web".to_owned(),
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "name": {"type": "string", "minLength": 1, "maxLength": 8, "default": "panel"},
                    "workers": {"type": "integer", "minimum": 1, "maximum": 64, "default": 4},
                    "ratio": {"type": "number", "minimum": 0, "maximum": 1, "default": 0.5},
                    "debug": {"type": "boolean", "default": false},
                    "theme": {"type": "string", "enum": ["light", "dark"], "default": "light"},
                    "node_limit": {"type": "integer", "default": 10, "x-scope": "node"}
                }
            }),
        )])
        .unwrap()
    }

    fn problems(
        registry: &SettingsRegistry,
        scope: Scope,
        values: Value,
    ) -> Vec<(String, &'static str)> {
        match registry.validate("web", scope, values.as_object().unwrap()) {
            Err(SettingsError::Invalid(p)) => p.into_iter().map(|p| (p.key, p.code)).collect(),
            other => panic!("expected invalid, got {other:?}"),
        }
    }

    #[test]
    fn the_wrong_type_is_refused_and_the_right_one_accepted() {
        let r = registry();
        let ok = |values: Value| {
            r.validate("web", Scope::Global, values.as_object().unwrap())
                .is_ok()
        };
        assert!(ok(
            json!({"name": "abc", "workers": 8, "ratio": 0.25, "debug": true, "theme": "dark"})
        ));
        assert!(ok(json!({})), "nothing to change is fine");
        assert!(ok(json!({"ratio": 1})), "an integer is a number");
        for (key, bad) in [
            ("workers", json!("eight")),
            ("workers", json!(8.5)),
            ("workers", json!(0)),
            ("workers", json!(65)),
            ("name", json!(7)),
            ("name", json!("")),
            ("name", json!("far too long")),
            ("debug", json!("yes")),
            ("debug", json!(1)),
            ("theme", json!("blue")),
            ("ratio", json!(1.5)),
            ("workers", json!(null)),
            ("name", json!({"a": 1})),
        ] {
            let found = problems(&r, Scope::Global, json!({ key: bad }));
            assert_eq!(found, [(key.to_owned(), "invalid_value")], "{key} = {bad}");
        }
    }

    #[test]
    fn every_problem_is_reported_and_nothing_leaks_the_schema_machinery() {
        let r = registry();
        let found = problems(
            &r,
            Scope::Global,
            json!({"workers": "x", "ghost": 1, "theme": "blue", "node_limit": 5, "name": "ok"}),
        );
        let mut expected = vec![
            ("ghost".to_owned(), "unknown_setting"),
            ("node_limit".to_owned(), "wrong_scope"),
            ("theme".to_owned(), "invalid_value"),
            ("workers".to_owned(), "invalid_value"),
        ];
        let mut found = found;
        found.sort();
        expected.sort();
        assert_eq!(found, expected);
        // A node-scoped setting is written for a node, and only there.
        let node = Scope::Node(Uuid::now_v7());
        assert!(
            r.validate("web", node, json!({"node_limit": 5}).as_object().unwrap())
                .is_ok()
        );
        assert_eq!(
            problems(&r, node, json!({"debug": true})),
            [("debug".to_owned(), "wrong_scope")]
        );
        assert!(matches!(
            r.validate("nothing", Scope::Global, &Map::new()),
            Err(SettingsError::UnknownModule(m)) if m == "nothing"
        ));
        // Long hostile keys and messages are cut.
        let long = "k".repeat(500);
        let found = r.validate(
            "web",
            Scope::Global,
            json!({ long.clone(): 1 }).as_object().unwrap(),
        );
        let Err(SettingsError::Invalid(p)) = found else {
            panic!()
        };
        assert_eq!(p[0].key.len(), 200);
    }

    #[test]
    fn broken_schemas_are_refused_with_the_module_and_property() {
        let build = |schema: Value| {
            SettingsRegistry::build(vec![("bad".to_owned(), schema)])
                .err()
                .unwrap()
        };
        let text = |errors: Vec<SchemaError>| {
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        };
        let t = text(build(json!({"type": "array"})));
        assert!(
            t.contains("settings schema of module `bad`") && t.contains("type\": \"object"),
            "{t}"
        );
        let t = text(build(
            json!({"type": "object", "additionalProperties": false}),
        ));
        assert!(t.contains("`properties`"), "{t}");
        let base = |prop: Value| json!({"type": "object", "additionalProperties": false, "properties": {"x": prop}});
        for (prop, needle) in [
            (
                json!({"type": "object", "default": {}}),
                "`type` must be string",
            ),
            (json!({"type": "string"}), "a `default` is required"),
            (
                json!({"type": "integer", "default": "one"}),
                "the default breaks the schema",
            ),
            (
                json!({"type": "integer", "default": 99, "maximum": 10}),
                "the default breaks the schema",
            ),
            (
                json!({"type": "string", "default": "", "x-scope": "planet"}),
                "`x-scope` must be global, node or tenant",
            ),
            (
                json!({"type": "string", "default": "", "x-scope": 3}),
                "`x-scope` must be a string",
            ),
            (
                json!({"type": "string", "default": "", "pattern": "("}),
                "not a valid schema",
            ),
            (
                json!({"type": "string", "default": "", "$ref": "http://example.com/other.json"}),
                "not a valid schema",
            ),
        ] {
            let t = text(build(base(prop.clone())));
            assert!(t.contains(needle), "{prop}: {t}");
            assert!(t.contains("property `x`"), "{t}");
        }
        let t = text(build(
            json!({"type": "object", "additionalProperties": false, "properties": {"Bad Key": {"type": "string", "default": ""}}}),
        ));
        assert!(t.contains("a-z, 0-9 and `_`"), "{t}");
        // A module without settings simply is not given.
        assert_eq!(
            SettingsRegistry::build(Vec::new())
                .unwrap()
                .modules()
                .count(),
            0
        );
    }

    #[test]
    fn the_shipped_system_schema_is_valid() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../modules/system/settings.schema.json"))
                .unwrap();
        let r = SettingsRegistry::build(vec![("system".to_owned(), schema)]).unwrap();
        let system = r.module("system").unwrap();
        assert_eq!(system.properties.len(), 8);
        assert_eq!(
            system.properties["panel_name"].default,
            json!("Built Panel")
        );
        assert_eq!(system.properties["default_language"].default, json!("en"));
    }
}
