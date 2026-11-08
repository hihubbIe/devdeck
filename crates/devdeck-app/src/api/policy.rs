//! What a caller may do (R13.5): per class for a terminal, a short list for outside.
//! Outside never gets more than a terminal (an agent leaving its terminal's process tree
//! must not gain anything); a test checks it for every method.

use super::Who;
use devdeck_api::types::ClassPolicy;
use devdeck_api::{Class, MethodInfo, Policy};

/// What an outside caller may call until the user raises it (a card, later).
pub const OUTSIDE_METHODS: [&str; 4] = ["api.describe", "devdeck.info", "session.hello", "session.whoami"];

/// A terminal's defaults (`docs/plans/control-api.md`, Defaults). Acting on what the
/// user or another caller started asks; that check comes with the methods that act.
pub fn terminal(class: Class) -> Policy {
    match class {
        Class::Observe | Class::Read | Class::Present | Class::Organize | Class::Run => Policy::Allow,
        Class::Type | Class::Remote | Class::Destructive => Policy::Ask,
        Class::Unknown => Policy::Deny,
    }
}

/// The policy for calling `m` (its base class; a target that raises it is checked by
/// the method).
pub fn of(who: &Who, m: &MethodInfo) -> Policy {
    match who {
        Who::Terminal { .. } => terminal(m.class),
        Who::Outside { .. } if OUTSIDE_METHODS.contains(&m.name) => Policy::Allow,
        Who::Outside { .. } => Policy::Deny,
    }
}

/// Per class, as `session.hello` reports it (outside: everything denied but its list).
pub fn per_class(who: &Who) -> Vec<ClassPolicy> {
    Class::ALL
        .iter()
        .map(|&class| ClassPolicy {
            class,
            policy: match who {
                Who::Terminal { .. } => terminal(class),
                Who::Outside { .. } => Policy::Deny,
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rank(p: Policy) -> u8 {
        match p {
            Policy::Allow => 2,
            Policy::Ask => 1,
            Policy::Deny | Policy::Unknown => 0,
        }
    }

    #[test]
    fn outside_never_gets_more_than_a_terminal() {
        let t = Who::Terminal { workspace: "w".into(), n: 1, uid: "u".into(), shell: 1, starts: 1 };
        let o = Who::Outside { pid: Some(9), program: "x".into() };
        for m in devdeck_api::catalog() {
            assert!(rank(of(&o, &m)) <= rank(of(&t, &m)), "{}", m.name);
        }
        for name in OUTSIDE_METHODS {
            assert!(devdeck_api::catalog().iter().any(|m| m.name == name), "{name} exists");
        }
    }

    #[test]
    fn the_permission_matrix() {
        // Every method of the catalog against each kind of caller.
        let t = Who::Terminal { workspace: "w".into(), n: 1, uid: "u".into(), shell: 1, starts: 1 };
        let o = Who::Outside { pid: None, program: "?".into() };
        for m in devdeck_api::catalog() {
            assert_eq!(of(&t, &m), terminal(m.class), "{}", m.name);
            let want = if OUTSIDE_METHODS.contains(&m.name) { Policy::Allow } else { Policy::Deny };
            assert_eq!(of(&o, &m), want, "{}", m.name);
        }
        assert_eq!(terminal(Class::Destructive), Policy::Ask);
        assert_eq!(terminal(Class::Unknown), Policy::Deny);
    }
}
