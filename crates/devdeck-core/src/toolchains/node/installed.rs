//! What a project's Node install provides: the programs in the `node_modules/.bin` of
//! its folder or a folder above it, where npm, pnpm and yarn look first when they run a
//! script. Each folder is read once per discovery pass.

use super::super::Cx;
use std::collections::BTreeSet;
use std::path::Path;

/// The programs in one folder's `node_modules/.bin` (none without one).
struct Bin(BTreeSet<String>);

/// Whether `program` is installed for a project at `dir`.
pub(super) fn has(dir: &Path, program: &str, cx: &mut Cx) -> bool {
    dir.ancestors().any(|d| {
        let bin = cx.memo(d, || {
            let entries = std::fs::read_dir(d.join("node_modules/.bin")).into_iter().flatten().flatten();
            Bin(entries.filter_map(|e| e.file_name().into_string().ok()).collect())
        });
        bin.0.contains(program)
    })
}
