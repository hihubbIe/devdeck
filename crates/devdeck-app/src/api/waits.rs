//! Waits and jobs (R13.10): `run.wait` and `terminal.wait` look every [`TICK`] while a
//! caller waits, and nothing runs otherwise (R1.3). A wait whose time is up answers with
//! what it has and a job: the same wait, resumed by `job.wait` from where it was (lines
//! already looked at aren't looked at again). A job is a caller's own, kept
//! [`JOB_KEPT`] after it began, [`JOBS_PER_CALLER`] at most. A caller whose shell ends
//! gets `expired`, and a client that hangs up stops its wait: both are the connection's
//! ([`super::answer`]).

use super::Who;
use super::dispatch::CallCx;
use super::handlers::{expired, output_of, run_state, with_run, with_terminal, workspace};
use crate::runner::RunKey;
use devdeck_api::types::{Cancelled, Job, JobParams, JobWaitParams, Jobs, NoParams, RunWaitParams, TerminalWaitParams, Until, Waited};
use devdeck_api::{ApiError, ErrorKind};
use devdeck_core::terminal::Terminal as CoreTerminal;
use gpui::{App, Global, Task};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// How often a wait looks.
pub const TICK: Duration = Duration::from_millis(100);
/// A wait's time when none is given, and at most (MCP clients wait 60 s for an answer).
pub const DEFAULT_WAIT: u32 = 30;
pub const MAX_WAIT: u32 = 50;
/// Waits one caller may have at once.
pub const WAITS_PER_CALLER: usize = 4;
pub const JOBS_PER_CALLER: usize = 16;
pub const JOB_KEPT: Duration = Duration::from_secs(600);

#[derive(Clone, Debug)]
enum Target {
    Run { ws: Option<String>, app: String, cmd: String },
    Terminal { ws: Option<String>, n: u32 },
}

/// A wait, as it resumes.
#[derive(Clone, Debug)]
struct Spec {
    target: Target,
    until: Until,
    re: Option<regex::Regex>,
    /// Where its output began, and the start of the run then.
    began: String,
    starts: u32,
    /// Lines up to here were looked at.
    scan: String,
}

impl Spec {
    fn what(&self) -> String {
        let target = match &self.target {
            Target::Run { app, cmd, .. } => format!("run {app} {cmd}"),
            Target::Terminal { n, .. } => format!("terminal {n}"),
        };
        let until = match &self.until {
            Until::Exit => "exit".to_string(),
            Until::Idle => "idle".to_string(),
            Until::Output(p) => format!("output /{p}/"),
        };
        format!("{target} until {until}")
    }
}

struct JobEntry {
    id: String,
    owner: String,
    spec: Spec,
    began: Instant,
    done: bool,
}

/// Every caller's jobs, and how many waits each has running.
#[derive(Default)]
struct JobBook {
    jobs: Vec<JobEntry>,
    waiting: Rc<RefCell<HashMap<String, usize>>>,
    next: u64,
}

impl Global for JobBook {}

/// A wait in progress, counted for its caller while it lives.
struct Waiting(Rc<RefCell<HashMap<String, usize>>>, String);

impl Drop for Waiting {
    fn drop(&mut self) {
        let mut w = self.0.borrow_mut();
        if let Some(n) = w.get_mut(&self.1) {
            *n -= 1;
            if *n == 0 {
                w.remove(&self.1);
            }
        }
    }
}

/// A caller's jobs belong to its shell instance: a new shell there isn't it.
fn owner(who: &Who) -> String {
    format!("{who:?}")
}

fn pattern(until: &Until) -> Result<Option<regex::Regex>, ApiError> {
    let Until::Output(p) = until else { return Ok(None) };
    regex::RegexBuilder::new(p).size_limit(1 << 20).dfa_size_limit(1 << 20).build().map(Some).map_err(|e| ApiError::invalid(format!("pattern: {e}")))
}

/// The terminal of `target`, its start count and what names it, the caller's rules
/// applied (a run: any; a terminal: its own).
fn with_target<T>(cx: &CallCx, target: &Target, app: &App, f: impl FnOnce(&CoreTerminal, u32, &str) -> Result<T, ApiError>) -> Result<T, ApiError> {
    match target {
        Target::Run { ws, app: a, cmd } => with_run(cx, ws.as_deref(), a, cmd, app, f),
        Target::Terminal { ws, n } => with_terminal(cx, ws.as_deref(), *n, app, f),
    }
}

fn key(target: &Target) -> RunKey {
    match target {
        Target::Run { app, cmd, .. } => RunKey::new(app, cmd),
        Target::Terminal { n, .. } => RunKey::terminal(*n),
    }
}

fn ws(target: &Target) -> Option<&str> {
    match target {
        Target::Run { ws, .. } | Target::Terminal { ws, .. } => ws.as_deref(),
    }
}

/// A new wait's spec: its output begins now.
fn begin(cx: &CallCx, target: Target, until: Until, app: &App) -> Result<Spec, ApiError> {
    if matches!((&target, &until), (Target::Run { .. }, Until::Idle)) {
        return Err(ApiError::invalid("until: a run has no prompt to be idle at (exit or output)"));
    }
    let re = pattern(&until)?;
    let (began, starts) = with_target(cx, &target, app, |t, starts, name| Ok((output_of(cx, name, t, starts, Some(0), None)?.next, starts)))?;
    Ok(Spec { target, until, re, scan: began.clone(), began, starts })
}

/// Looks once: whether what's waited for happened, and the state and output now.
fn look(cx: &CallCx, spec: &mut Spec, app: &App) -> Result<(bool, Waited), ApiError> {
    let re = spec.re.clone();
    let (began, mut scan) = (spec.began.clone(), spec.scan.clone());
    let (matched, output, starts_now) = with_target(cx, &spec.target, app, |t, starts, name| {
        let mut matched = false;
        if let Some(re) = &re {
            loop {
                let read = if scan.is_empty() { Err(expired()) } else { output_of(cx, name, t, starts, None, Some(&scan)) };
                match read {
                    Ok(out) => {
                        matched |= out.lines.iter().any(|l| re.is_match(l.as_str()));
                        scan = out.next.clone();
                        if matched || !out.truncated {
                            break;
                        }
                    }
                    // A restart, a resize, a full-screen program: on from what shows now.
                    Err(e) if e.kind == ErrorKind::Expired => {
                        let now = output_of(cx, name, t, starts, None, None)?;
                        matched |= now.lines.iter().any(|l| re.is_match(l.as_str()));
                        scan = now.next;
                        break;
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        let output = match output_of(cx, name, t, starts, None, (!began.is_empty()).then_some(began.as_str())) {
            Err(e) if e.kind == ErrorKind::Expired => output_of(cx, name, t, starts, None, None)?,
            other => other?,
        };
        Ok((matched, output, starts))
    })?;
    spec.scan = scan;
    let w = workspace(cx, ws(&spec.target), app)?;
    let runner = w.services.read(app).runner.read(app);
    let k = key(&spec.target);
    let run = runner.run(&k).ok_or_else(|| ApiError::not_found("it went away"))?;
    let (state, exit_code, _) = run_state(run.state);
    let ended = !run.state.is_running() || starts_now != spec.starts;
    let done = match spec.until {
        Until::Exit => ended,
        Until::Idle => ended || runner.terminal_program(&k).is_none(),
        Until::Output(_) => matched,
    };
    Ok((done, Waited { done, job: None, state, exit_code, output }))
}

fn seconds(timeout: Option<u32>) -> Duration {
    Duration::from_secs(timeout.unwrap_or(DEFAULT_WAIT).min(MAX_WAIT) as u64)
}

/// Waits on `spec` for at most `timeout`; `job` when it resumes one.
fn wait(cx: &CallCx, spec: Spec, job: Option<String>, timeout: Option<u32>, app: &mut App) -> Result<Task<Result<Waited, ApiError>>, ApiError> {
    let who = cx.who.clone();
    let me = owner(&who);
    let counter = app.default_global::<JobBook>().waiting.clone();
    {
        let mut w = counter.borrow_mut();
        let n = w.entry(me.clone()).or_default();
        if *n >= WAITS_PER_CALLER {
            return Err(ApiError::new(ErrorKind::Busy, format!("{WAITS_PER_CALLER} waits at once at most")));
        }
        *n += 1;
    }
    let waiting = Waiting(counter, me.clone());
    let (instance, state) = (cx.instance.to_string(), cx.state);
    let deadline = app.background_executor().now() + seconds(timeout);
    Ok(app.spawn(async move |cx| {
        let _waiting = waiting;
        let mut spec = spec;
        loop {
            let looked = cx.update(|app| look(&CallCx { who: &who, instance: &instance, state, conn: None }, &mut spec, app));
            let (done, mut waited) = match looked {
                Ok(l) => l,
                Err(e) => {
                    cx.update(|app| forget(app, job.as_deref()));
                    return Err(e);
                }
            };
            let up = cx.background_executor().now() >= deadline;
            if done || up {
                waited.job = cx.update(|app| keep(app, &me, job.clone(), spec, done));
                return Ok(waited);
            }
            cx.background_executor().timer(TICK).await;
        }
    }))
}

/// Keeps a wait as a job (or updates the one it resumed); its id while not done.
fn keep(app: &mut App, owner: &str, job: Option<String>, spec: Spec, done: bool) -> Option<String> {
    let now = app.background_executor().now();
    let book = app.default_global::<JobBook>();
    book.jobs.retain(|j| now.duration_since(j.began) < JOB_KEPT);
    if let Some(id) = &job
        && let Some(j) = book.jobs.iter_mut().find(|j| &j.id == id)
    {
        j.spec = spec;
        j.done = done;
        return (!done).then(|| id.clone());
    }
    if done {
        return None;
    }
    // The oldest go first past the cap.
    while book.jobs.iter().filter(|j| j.owner == owner).count() >= JOBS_PER_CALLER {
        let oldest = book.jobs.iter().position(|j| j.owner == owner).expect("counted");
        book.jobs.remove(oldest);
    }
    book.next += 1;
    let id = format!("wait-{}", book.next);
    book.jobs.push(JobEntry { id: id.clone(), owner: owner.to_string(), spec, began: now, done });
    Some(id)
}

fn forget(app: &mut App, job: Option<&str>) {
    if let Some(id) = job {
        app.default_global::<JobBook>().jobs.retain(|j| j.id != id);
    }
}

/// The caller's job `id`'s spec.
fn job_spec(cx: &CallCx, id: &str, app: &mut App) -> Result<Spec, ApiError> {
    let me = owner(cx.who);
    let now = app.background_executor().now();
    let book = app.default_global::<JobBook>();
    book.jobs.retain(|j| now.duration_since(j.began) < JOB_KEPT);
    book.jobs.iter().find(|j| j.id == id && j.owner == me).map(|j| j.spec.clone()).ok_or_else(|| ApiError::not_found(format!("no job {id} of yours")))
}

pub fn run_wait(cx: &CallCx, p: RunWaitParams, app: &mut App) -> Result<Task<Result<Waited, ApiError>>, ApiError> {
    let spec = begin(cx, Target::Run { ws: p.workspace, app: p.app, cmd: p.cmd }, p.until, app)?;
    wait(cx, spec, None, p.timeout, app)
}

pub fn terminal_wait(cx: &CallCx, p: TerminalWaitParams, app: &mut App) -> Result<Task<Result<Waited, ApiError>>, ApiError> {
    let spec = begin(cx, Target::Terminal { ws: p.workspace, n: p.n }, p.until, app)?;
    wait(cx, spec, None, p.timeout, app)
}

pub fn job_wait(cx: &CallCx, p: JobWaitParams, app: &mut App) -> Result<Task<Result<Waited, ApiError>>, ApiError> {
    let spec = job_spec(cx, &p.job, app)?;
    wait(cx, spec, Some(p.job), p.timeout, app)
}

pub fn job_get(cx: &CallCx, p: JobParams, app: &mut App) -> Result<Waited, ApiError> {
    let mut spec = job_spec(cx, &p.job, app)?;
    let (done, mut waited) = look(cx, &mut spec, app)?;
    let me = owner(cx.who);
    waited.job = keep(app, &me, Some(p.job), spec, done);
    Ok(waited)
}

pub fn job_cancel(cx: &CallCx, p: JobParams, app: &mut App) -> Result<Cancelled, ApiError> {
    job_spec(cx, &p.job, app)?;
    forget(app, Some(&p.job));
    Ok(Cancelled { job: p.job })
}

pub fn job_list(cx: &CallCx, _: NoParams, app: &mut App) -> Result<Jobs, ApiError> {
    let me = owner(cx.who);
    let now = app.background_executor().now();
    let book = app.default_global::<JobBook>();
    book.jobs.retain(|j| now.duration_since(j.began) < JOB_KEPT);
    let jobs = book
        .jobs
        .iter()
        .filter(|j| j.owner == me)
        .map(|j| Job { id: j.id.clone(), what: j.spec.what().into(), done: j.done, age: now.duration_since(j.began).as_secs() })
        .collect();
    Ok(Jobs { jobs })
}
