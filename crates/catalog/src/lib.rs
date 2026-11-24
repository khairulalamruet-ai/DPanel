//! The catalog of applications (T12.05): one TOML file per application in `deploy/catalog/`,
//! read into [`AppDefinition`]s and checked. Adding an application is adding a file; the
//! engine that installs them (on the node) only knows the fixed steps of the definitions.
//!
//! The files shipped with the build are compiled in (see `build.rs`); an operator can add
//! more in a directory (`BUILTPANEL_CATALOG_DIR`), where a file with the slug of a built-in one
//! replaces it.

use std::{collections::BTreeMap, path::Path};

use builtpanel_node_protocol::AppDefinition;

pub mod marketplace;

include!(concat!(env!("OUT_DIR"), "/catalog_files.rs"));

/// Why the catalog could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogError {
    /// The file.
    pub file: String,
    /// What is wrong with it.
    pub problem: String,
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.file, self.problem)
    }
}

impl std::error::Error for CatalogError {}

/// The applications of the catalog, by slug.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    apps: BTreeMap<String, AppDefinition>,
}

/// Reads and checks one definition file.
///
/// # Errors
/// [`CatalogError`].
pub fn parse(file: &str, text: &str) -> Result<AppDefinition, CatalogError> {
    let wrong = |problem: String| CatalogError {
        file: file.to_owned(),
        problem,
    };
    if text.len() > 256 * 1024 {
        return Err(wrong("larger than 256 KiB".into()));
    }
    let definition: AppDefinition =
        toml::from_str(text).map_err(|e| wrong(e.to_string().replace('\n', " ")))?;
    definition.checked().map_err(wrong)?;
    if file
        .strip_suffix(".toml")
        .is_some_and(|stem| stem != definition.slug)
    {
        return Err(wrong(format!(
            "the file is called {file} but the slug is {}",
            definition.slug
        )));
    }
    Ok(definition)
}

impl Catalog {
    /// The files shipped with this build.
    ///
    /// # Errors
    /// [`CatalogError`] (a build mistake; tested).
    pub fn embedded() -> Result<Self, CatalogError> {
        let mut catalog = Self::default();
        for (file, text) in FILES {
            catalog.add(parse(file, text)?)?;
        }
        Ok(catalog)
    }

    fn add(&mut self, definition: AppDefinition) -> Result<(), CatalogError> {
        self.apps.insert(definition.slug.clone(), definition);
        Ok(())
    }

    /// The built-in files and the `*.toml` files of `dir` (a bad file there is an error, not
    /// silently left out).
    ///
    /// # Errors
    /// [`CatalogError`].
    pub fn with_dir(dir: &Path) -> Result<Self, CatalogError> {
        let mut catalog = Self::embedded()?;
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| CatalogError {
                file: dir.display().to_string(),
                problem: e.to_string(),
            })?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .collect();
        files.sort();
        for path in files {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_owned();
            let text = std::fs::read_to_string(&path).map_err(|e| CatalogError {
                file: name.clone(),
                problem: e.to_string(),
            })?;
            catalog.add(parse(&name, &text)?)?;
        }
        Ok(catalog)
    }

    /// Every application, by slug.
    pub fn all(&self) -> impl Iterator<Item = &AppDefinition> {
        self.apps.values()
    }

    /// One application.
    #[must_use]
    pub fn get(&self, slug: &str) -> Option<&AppDefinition> {
        self.apps.get(slug)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_catalog_is_valid_and_has_the_first_applications() {
        let catalog = Catalog::embedded().unwrap();
        let slugs: Vec<&str> = catalog.all().map(|a| a.slug.as_str()).collect();
        for wanted in ["wordpress", "joomla", "laravel", "drupal", "static-site"] {
            assert!(slugs.contains(&wanted), "{wanted} is missing: {slugs:?}");
        }
        assert!(catalog.get("joomla").unwrap().needs_admin());
        assert!(!catalog.get("static-site").unwrap().needs_database);
        assert_eq!(catalog.get("laravel").unwrap().document_root, "public");
        assert_eq!(catalog.get("drupal").unwrap().document_root, "web");
    }

    #[test]
    fn a_new_application_is_a_file_and_a_bad_one_is_refused() {
        let dir = std::env::temp_dir().join(format!("bp-catalog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("notes.toml"),
            "slug = 'notes'\nname = 'Notes'\nversion = '1.0'\n[[steps]]\ntype = 'write_file'\npath = 'index.html'\ncontent = '<p>{{title}}</p>'\n",
        )
        .unwrap();
        let catalog = Catalog::with_dir(&dir).unwrap();
        assert_eq!(catalog.get("notes").unwrap().name, "Notes");
        assert!(catalog.get("joomla").is_some(), "the built-in ones stay");
        // Refused: an unknown placeholder, a path that climbs out, a plain-http source,
        // the administrator's password as file text, a name that is not the slug.
        for (file, body) in [
            (
                "a1.toml",
                "slug='a1'\nname='A'\nversion='1'\n[[steps]]\ntype='write_file'\npath='x'\ncontent='{{shell}}'\n",
            ),
            (
                "a2.toml",
                "slug='a2'\nname='A'\nversion='1'\n[[steps]]\ntype='write_file'\npath='../x'\ncontent='x'\n",
            ),
            (
                "a3.toml",
                "slug='a3'\nname='A'\nversion='1'\n[source]\nurl='http://x.example/a.zip'\nsha256='0000000000000000000000000000000000000000000000000000000000000000'\nformat='zip'\n",
            ),
            (
                "a4.toml",
                "slug='a4'\nname='A'\nversion='1'\n[[steps]]\ntype='write_file'\npath='x'\ncontent='{{admin_password}}'\n",
            ),
            ("a5.toml", "slug='other'\nname='A'\nversion='1'\n"),
            ("a6.toml", "slug='a6'\nname='A'\nversion='1'\nsurprise=1\n"),
            (
                "a7.toml",
                "slug='a7'\nname='A'\nversion='1'\n[[steps]]\ntype='php'\nargs=['{{db_name}}']\n",
            ),
        ] {
            assert!(parse(file, body).is_err(), "{file} should be refused");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
