//! Which app a cluster's service is (M3): by the names it's deployed as.
//!
//! Names are compared lowercased with only letters and digits kept
//! (`nestjs-canvasmodules` = `nestjscanvasmodules`), images by their own name (no registry
//! path, tag or digest). The apps' `images` (their CI's `IMAGE_NAME`, the config's
//! `deployments`) match first; only when none does, the folder names of apps without
//! `images` do.

use crate::apps::App;

/// The app deploying as `deployment` (or its image's short name `image`): the id, `None`
/// when no app does, or a problem when several do.
pub fn app_for(deployment: &str, image: &str, apps: &[App]) -> Result<Option<String>, String> {
    let wanted: Vec<String> = [normal(deployment), normal(short(image))].into_iter().filter(|n| !n.is_empty()).collect();
    let matches = |name: &str| {
        let name = normal(name);
        !name.is_empty() && wanted.contains(&name)
    };
    let mut found: Vec<&App> = apps.iter().filter(|app| app.images.iter().any(|i| matches(short(i)))).collect();
    if found.is_empty() {
        found = apps.iter().filter(|app| app.images.is_empty() && matches(app.id.rsplit('/').next().unwrap_or(&app.id))).collect();
    }
    match found[..] {
        [] => Ok(None),
        [one] => Ok(Some(one.id.clone())),
        _ => {
            let ids: Vec<&str> = found.iter().map(|a| a.id.as_str()).collect();
            Err(format!("{deployment}: several apps match ({}); set `deployments` on one", ids.join(", ")))
        }
    }
}

/// The project an app sits in, deployed as one when its apps aren't on their own: the
/// first folder of its id (`front-end` for `front-end/packages/mf/entities-graph`);
/// `None` for an app that is a project. For now the one rule (user, 2026-11-11: a
/// monorepo served by one deployment).
pub fn project_of(app_id: &str) -> Option<&str> {
    app_id.split_once('/').map(|(project, _)| project)
}

/// Whether a deployment (or its image's short name) is named `name`, compared as
/// [`app_for`] does.
pub fn deploys_as(deployment: &str, image: &str, name: &str) -> bool {
    let name = normal(name);
    !name.is_empty() && (normal(deployment) == name || normal(short(image)) == name)
}

/// An image's own name: without its registry path, tag or digest.
fn short(image: &str) -> &str {
    let name = image.rsplit('/').next().unwrap_or(image);
    let name = name.split('@').next().unwrap_or(name);
    name.split(':').next().unwrap_or(name)
}

/// Lowercase, letters and digits only.
fn normal(name: &str) -> String {
    name.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::EnvApply;
    use crate::config::{AppOverride, apply_overrides};
    use std::collections::BTreeMap;

    #[test]
    fn a_monorepos_apps_deploy_as_their_project() {
        assert_eq!(project_of("front-end/packages/mf/entities-graph"), Some("front-end"));
        assert_eq!(project_of("graphservice"), None);
        assert!(deploys_as("front-end", "registry.example/web/front-end:1.2.0", "front-end"));
        assert!(deploys_as("web", "registry.example/front-end:1.2.0", "front-end"), "by its image");
        assert!(!deploys_as("graphservice", "graphservice:1.0", "front-end"));
    }

    fn app(id: &str, images: &[&str]) -> App {
        App {
            id: id.into(),
            name: id.rsplit('/').next().unwrap().into(),
            folder: format!("~/{id}"),
            dir: format!("/{id}").into(),
            kind: "go".into(),
            library: false,
            facts: Default::default(),
            commands: vec![],
            env: EnvApply::None,
            urls: vec![],
            images: images.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn apps() -> Vec<App> {
        vec![
            app("nestjs-canvasmodules", &["nestjscanvasmodules"]),
            app("entityservice", &[]),
            app("boardservice", &["boardservice"]),
            app("front-end/mastertadaapp", &[]),
        ]
    }

    #[test]
    fn the_ci_image_name_links_a_differently_named_folder() {
        let apps = apps();
        assert_eq!(app_for("canvasmodulesservice", "nestjscanvasmodules", &apps), Ok(Some("nestjs-canvasmodules".into())));
        let full = "europe-west1-docker.pkg.dev/p/registry06/nestjscanvasmodules:feature-x";
        assert_eq!(app_for("canvasmodulesservice", full, &apps), Ok(Some("nestjs-canvasmodules".into())), "a full image reference");
    }

    #[test]
    fn the_folder_name_links_an_app_without_images() {
        let apps = apps();
        assert_eq!(app_for("entityservice", "entityservice", &apps), Ok(Some("entityservice".into())));
        assert_eq!(app_for("EntityService", "", &apps), Ok(Some("entityservice".into())), "case and punctuation don't count");
        assert_eq!(app_for("master-tada-app", "x", &apps), Ok(Some("front-end/mastertadaapp".into())), "a monorepo child by its own folder");
        let other = [app("nestjs-canvasmodules", &["other"])];
        assert_eq!(app_for("nestjscanvasmodules", "", &other), Ok(None), "images, when an app has some, decide");
    }

    #[test]
    fn an_image_no_app_deploys_as_is_no_link() {
        assert_eq!(app_for("foreignapiservice", "foreignapiservicev2", &apps()), Ok(None));
        assert_eq!(app_for("", "", &[app("weird", &["--"])]), Ok(None), "empty names match nothing");
    }

    #[test]
    fn an_explicit_image_wins_over_a_folder_name() {
        let apps = vec![app("a/entityservice", &["entityservice"]), app("b/entity-service", &[])];
        assert_eq!(app_for("entityservice", "entityservice", &apps), Ok(Some("a/entityservice".into())));
        let apps = vec![app("a/entity", &["registry.example/team/entityservice:main"]), app("b/entity-service", &[])];
        assert_eq!(app_for("entityservice", "", &apps), Ok(Some("a/entity".into())), "images compared by their own name");
    }

    #[test]
    fn several_matches_are_a_problem() {
        let apps = vec![app("a/entityservice", &[]), app("b/entity-service", &[])];
        let problem = "entityservice: several apps match (a/entityservice, b/entity-service); set `deployments` on one";
        assert_eq!(app_for("entityservice", "entityservice", &apps), Err(problem.into()));
    }

    #[test]
    fn an_override_wins_over_the_implicit_names() {
        let found = vec![app("a/entityservice", &[]), app("b/entity-service", &[]), app("foreign", &[])];
        let overrides = BTreeMap::from([
            ("b/entity-service".to_string(), AppOverride { deployments: vec!["entityworker".into()], ..Default::default() }),
            ("foreign".to_string(), AppOverride { deployments: vec!["foreignapiservice".into()], ..Default::default() }),
        ]);
        let (apps, _) = apply_overrides(found, &overrides);
        assert_eq!(app_for("entityservice", "entityservice", &apps), Ok(Some("a/entityservice".into())), "the override removes its folder name");
        assert_eq!(app_for("entityworker", "entityworker", &apps), Ok(Some("b/entity-service".into())));
        assert_eq!(app_for("foreignapiservice", "foreignapiservicev2", &apps), Ok(Some("foreign".into())));
    }
}
