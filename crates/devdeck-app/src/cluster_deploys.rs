//! Deploys (R7.3, R7.4, R7.6): changing a deployment's branch, following every deploy to
//! its end, and devdeck's history of them.
//!
//! - A deploy changes the main container's image only, pinned to the image confirmed
//!   (the driver's write fails if the cluster changed meanwhile). Guards, below the UI:
//!   edit mode and the driver ([`Cluster::writes`]), the account (the access review's
//!   answer, when known), a protected env unlocked, no write of the same service in flight,
//!   not paused, a container that is clearly the main one, another image.
//! - Deploys are spotted on the deployment watch: a revision that grows (or an image that
//!   changes) is one, whoever made it: devdeck's own when it matches the write in flight,
//!   else the tool that changed the spec (`managedFields`), else "seen". The first list of
//!   an env is the baseline: nothing before it is a deploy.
//! - Each deploy is followed ([`assess`]) on every update of its deployment and pods until
//!   done, stuck or replaced; failing keeps following. devdeck's own hold their service's
//!   pods while followed; others are followed from what's known (pods when someone holds
//!   them).
//! - The history keeps [`HISTORY_KEPT`] deploys per service in the state folder; one left
//!   without an end (devdeck closed mid-rollout) is followed again if the deployment still
//!   runs its image, else ends as "unknown: devdeck was closed".
//! - The access review (may this account patch deployments here?) is asked once per env
//!   and log generation, when a service view opens.

use crate::cluster::{Cluster, PodScope, PodsInterest};
use devdeck_core::cluster::{DeployRecord, Deployment, HISTORY_KEPT, Progress, Rollout, RolloutState, assess, check_write, remember};
use devdeck_core::state::{Deploys, Loaded, StateStore};
use devdeck_kube::ImageChange;
use gpui::{Context, Task};
use std::collections::BTreeMap;

type Key = (String, String);

/// A write as the view that asked sees it.
#[derive(Clone, Debug, PartialEq)]
pub enum WriteState {
    Sending,
    /// The cluster took it; the rollout is followed from now on.
    Sent,
    Failed(String),
}

/// What the account may do in an env, as the cluster said.
#[derive(Clone, Debug, PartialEq)]
pub enum Access {
    Asking,
    Allowed,
    Denied,
    /// The review itself failed: the write decides.
    Unknown,
}

/// devdeck's own write of a service, from its sending until the watch sees it.
struct Pending {
    to_image: String,
    /// The generation the cluster gave it, once answered.
    generation: Option<u64>,
}

#[derive(Default)]
pub(crate) struct DeployState {
    rollouts: BTreeMap<Key, Rollout>,
    history: BTreeMap<Key, Vec<DeployRecord>>,
    /// Each deployment's revision and the main image it carried when that revision came
    /// (the image changes in the spec before the controller moves the revision).
    versions: BTreeMap<Key, (u64, String)>,
    pending: BTreeMap<Key, Pending>,
    writes: BTreeMap<Key, WriteState>,
    /// Each service's last write task.
    sending: BTreeMap<Key, Task<()>>,
    /// devdeck's own deploys hold their service's pods while followed.
    following: BTreeMap<Key, PodsInterest>,
    /// Per env: the generation it was asked for, and the answer.
    access: BTreeMap<String, ((u64, u64), Access)>,
    asking: BTreeMap<String, Task<()>>,
    /// Writes a confirmation waits on; a write nobody waits on says its refusal in a toast.
    watched: std::collections::BTreeSet<Key>,
    store: Option<(StateStore, String)>,
    saving: Option<Task<()>>,
    /// The history changed while a save ran: saved again after it.
    save_again: bool,
}

/// The image `d` would run with `tag`: its main container's repository and the tag.
fn image_with(d: &Deployment, tag: &str) -> Option<String> {
    let (_, c) = d.deploy_target().ok()?;
    Some(format!("{}:{tag}", c.image.repo))
}

impl Cluster {
    /// Reads the history kept in `store` for `workspace`, and keeps it there from now on.
    pub fn keep_history(&mut self, store: StateStore, workspace: &str, cx: &mut Context<Self>) {
        if let Loaded::Ok(d) = store.load_deploys(workspace) {
            for (env, services) in d.envs {
                for (name, records) in services {
                    self.dep.history.insert((env.clone(), name), records);
                }
            }
        }
        self.dep.store = Some((store, workspace.to_string()));
        cx.notify();
    }

    /// The deploy followed (or last followed) of a service.
    pub fn rollout(&self, env: &str, name: &str) -> Option<&Rollout> {
        self.dep.rollouts.get(&(env.to_string(), name.to_string()))
    }

    /// A service's deploys as devdeck saw them, oldest first.
    pub fn history(&self, env: &str, name: &str) -> &[DeployRecord] {
        self.dep.history.get(&(env.to_string(), name.to_string())).map_or(&[], |h| h.as_slice())
    }

    /// devdeck's own write of a service, as its view shows it.
    pub fn write_state(&self, env: &str, name: &str) -> Option<&WriteState> {
        self.dep.writes.get(&(env.to_string(), name.to_string()))
    }

    /// Whether a confirmation waits on `name`'s write (it shows the answer itself).
    pub fn watch_write(&mut self, env: &str, name: &str, watched: bool) {
        let key = (env.to_string(), name.to_string());
        if watched {
            self.dep.watched.insert(key);
        } else {
            self.dep.watched.remove(&key);
        }
    }

    /// Forgets a finished write's state (its confirmation closed).
    pub fn clear_write(&mut self, env: &str, name: &str) {
        let key = (env.to_string(), name.to_string());
        if self.dep.writes.get(&key).is_some_and(|w| *w != WriteState::Sending) {
            self.dep.writes.remove(&key);
        }
    }

    /// What the account may do in `env` (deployments), as last asked.
    pub fn access(&self, env: &str) -> Option<&Access> {
        let generation = self.log_generation(env);
        self.dep.access.get(env).filter(|(g, _)| *g == generation).map(|(_, a)| a)
    }

    /// Asks the cluster whether deployments may be changed in `env`, unless it was asked
    /// for this generation already.
    pub fn check_access(&mut self, env: &str, cx: &mut Context<Self>) {
        if self.env(env).is_none() || self.access(env).is_some() {
            return;
        }
        let generation = self.log_generation(env);
        let driver = self.driver.clone();
        // Not asked (the kubeconfigs aren't read yet): asked again next time.
        if let Some(task) = driver.can_deploy(env, cx) {
            self.dep.access.insert(env.to_string(), (generation, Access::Asking));
            self.dep.asking.insert(env.to_string(), task);
        }
    }

    /// The access review's answer.
    pub fn access_known(&mut self, env: &str, result: Result<bool, String>, cx: &mut Context<Self>) {
        // Filed under the generation it was asked for: a reload meanwhile asks again.
        let generation = self.dep.access.get(env).map_or_else(|| self.log_generation(env), |(g, _)| *g);
        let access = match result {
            Ok(true) => Access::Allowed,
            Ok(false) => Access::Denied,
            Err(_) => Access::Unknown,
        };
        self.dep.access.insert(env.to_string(), (generation, access));
        self.dep.asking.remove(env);
        cx.notify();
    }

    /// Why `to` can't be deployed on `name` in `env` now, if it can't.
    pub fn deploy_blocked(&self, env: &str, name: &str, to: &str, unlocked: bool, cx: &gpui::App) -> Option<String> {
        if let Err(why) = self.writes(cx) {
            return Some(why);
        }
        if self.access(env) == Some(&Access::Denied) {
            return Some(format!("your account can't change deployments in {env}"));
        }
        let e = self.env(env)?;
        if let Err(why) = check_write(e, unlocked) {
            return Some(why);
        }
        let key = (env.to_string(), name.to_string());
        if self.dep.writes.get(&key) == Some(&WriteState::Sending) {
            return Some(format!("a deploy of {name} is being sent"));
        }
        let Some(d) = self.deployment(env, name) else { return Some(format!("{name} isn't deployed in {env}")) };
        if d.progress == Progress::Paused {
            return Some(format!("rollouts of {name} are paused"));
        }
        let (_, c) = match d.deploy_target() {
            Ok(t) => t,
            Err(why) => return Some(why),
        };
        if image_with(d, to).as_deref() == Some(c.image.raw.as_str()) {
            return Some(format!("{name} already runs {to}"));
        }
        None
    }

    /// Deploys `to` on `name` in `env`, pinned to `from` (the image confirmed). The
    /// answer comes through [`Cluster::write_done`]; the rollout is followed from the watch.
    pub fn deploy(&mut self, env: &str, name: &str, from: &str, to: &str, unlocked: bool, cx: &mut Context<Self>) -> Result<(), String> {
        if let Some(why) = self.deploy_blocked(env, name, to, unlocked, cx) {
            return Err(why);
        }
        let d = self.deployment(env, name).ok_or_else(|| format!("{name} isn't deployed in {env}"))?;
        let (index, c) = d.deploy_target()?;
        let change = ImageChange { index, container: c.name.clone(), from: from.to_string(), to: format!("{}:{to}", c.image.repo) };
        let key = (env.to_string(), name.to_string());
        self.dep.pending.insert(key.clone(), Pending { to_image: change.to.clone(), generation: None });
        self.dep.writes.insert(key.clone(), WriteState::Sending);
        let interest = self.pods_interest(env, PodScope::Service(name.to_string()), cx);
        self.dep.following.insert(key.clone(), interest);
        let driver = self.driver.clone();
        match driver.set_image(env, name, change, cx) {
            Some(task) => {
                self.dep.sending.insert(key, task);
            }
            None => self.write_done(env, name, Err("this cluster can't deploy".into()), cx),
        }
        cx.notify();
        Ok(())
    }

    /// The cluster's answer to devdeck's write: the generation it gave the new spec, or why
    /// not.
    pub fn write_done(&mut self, env: &str, name: &str, result: Result<u64, String>, cx: &mut Context<Self>) {
        let key = (env.to_string(), name.to_string());
        // The driver's task stays until the next write of the service (the fake's goes on
        // rolling after it answered).
        match result {
            Ok(generation) => {
                if let Some(p) = self.dep.pending.get_mut(&key) {
                    p.generation = Some(generation);
                }
                // The watch may have seen it first.
                if let Some(r) = self.dep.rollouts.get_mut(&key)
                    && r.by == "devdeck"
                    && !r.finished()
                {
                    r.generation = r.generation.max(generation);
                }
                if self.dep.watched.contains(&key) {
                    self.dep.writes.insert(key, WriteState::Sent);
                } else {
                    self.dep.writes.remove(&key);
                }
                self.reassess(env, cx);
            }
            Err(why) => {
                self.dep.pending.remove(&key);
                // Seen already on the watch: it's followed as anyone's deploy would be.
                if !self.dep.rollouts.get(&key).is_some_and(|r| r.by == "devdeck" && !r.finished()) {
                    self.dep.following.remove(&key);
                }
                if self.dep.watched.contains(&key) {
                    self.dep.writes.insert(key, WriteState::Failed(why));
                } else {
                    self.dep.writes.remove(&key);
                    devdeck_ui::toasts(cx).update(cx, |t, cx| t.push(devdeck_ui::ToastKind::Err, format!("{env} › {name}: not deployed: {why}"), vec![], cx));
                }
            }
        }
        cx.notify();
    }

    /// A deployments list came for `env`: spots deploys, follows them.
    pub(crate) fn saw_deployments(&mut self, env: &str, cx: &mut Context<Self>) {
        let Some(map) = self.state(env).and_then(|s| s.deployments.clone()) else { return };
        let now = self.now_s();
        let mut changed = false;
        for d in map.values() {
            let key = (env.to_string(), d.name.clone());
            let image = d.main().image.raw.clone();
            let tag = d.tag();
            let Some(before) = self.dep.versions.get(&key).cloned() else {
                // The baseline; a deploy left open by a devdeck that closed resumes or ends.
                self.dep.versions.insert(key.clone(), (d.revision, image));
                changed |= self.reopen(&key, d, now);
                continue;
            };
            let by_revision = d.revision > 0 && before.0 > 0;
            let deployed = if by_revision { d.revision > before.0 } else { image != before.1 };
            // A new image under the same revision: the controller hasn't moved it yet.
            if deployed || !by_revision {
                self.dep.versions.insert(key.clone(), (d.revision, image.clone()));
            }
            if !deployed {
                continue;
            }
            let from = devdeck_core::cluster::Image::parse(&before.1).label();
            let ours = self.dep.pending.get(&key).is_some_and(|p| p.to_image == image);
            let by = if ours { "devdeck".to_string() } else { d.manager.clone().unwrap_or_else(|| "seen".into()) };
            let generation = if ours { self.dep.pending.get(&key).and_then(|p| p.generation).unwrap_or(d.generation) } else { d.generation };
            if ours {
                self.dep.pending.remove(&key);
            } else if self.dep.pending.contains_key(&key) && self.dep.writes.get(&key) != Some(&WriteState::Sending) {
                // devdeck's write was answered, but someone else's came after it.
                self.dep.pending.remove(&key);
            }
            if let Some(old) = self.dep.rollouts.get_mut(&key)
                && !old.finished()
            {
                old.state = RolloutState::Replaced { by: tag.clone() };
                let outcome = old.outcome();
                self.end_record(&key, &outcome);
            }
            if !ours {
                self.dep.following.remove(&key);
            }
            let r = Rollout { from: from.clone(), to: tag.clone(), image, by: by.clone(), generation, at_s: now, state: RolloutState::Rolling { ready: 0, of: d.replicas } };
            self.dep.rollouts.insert(key.clone(), r);
            remember(self.dep.history.entry(key).or_default(), DeployRecord { at_s: now, from, to: tag, by, outcome: String::new() });
            changed = true;
        }
        changed |= self.reassess_quiet(env);
        if changed {
            self.save_history(cx);
            cx.notify();
        }
    }

    /// At the first list of an env, a deploy whose record has no end: followed again if the
    /// deployment still runs its image, else ended as unknown.
    fn reopen(&mut self, key: &Key, d: &Deployment, now: u64) -> bool {
        if self.dep.rollouts.get(key).is_some_and(|r| !r.finished()) {
            return false;
        }
        let open = |r: &&DeployRecord| r.outcome.is_empty() || r.outcome.starts_with("failing");
        let Some(last) = self.dep.history.get(key).and_then(|h| h.last()).filter(open).cloned() else { return false };
        if d.tag() == last.to {
            let r = Rollout { from: last.from, to: last.to, image: d.main().image.raw.clone(), by: last.by, generation: 0, at_s: last.at_s.min(now), state: RolloutState::Rolling { ready: 0, of: d.replicas } };
            self.dep.rollouts.insert(key.clone(), r);
        } else {
            self.end_record(key, "unknown: devdeck was closed");
        }
        true
    }

    /// Follows `env`'s deploys again (pods or deployments changed).
    pub(crate) fn reassess(&mut self, env: &str, cx: &mut Context<Self>) {
        if self.reassess_quiet(env) {
            self.save_history(cx);
            cx.notify();
        }
    }

    /// Updates `env`'s unfinished rollouts; whether anything changed.
    fn reassess_quiet(&mut self, env: &str) -> bool {
        let keys: Vec<Key> = self.dep.rollouts.iter().filter(|((e, _), r)| e == env && !r.finished()).map(|(k, _)| k.clone()).collect();
        let mut changed = false;
        for key in keys {
            let Some(d) = self.deployment(&key.0, &key.1).cloned() else {
                // Deleted while followed (its env listed without it).
                if self.peek(&key.0).ready().is_some() {
                    self.dep.rollouts.remove(&key);
                    self.dep.following.remove(&key);
                    self.end_record(&key, "the deployment is gone");
                    changed = true;
                }
                continue;
            };
            let pods = self.pods(&key.0, &key.1).to_vec();
            let Some(r) = self.dep.rollouts.get_mut(&key) else { continue };
            let state = assess(r, &d, &pods);
            if state == r.state {
                continue;
            }
            r.state = state;
            changed = true;
            let (outcome, finished) = (r.outcome(), r.finished());
            if self.history(&key.0, &key.1).last().is_some_and(|l| l.outcome != outcome) {
                self.end_record(&key, &outcome);
            }
            if finished {
                self.dep.following.remove(&key);
            }
        }
        changed
    }

    /// Sets the outcome of a service's last deploy.
    fn end_record(&mut self, key: &Key, outcome: &str) {
        if let Some(last) = self.dep.history.get_mut(key).and_then(|h| h.last_mut()) {
            last.outcome = outcome.to_string();
        }
    }

    /// Writes the history to the state folder, off the UI thread.
    fn save_history(&mut self, cx: &mut Context<Self>) {
        let Some((store, workspace)) = self.dep.store.clone() else { return };
        // One save at a time, in order: a later one waits for the one running.
        if self.dep.saving.is_some() {
            self.dep.save_again = true;
            return;
        }
        let mut deploys = Deploys { version: 1, envs: BTreeMap::new() };
        for ((env, name), records) in &self.dep.history {
            let kept = records.iter().rev().take(HISTORY_KEPT).rev().cloned().collect();
            deploys.envs.entry(env.clone()).or_default().insert(name.clone(), kept);
        }
        let task = cx.background_executor().spawn(async move { store.save_deploys(&workspace, &deploys) });
        self.dep.saving = Some(cx.spawn(async move |this, cx| {
            if let Err(e) = task.await {
                log::warn!("deploy history not saved: {e}");
            }
            let _ = this.update(cx, |c, cx| {
                c.dep.saving = None;
                if std::mem::take(&mut c.dep.save_again) {
                    c.save_history(cx);
                }
            });
        }));
    }

    /// Forgets what was seen of `env` (its target changed): the next list is a new baseline.
    pub(crate) fn forget_versions(&mut self, env: Option<&str>) {
        self.dep.versions.retain(|(e, _), _| env.is_some_and(|env| env != e));
    }

    /// Who wants a service's pods while its deploy is followed (tests).
    pub fn following(&self) -> usize {
        self.dep.following.len()
    }
}
