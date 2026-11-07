//! The kubeconfigs of a folder and the target of each env.

use devdeck_core::config::shown;
use devdeck_core::fnv1a;
use kube::config::Kubeconfig;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Every context of the kubeconfigs in a folder, with the file it comes from.
#[derive(Clone, Debug, Default)]
pub struct Kubeconfigs {
    folder: PathBuf,
    contexts: BTreeMap<String, Source>,
    /// Contexts named in more than one file: the other files (the first one is used).
    duplicates: BTreeMap<String, Vec<PathBuf>>,
    /// Why the folder couldn't be read: what every target says.
    unreadable: Option<String>,
}

/// The file a context comes from.
#[derive(Clone, Debug)]
struct Source {
    file: PathBuf,
    fingerprint: u64,
    config: Parsed,
}

impl Kubeconfigs {
    /// Reads every kubeconfig at the top of `folder`; returns the problems found (a
    /// context in two files is one only for an env that uses it: [`Kubeconfigs::duplicate`]).
    pub fn load(folder: &Path) -> (Kubeconfigs, Vec<String>) {
        let mut found = Kubeconfigs { folder: folder.to_path_buf(), ..Default::default() };
        let mut problems = Vec::new();
        let entries = match std::fs::read_dir(folder) {
            Ok(entries) => entries,
            Err(e) => {
                let why = if e.kind() == std::io::ErrorKind::NotFound { format!("{} doesn't exist", shown(folder)) } else { format!("{}: {e}", shown(folder)) };
                problems.push(why.clone());
                found.unreadable = Some(why);
                return (found, problems);
            }
        };
        let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| read(p)).collect();
        files.sort();
        for file in files {
            let bytes = match std::fs::read(&file) {
                Ok(bytes) => bytes,
                Err(e) => {
                    problems.push(format!("{}: {e}", shown(&file)));
                    continue;
                }
            };
            let config = match Kubeconfig::read_from(&file) {
                Ok(config) => Parsed(Arc::new(config)),
                Err(e) => {
                    problems.push(format!("{}: not a kubeconfig: {e}", shown(&file)));
                    continue;
                }
            };
            let fingerprint = fnv1a(&bytes);
            for named in &config.0.contexts {
                match found.contexts.get(&named.name) {
                    Some(_) => found.duplicates.entry(named.name.clone()).or_default().push(file.clone()),
                    None => {
                        let source = Source { file: file.clone(), fingerprint, config: config.clone() };
                        found.contexts.insert(named.name.clone(), source);
                    }
                }
            }
        }
        (found, problems)
    }

    /// Why `context` is ambiguous, when it's in more than one file.
    pub fn duplicate(&self, context: &str) -> Option<String> {
        let others = self.duplicates.get(context)?;
        let first = &self.contexts.get(context)?.file;
        let others: Vec<String> = others.iter().map(|f| shown(f)).collect();
        Some(format!("context {context} is in {} and {}: {} is used", shown(first), others.join(", "), shown(first)))
    }

    /// Where `env` lives: the kubeconfig holding `context`.
    pub fn target(&self, env: &str, context: &str) -> Result<Target, String> {
        if let Some(why) = &self.unreadable {
            return Err(why.clone());
        }
        let source = self.contexts.get(context).ok_or_else(|| format!("no context {context} in {}", shown(&self.folder)))?;
        let named = source.config.0.contexts.iter().find(|c| c.name == context).and_then(|c| c.context.as_ref());
        Ok(Target {
            env: env.to_string(),
            file: source.file.clone(),
            context: context.to_string(),
            namespace: named.and_then(|c| c.namespace.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| "default".into()),
            cluster: named.map(|c| c.cluster.clone()).filter(|c| !c.is_empty()).unwrap_or_else(|| context.to_string()),
            fingerprint: source.fingerprint,
            config: source.config.clone(),
        })
    }
}

/// Where an env's resources are read.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub env: String,
    pub file: PathBuf,
    pub context: String,
    pub namespace: String,
    /// The context's cluster, for messages (`km011gcp`).
    pub cluster: String,
    /// Of the file's content: a changed file (a rotated token) reconnects.
    pub fingerprint: u64,
    /// The file as read, to build its client from.
    pub(crate) config: Parsed,
}

impl Target {
    /// The user the context connects as.
    pub(crate) fn user(&self) -> String {
        let named = self.config.0.contexts.iter().find(|c| c.name == self.context);
        named.and_then(|c| c.context.as_ref()?.user.clone()).unwrap_or_default()
    }

    /// A target without a kubeconfig behind it, for the driver's tests.
    #[cfg(test)]
    pub(crate) fn for_tests(env: &str, cluster: &str) -> Target {
        Target {
            env: env.into(),
            file: PathBuf::from("kubeconfig"),
            context: env.into(),
            namespace: env.into(),
            cluster: cluster.into(),
            fingerprint: 0,
            config: Parsed(Arc::default()),
        }
    }
}

/// A parsed kubeconfig. Equal to any other (the fingerprint stands for its content), and
/// shown without its content (its tokens).
#[derive(Clone)]
pub(crate) struct Parsed(pub(crate) Arc<Kubeconfig>);

impl PartialEq for Parsed {
    fn eq(&self, _: &Parsed) -> bool {
        true
    }
}

impl std::fmt::Debug for Parsed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Kubeconfig")
    }
}

/// Whether a folder's entry is read as a kubeconfig: regular files, except hidden ones and
/// notes (`*.md`, `*.txt`).
fn read(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return false };
    let note = path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("txt"));
    !name.starts_with('.') && !note && path.is_file()
}


#[cfg(test)]
mod tests {
    use super::*;
    use devdeck_core::testing::TempDir;

    /// A kubeconfig with one user and one cluster; each context's namespace is its name
    /// unless `None`.
    fn kubeconfig(cluster: &str, user: &str, contexts: &[(&str, Option<&str>)]) -> String {
        let mut text = format!(
            "apiVersion: v1\nkind: Config\nclusters:\n- name: {cluster}\n  cluster:\n    server: https://{cluster}.example.test\nusers:\n- name: {user}\n  user:\n    token: fake-token-{cluster}\ncontexts:\n"
        );
        for (name, namespace) in contexts {
            text += &format!("- name: {name}\n  context:\n    cluster: {cluster}\n    user: {user}\n");
            if let Some(ns) = namespace {
                text += &format!("    namespace: {ns}\n");
            }
        }
        text
    }

    /// The real folder's layout, with fake tokens.
    fn folder() -> TempDir {
        let dir = TempDir::new("kubeconfigs");
        let devs: Vec<_> = ["dev31", "dev32", "dev33", "dev35", "dev36"].map(|c| (c, Some(c))).into();
        std::fs::write(dir.join("dev-km011gcp"), kubeconfig("km011gcp", "tailscale-auth", &devs)).unwrap();
        for ns in ["hecate", "mnemosyne", "empusa"] {
            let ctx = format!("hades00-{ns}");
            std::fs::write(dir.join(&ctx), kubeconfig("km009gcp", &format!("{ns}-user"), &[(&ctx, Some(ns))])).unwrap();
        }
        std::fs::write(dir.join("README.md"), "# Kubeconfigs\n\nOne file per cluster.\n").unwrap();
        std::fs::create_dir(dir.join("qa")).unwrap();
        std::fs::write(dir.join("qa/qa-km011gcp"), kubeconfig("km011gcp", "qa", &[("qa1", Some("qa1"))])).unwrap();
        dir
    }

    #[test]
    fn every_context_of_the_folder_is_found_with_its_file() {
        let dir = folder();
        let (configs, problems) = Kubeconfigs::load(&dir);
        assert_eq!(problems, Vec::<String>::new(), "the README and qa/ aren't read");
        assert_eq!(configs.contexts.keys().collect::<Vec<_>>(), ["dev31", "dev32", "dev33", "dev35", "dev36", "hades00-empusa", "hades00-hecate", "hades00-mnemosyne"]);
        let t = configs.target("dev35", "dev35").unwrap();
        assert_eq!((t.env.as_str(), t.file.as_path(), t.namespace.as_str(), t.cluster.as_str(), t.user()), ("dev35", dir.join("dev-km011gcp").as_path(), "dev35", "km011gcp", "tailscale-auth".into()));
        let hades = configs.target("hades", "hades00-hecate").unwrap();
        assert_eq!((hades.file.as_path(), hades.context.as_str(), hades.namespace.as_str(), hades.cluster.as_str()), (dir.join("hades00-hecate").as_path(), "hades00-hecate", "hecate", "km009gcp"));
        assert_eq!(t.fingerprint, configs.target("dev36", "dev36").unwrap().fingerprint, "same file");
        assert_ne!(t.fingerprint, hades.fingerprint);
        assert!(!format!("{t:?}").contains("fake-token"), "tokens aren't shown");
    }

    #[test]
    fn a_context_without_namespace_is_in_default() {
        let dir = TempDir::new("kubeconfigs-default");
        std::fs::write(dir.join("lab"), kubeconfig("km009gcp", "u", &[("lab", None)])).unwrap();
        let (configs, _) = Kubeconfigs::load(&dir);
        assert_eq!(configs.target("lab", "lab").unwrap().namespace, "default");
    }

    #[test]
    fn a_changed_file_changes_the_fingerprint_only() {
        let dir = folder();
        let before = Kubeconfigs::load(&dir).0.target("dev35", "dev35").unwrap();
        let rotated = kubeconfig("km011gcp", "tailscale-auth", &[("dev35", Some("dev35"))]).replace("fake-token", "rotated-token");
        std::fs::write(dir.join("dev-km011gcp"), rotated).unwrap();
        let after = Kubeconfigs::load(&dir).0.target("dev35", "dev35").unwrap();
        assert_ne!(before, after);
        assert_eq!(Target { fingerprint: before.fingerprint, ..after }, before);
    }

    #[test]
    fn duplicates_broken_and_empty_files_are_problems_or_nothing() {
        let dir = folder();
        std::fs::write(dir.join("a-dup"), kubeconfig("other", "u", &[("dev35", Some("x"))])).unwrap();
        std::fs::write(dir.join("broken"), "clusters: [unclosed\n").unwrap();
        std::fs::write(dir.join("empty"), "").unwrap();
        std::fs::write(dir.join(".hidden"), "not: [yaml").unwrap();
        std::fs::write(dir.join("notes.txt"), "not: [yaml").unwrap();
        let (configs, problems) = Kubeconfigs::load(&dir);
        let dup = shown(&dir.join("a-dup"));
        let real = shown(&dir.join("dev-km011gcp"));
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].starts_with(&format!("{}: not a kubeconfig: ", shown(&dir.join("broken")))), "{problems:?}");
        assert_eq!(configs.duplicate("dev35"), Some(format!("context dev35 is in {dup} and {real}: {dup} is used")));
        assert_eq!(configs.duplicate("dev33"), None);
        assert_eq!(configs.target("dev35", "dev35").unwrap().cluster, "other", "the first file by name wins");
    }

    #[test]
    fn missing_contexts_and_folders() {
        let dir = folder();
        let (configs, _) = Kubeconfigs::load(&dir);
        assert_eq!(configs.target("dls", "nope").unwrap_err(), format!("no context nope in {}", shown(&dir)));
        let gone = dir.join("gone");
        let (configs, problems) = Kubeconfigs::load(&gone);
        assert_eq!(problems, [format!("{} doesn't exist", shown(&gone))]);
        assert_eq!(configs.target("dev35", "dev35").unwrap_err(), format!("{} doesn't exist", shown(&gone)), "the cause, not a missing context");
    }

    #[test]
    fn the_home_folder_is_shown_as_tilde() {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from).filter(|h| h.parent().is_some()) else { return };
        assert_eq!(shown(&home.join("kc/dev")), "~/kc/dev");
        assert_eq!(shown(&home), "~");
        assert_eq!(shown(Path::new("/srv/kc")), "/srv/kc");
    }
}
