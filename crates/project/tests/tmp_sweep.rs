//! H-17 item 6: `TempDir`'s sweep of `vox-project-*` leftovers from killed test runs (a `TempDir`
//! whose process was `SIGKILL`ed, e.g. `crash.rs`'s crash tests, never runs `Drop`).

mod common;

use std::path::Path;

#[test]
fn sweep_removes_dead_pid_leftovers_but_keeps_live_and_unrelated_dirs() {
    let base = std::env::temp_dir().join(format!("vox-project-sweep-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();

    // A pid this improbable is never actually running (Linux's default pid_max is 32 768, and
    // never goes anywhere near this).
    let dead_pid = 999_999u32;
    assert!(
        !Path::new(&format!("/proc/{dead_pid}")).exists(),
        "test needs a pid that's actually dead"
    );
    let dead_dir = base.join(format!("vox-project-crashtest-{dead_pid}-0"));
    std::fs::create_dir_all(&dead_dir).unwrap();

    // A tag with dashes in it: the pid must still be read off as the field before the trailing
    // counter, not confused by these.
    let dead_dashed = base.join(format!("vox-project-big-ac5-{dead_pid}-3"));
    std::fs::create_dir_all(&dead_dashed).unwrap();

    let live_dir = base.join(format!("vox-project-crashtest-{}-0", std::process::id()));
    std::fs::create_dir_all(&live_dir).unwrap();

    let unrelated = base.join("not-ours");
    std::fs::create_dir_all(&unrelated).unwrap();

    common::sweep_finished_runs(&base);

    assert!(!dead_dir.exists(), "a dead pid's leftover should be swept");
    assert!(
        !dead_dashed.exists(),
        "a dashed tag's dead-pid leftover should be swept too"
    );
    assert!(
        live_dir.exists(),
        "the current process's own dir must survive"
    );
    assert!(unrelated.exists(), "non-matching dirs are left alone");

    let _ = std::fs::remove_dir_all(&base);
}
