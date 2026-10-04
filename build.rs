//! Records which commit a binary was built from, for `--version`.
//!
//! The point is a practical one: an installed copy is a snapshot, and when a fix
//! appears not to have taken, the first question is whether the binary on the
//! path is the build you think it is. A version number alone cannot answer that;
//! a commit and a dirty marker can.
//!
//! No `rerun-if-changed` is printed on purpose. Saying nothing leaves cargo's
//! default in place, which is to run this again whenever any file in the package
//! changes, and that is what keeps the dirty marker honest when source is edited.
//! The trade is that a commit touching no source file may leave the recorded
//! commit one behind until the next build; a wrong label on identical code is the
//! lesser of the two problems.

use std::process::Command;

fn main() {
    let commit = git(&["rev-parse", "--short=9", "HEAD"]);
    let dirty = git(&["status", "--porcelain"]).is_some_and(|out| !out.is_empty());

    let describe = match (commit, dirty) {
        // Not a git checkout: a release tarball, or a copy without history.
        (None, _) => "unknown".to_string(),
        (Some(commit), false) => commit,
        (Some(commit), true) => format!("{commit}-dirty"),
    };
    println!("cargo::rustc-env=DRAFT_TRACKER_COMMIT={describe}");
}

/// Run a git command, returning its trimmed output, or `None` if git is absent,
/// fails, or this is not a repository.
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_string())
}
