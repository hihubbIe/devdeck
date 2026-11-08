//! Files dropped from outside (a file manager) onto a pane (R12.2): each opens in a file
//! view there, in the pane or at its edge like a list row (R3.4); one already open moves
//! there. Several files open together: on an edge, the first one splits and the others
//! join its new pane. A folder or a path that isn't UTF-8 opens nothing, and a toast
//! names it.

use crate::deck::Deck;
use crate::sidebar;
use devdeck_core::layout::{Drop, Layout, Result as LayoutResult, ViewRef};
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::{ToastKind, toasts};
use gpui::{App, Entity};
use std::path::PathBuf;

/// The views `paths` open, and what was refused, named, with why.
pub fn sort(paths: &[PathBuf]) -> (Vec<ViewRef>, Vec<String>) {
    let (mut open, mut refused) = (Vec::new(), Vec::new());
    for p in paths {
        let name = p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned());
        match p.to_str() {
            None => refused.push(format!("{name} (its path isn't UTF-8)")),
            Some(_) if p.is_dir() => refused.push(format!("{name} (a folder)")),
            Some(s) => open.push(ViewAddr::File { path: s.to_string() }.to_ref()),
        }
    }
    (open, refused)
}

/// Opens `views` at `drop`: the first there, the others with it.
pub fn place(l: &mut Layout, views: &[ViewRef], drop: Drop) -> LayoutResult<()> {
    let Some((first, rest)) = views.split_first() else { return Ok(()) };
    sidebar::open_at(l, first.clone(), drop)?;
    for v in rest {
        let Some((pane, _)) = l.find_view(first) else { break };
        sidebar::open_at(l, v.clone(), Drop::Into(pane))?;
    }
    if let Some((pane, i)) = l.find_view(first) {
        l.activate(pane, i)?;
        l.focus(pane)?;
    }
    Ok(())
}

/// Opens dropped `paths` in `deck` at `drop` (R12.2).
pub fn open_files(deck: &Entity<Deck>, paths: &[PathBuf], drop: Drop, cx: &mut App) {
    if !deck.read(cx).has_services() {
        toasts(cx).update(cx, |t, cx| t.push(ToastKind::Info, "Files open in a workspace.", vec![], cx));
        return;
    }
    let (views, refused) = sort(paths);
    if !views.is_empty() {
        deck.update(cx, |d, cx| d.apply(cx, |l| place(l, &views, drop)));
    }
    if !refused.is_empty() {
        let text = format!("Not opened: {}.", refused.join(", "));
        toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, text, vec![], cx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devdeck_core::layout::Edge;

    #[test]
    fn folders_and_odd_paths_are_refused() {
        let dir = devdeck_core::testing::TempDir::new("drops");
        std::fs::write(dir.join("a.rs"), "").unwrap();
        std::fs::create_dir(dir.join("src")).unwrap();
        let (open, refused) = sort(&[dir.join("a.rs"), dir.join("src")]);
        assert_eq!(open, vec![ViewAddr::File { path: dir.join("a.rs").to_str().unwrap().into() }.to_ref()]);
        assert_eq!(refused, vec!["src (a folder)".to_string()]);
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let odd = dir.join(std::ffi::OsStr::from_bytes(b"caf\xe9.txt"));
            let (open, refused) = sort(&[odd]);
            assert!(open.is_empty());
            assert!(refused[0].ends_with("(its path isn't UTF-8)"), "{refused:?}");
        }
    }

    #[test]
    fn several_files_on_an_edge_make_one_split() {
        let mut l = crate::initial_layout();
        let p = l.main().focused;
        let panes = l.panes.len();
        let views: Vec<ViewRef> = ["/a.rs", "/b.rs"].iter().map(|f| ViewAddr::File { path: f.to_string() }.to_ref()).collect();
        place(&mut l, &views, Drop::Edge(p, Edge::Right)).unwrap();
        assert_eq!(l.panes.len(), panes + 1, "one new pane");
        let (a, _) = l.find_view(&views[0]).unwrap();
        let (b, _) = l.find_view(&views[1]).unwrap();
        assert_eq!(a, b, "both in it");
        assert_eq!(l.main().focused, a);
    }
}
