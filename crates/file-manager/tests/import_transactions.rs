//! Import behaviour against a real filesystem.
//!
//! The planning rules are unit-tested next to the code. These are the parts
//! that only a real directory can answer: that a rollback removes what the
//! import created and nothing else, that a cancelled copy leaves no wreckage,
//! that dotfiles and empty folders and binary content survive the round trip,
//! and that a symlinked cycle does not spin forever.
//!
//! Failures are injected rather than provoked. Waiting for a permission error
//! to happen on its own gives a test that passes on one machine and hangs on
//! another; a cancellation callback that returns true on the third file fails
//! in exactly the same place every time.

// The same allowance the other integration test carries: the workspace denies
// these because a panic in the running service takes projects down with it,
// and a test that cannot assert is not a test. Scoped to this crate, not
// widened anywhere else.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use project_host_file_manager::operations::{
    import_local_paths, import_local_sources, plan_import_destinations, ImportSource,
};
use project_host_file_manager::{FileError, FileLimits};

struct Project {
    _dir: tempfile::TempDir,
    root: PathBuf,
    outside: PathBuf,
}

/// A project with a `src/app.ts`, an `index.js` and a README already in it.
fn project() -> Project {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path().join("project");
    let outside = dir.path().join("elsewhere");
    std::fs::create_dir_all(root.join("src")).expect("create");
    std::fs::create_dir_all(&outside).expect("create");
    std::fs::write(root.join("index.js"), "original index\n").expect("write");
    std::fs::write(root.join("src/app.ts"), "original app\n").expect("write");
    std::fs::write(root.join("README.md"), "# original\n").expect("write");
    Project {
        _dir: dir,
        root,
        outside,
    }
}

fn limits() -> FileLimits {
    FileLimits::default()
}

fn write(path: &Path, contents: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("dirs");
    }
    std::fs::write(path, contents).expect("write");
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("read")
}

/// Nothing this crate creates while staging may survive a finished import.
fn assert_no_staging(root: &Path) {
    for entry in std::fs::read_dir(root).expect("read dir").flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(
            !name.starts_with(".project-host-import-"),
            "staging directory left behind: {name}",
        );
    }
}

#[test]
fn a_project_folder_lands_as_its_contents_and_leaves_no_staging() {
    let p = project();
    let source = p.outside.join("MyProject");
    write(&source.join("package.json"), b"{}");
    write(&source.join("src/deep/index.ts"), b"deep");
    write(&source.join(".env"), b"TOKEN=1");
    std::fs::create_dir_all(source.join("empty")).expect("dirs");

    import_local_sources(
        &p.root,
        "",
        &[ImportSource::unwrapped(&source)],
        "tx-1",
        &limits(),
        |_| {},
        || false,
    )
    .expect("import");

    assert!(p.root.join("package.json").is_file());
    assert!(p.root.join(".env").is_file(), "dotfiles survive");
    assert!(p.root.join("empty").is_dir(), "empty folders survive");
    assert!(p.root.join("src/deep/index.ts").is_file());
    assert!(!p.root.join("MyProject").exists(), "no wrapper folder");
    assert_no_staging(&p.root);
}

#[test]
fn a_normal_folder_keeps_its_wrapper() {
    let p = project();
    let source = p.outside.join("Photos");
    write(&source.join("a.jpg"), b"\xff\xd8\xff\xe0binary");

    import_local_paths(&p.root, "", &[&source], "tx-2", &limits(), |_| {}, || false)
        .expect("import");

    assert!(p.root.join("Photos/a.jpg").is_file());
    assert_eq!(
        std::fs::read(p.root.join("Photos/a.jpg")).expect("read"),
        b"\xff\xd8\xff\xe0binary",
        "binary content is preserved byte for byte",
    );
}

#[test]
fn merging_into_an_existing_folder_keeps_what_was_there() {
    let p = project();
    let source = p.outside.join("incoming");
    write(&source.join("src/added.ts"), b"added");

    import_local_sources(
        &p.root,
        "",
        &[ImportSource::unwrapped(&source)],
        "tx-3",
        &limits(),
        |_| {},
        || false,
    )
    .expect("import");

    assert!(p.root.join("src/added.ts").is_file(), "incoming file");
    assert_eq!(
        read(&p.root.join("src/app.ts")),
        "original app\n",
        "the file that was already there is untouched",
    );
}

#[test]
fn a_file_that_already_exists_stops_the_import_before_anything_is_copied() {
    let p = project();
    let source = p.outside.join("incoming");
    write(&source.join("index.js"), b"replacement");
    write(&source.join("brand-new.ts"), b"new");

    let error = import_local_sources(
        &p.root,
        "",
        &[ImportSource::unwrapped(&source)],
        "tx-4",
        &limits(),
        |_| {},
        || false,
    )
    .expect_err("must refuse");

    assert!(matches!(error, FileError::AlreadyExists(_)));
    assert_eq!(
        read(&p.root.join("index.js")),
        "original index\n",
        "the existing file is untouched",
    );
    assert!(
        !p.root.join("brand-new.ts").exists(),
        "nothing was copied: the refusal happens during planning",
    );
    assert_no_staging(&p.root);
}

#[test]
fn a_directory_arriving_where_a_file_sits_is_refused() {
    let p = project();
    let source = p.outside.join("incoming");
    // `index.js` is a file in the project; here it is a folder.
    write(&source.join("index.js/inner.txt"), b"x");

    let error = import_local_sources(
        &p.root,
        "",
        &[ImportSource::unwrapped(&source)],
        "tx-5",
        &limits(),
        |_| {},
        || false,
    )
    .expect_err("must refuse");

    assert!(matches!(error, FileError::AlreadyExists(_)));
    assert!(p.root.join("index.js").is_file(), "still a file");
}

#[test]
fn a_file_arriving_where_a_directory_sits_is_refused() {
    let p = project();
    let source = p.outside.join("incoming");
    // `src` is a directory in the project; here it is a file.
    write(&source.join("src"), b"not a folder");

    let error = import_local_sources(
        &p.root,
        "",
        &[ImportSource::unwrapped(&source)],
        "tx-6",
        &limits(),
        |_| {},
        || false,
    )
    .expect_err("must refuse");

    assert!(matches!(error, FileError::AlreadyExists(_)));
    assert!(p.root.join("src").is_dir(), "still a directory");
    assert!(p.root.join("src/app.ts").is_file(), "and still populated");
}

#[test]
fn two_incoming_items_wanting_one_destination_are_refused() {
    let p = project();
    let one = p.outside.join("one/notes.txt");
    let two = p.outside.join("two/notes.txt");
    write(&one, b"first");
    write(&two, b"second");

    let error = import_local_paths(
        &p.root,
        "",
        &[&one, &two],
        "tx-7",
        &limits(),
        |_| {},
        || false,
    )
    .expect_err("must refuse");

    assert!(matches!(error, FileError::AlreadyExists(_)));
    assert!(!p.root.join("notes.txt").exists(), "neither was written");
}

#[test]
fn cancelling_before_the_copy_starts_leaves_the_project_alone() {
    let p = project();
    let source = p.outside.join("incoming");
    write(&source.join("a.txt"), b"a");

    let error = import_local_sources(
        &p.root,
        "",
        &[ImportSource::unwrapped(&source)],
        "tx-8",
        &limits(),
        |_| {},
        // Cancelled from the very first check.
        || true,
    )
    .expect_err("must stop");

    assert!(matches!(error, FileError::Refused("import cancelled")));
    assert!(!p.root.join("a.txt").exists());
    assert_no_staging(&p.root);
}

#[test]
fn cancelling_partway_through_copies_nothing_into_the_project() {
    let p = project();
    let source = p.outside.join("incoming");
    for index in 0..6 {
        write(&source.join(format!("file{index}.txt")), b"contents");
    }

    // Injected rather than provoked: the third check gives up, in the same
    // place on every machine and every run.
    let checks = AtomicUsize::new(0);
    let error = import_local_sources(
        &p.root,
        "",
        &[ImportSource::unwrapped(&source)],
        "tx-9",
        &limits(),
        |_| {},
        || checks.fetch_add(1, Ordering::SeqCst) >= 3,
    )
    .expect_err("must stop");

    assert!(matches!(error, FileError::Refused("import cancelled")));
    for index in 0..6 {
        assert!(
            !p.root.join(format!("file{index}.txt")).exists(),
            "a cancelled import commits nothing",
        );
    }
    assert_no_staging(&p.root);
    // The files that were there before are all still there.
    assert_eq!(read(&p.root.join("index.js")), "original index\n");
    assert_eq!(read(&p.root.join("src/app.ts")), "original app\n");
}

#[test]
fn a_cancelled_merge_leaves_the_existing_folder_exactly_as_it_was() {
    let p = project();
    let source = p.outside.join("incoming");
    for index in 0..6 {
        write(&source.join(format!("src/added{index}.ts")), b"added");
    }

    let checks = AtomicUsize::new(0);
    let _ = import_local_sources(
        &p.root,
        "",
        &[ImportSource::unwrapped(&source)],
        "tx-10",
        &limits(),
        |_| {},
        || checks.fetch_add(1, Ordering::SeqCst) >= 4,
    );

    assert!(p.root.join("src").is_dir(), "the folder still exists");
    assert_eq!(
        read(&p.root.join("src/app.ts")),
        "original app\n",
        "and its contents are untouched",
    );
    for index in 0..6 {
        assert!(!p.root.join(format!("src/added{index}.ts")).exists());
    }
}

#[test]
fn progress_is_reported_and_never_goes_backwards() {
    let p = project();
    let source = p.outside.join("incoming");
    for index in 0..5 {
        write(&source.join(format!("f{index}.bin")), &[b'x'; 512]);
    }

    let mut seen = Vec::new();
    import_local_sources(
        &p.root,
        "",
        &[ImportSource::unwrapped(&source)],
        "tx-11",
        &limits(),
        |event| seen.push((event.copied_files, event.copied_bytes, event.total_bytes)),
        || false,
    )
    .expect("import");

    assert!(!seen.is_empty(), "progress is reported at all");
    let last = seen.last().expect("last");
    assert_eq!(last.0, 5, "every file counted");
    assert_eq!(last.1, 5 * 512, "every byte counted");
    assert_eq!(last.2, 5 * 512, "the total was known in advance");

    // Zipped rather than indexed: the slice access is what clippy objects to,
    // and pairing the sequence with itself offset by one says the same thing.
    for (before, after) in seen.iter().zip(seen.iter().skip(1)) {
        assert!(
            after.0 >= before.0 && after.1 >= before.1,
            "progress must be monotonic: {before:?} then {after:?}",
        );
    }
}

#[test]
fn a_file_larger_than_the_limit_is_refused_before_anything_is_copied() {
    let p = project();
    let source = p.outside.join("incoming");
    write(&source.join("small.txt"), b"ok");
    write(&source.join("big.bin"), &vec![0u8; 4096]);

    let tight = FileLimits {
        max_upload_bytes: 1024,
        ..FileLimits::default()
    };

    let error = import_local_sources(
        &p.root,
        "",
        &[ImportSource::unwrapped(&source)],
        "tx-12",
        &tight,
        |_| {},
        || false,
    )
    .expect_err("must refuse");

    assert!(matches!(error, FileError::TooLarge { .. }));
    assert!(
        !p.root.join("small.txt").exists(),
        "the refusal happens while planning, so nothing was copied",
    );
}

/// Create a directory link the way this platform actually makes them.
///
/// A junction is tried second on Windows because it needs no privilege, while
/// `symlink_dir` needs Developer Mode or elevation. It is also what `npm` and
/// `pnpm` write into `node_modules`, so it is the link a real import meets —
/// and without it this test silently skips on an ordinary Windows session and
/// reports a pass it did not earn.
#[cfg(windows)]
fn make_link(target: &Path, link: &Path) -> bool {
    if std::os::windows::fs::symlink_dir(target, link).is_ok() {
        return true;
    }
    // `cmd` reads a forward slash as the start of a switch, so the paths have
    // to reach it in Windows form whatever they were built from.
    let windows_form = |path: &Path| path.to_string_lossy().replace('/', "\\");
    std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(windows_form(link))
        .arg(windows_form(target))
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

#[cfg(unix)]
fn make_link(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[test]
fn a_symlink_is_skipped_rather_than_followed() {
    let p = project();
    let source = p.outside.join("incoming");
    std::fs::create_dir_all(&source).expect("dirs");
    write(&source.join("real.txt"), b"real");

    // A link pointing at its own parent: following it would never terminate.
    let link = source.join("loop");
    if !make_link(&source, &link) {
        // Skipping is honest; asserting a pass we did not earn is not.
        eprintln!("skipped: this machine cannot create links of any kind");
        return;
    }

    // Reaching the assertions at all is the first half of what this tests:
    // walking the cycle would not terminate, so a test that returns has
    // already proved the link was not followed.
    let report = import_local_sources(
        &p.root,
        "",
        &[ImportSource::unwrapped(&source)],
        "tx-13",
        &limits(),
        |_| {},
        || false,
    )
    .expect("a cycle is skipped, not walked, and not fatal");

    // The link is not copied. A copy is not the place it points at, and on
    // Windows recreating one needs a privilege an ordinary session lacks.
    assert!(!p.root.join("loop").exists(), "the link must not be copied");
    assert_eq!(report.skipped_symlinks, 1);

    // And the file beside it still arrives. This is the half that used to be
    // lost: one link anywhere in a source refused the whole import, and every
    // Node project has them in `node_modules` — junctions on Windows, which
    // `is_symlink` also reports.
    assert_eq!(
        std::fs::read_to_string(p.root.join("real.txt")).expect("read"),
        "real"
    );
}

#[test]
fn importing_into_a_subdirectory_puts_everything_under_it() {
    let p = project();
    let source = p.outside.join("bundle");
    write(&source.join("a.txt"), b"a");
    write(&source.join("nested/b.txt"), b"b");

    import_local_sources(
        &p.root,
        "src",
        &[ImportSource::unwrapped(&source)],
        "tx-14",
        &limits(),
        |_| {},
        || false,
    )
    .expect("import");

    assert!(p.root.join("src/a.txt").is_file());
    assert!(p.root.join("src/nested/b.txt").is_file());
    assert!(!p.root.join("src/bundle").exists());
    assert_eq!(read(&p.root.join("src/app.ts")), "original app\n");
}

#[test]
fn a_name_differing_only_in_case_is_refused_on_a_case_insensitive_volume() {
    let p = project();
    let source = p.outside.join("incoming");
    write(&source.join("readme.md"), b"lower case");

    let result = import_local_sources(
        &p.root,
        "",
        &[ImportSource::unwrapped(&source)],
        "tx-15",
        &limits(),
        |_| {},
        || false,
    );

    // Windows and a default macOS volume treat this as the same file as the
    // project's `README.md`; Linux does not. Both answers are correct for the
    // filesystem underneath, and both must leave the original intact.
    match result {
        Err(FileError::AlreadyExists(_)) => {}
        Err(other) => panic!("unexpected error: {other:?}"),
        Ok(_) => {
            // Only a case-sensitive filesystem may reach here. Written as a
            // branch rather than an assertion on a compile-time constant,
            // which clippy reads as an assertion that cannot fail.
            #[cfg(not(target_os = "linux"))]
            panic!("a case-insensitive volume must have refused this");
        }
    }
    assert!(p.root.join("README.md").exists());
}

#[test]
fn planning_reports_where_an_unwrapped_folder_would_land_and_how_much_it_holds() {
    let p = project();
    let source = p.outside.join("MyProject");
    write(&source.join("package.json"), b"{}");
    write(&source.join("src/a.ts"), b"12345");
    write(&source.join("src/b.ts"), b"678");
    write(&source.join("README.md"), b"readme");

    let mut planned =
        plan_import_destinations(&p.root, "", &[ImportSource::unwrapped(&source)], &limits())
            .expect("plan");
    planned.sort_by(|a, b| a.relative.cmp(&b.relative));

    let names: Vec<&str> = planned
        .iter()
        .map(|entry| entry.relative.as_str())
        .collect();
    assert_eq!(names, vec!["README.md", "package.json", "src"]);

    // The folder rolls its whole subtree up into one entry, which is what a
    // progress bar and a conflict check both need.
    let src = planned
        .iter()
        .find(|entry| entry.relative == "src")
        .expect("src");
    assert!(src.is_directory);
    assert_eq!(src.total_files, 2);
    assert_eq!(src.total_bytes, 8);

    // Planning writes nothing.
    assert!(!p.root.join("package.json").exists());
    assert_no_staging(&p.root);
}

#[test]
fn planning_reports_collisions_rather_than_refusing_them() {
    let p = project();
    let source = p.outside.join("incoming");
    write(&source.join("index.js"), b"replacement");

    // `index.js` already exists in the project. An import would refuse; a plan
    // reports it so the window can ask what to do.
    let planned =
        plan_import_destinations(&p.root, "", &[ImportSource::unwrapped(&source)], &limits())
            .expect("plan must not refuse");

    assert_eq!(planned.len(), 1);
    let only = planned.first().expect("one destination");
    assert_eq!(only.relative, "index.js");
    assert_eq!(
        read(&p.root.join("index.js")),
        "original index
"
    );
}

#[test]
fn an_import_can_land_under_a_name_of_the_callers_choosing() {
    let p = project();
    let source = p.outside.join("elsewhere/index.js");
    write(&source, b"incoming");

    import_local_sources(
        &p.root,
        "",
        &[ImportSource::renamed(&source, "index copy.js")],
        "tx-16",
        &limits(),
        |_| {},
        || false,
    )
    .expect("import");

    assert_eq!(read(&p.root.join("index copy.js")), "incoming");
    assert_eq!(
        read(&p.root.join("index.js")),
        "original index
",
        "the file it would have collided with is untouched",
    );
}

#[test]
fn a_chosen_name_may_not_contain_a_separator() {
    let p = project();
    let source = p.outside.join("elsewhere/a.txt");
    write(&source, b"x");

    let error = import_local_sources(
        &p.root,
        "",
        &[ImportSource::renamed(&source, "nested/a.txt")],
        "tx-17",
        &limits(),
        |_| {},
        || false,
    )
    .expect_err("must refuse");

    assert!(matches!(error, FileError::Refused(_)));
}

// ------------------------------------------------- commit-time rollback
//
// Planning refuses a collision before anything is copied, so the only way to
// reach the commit's own rollback is for the destination to change *while the
// copy is running*. The progress callback is the injection point: creating the
// colliding file from inside it fails the commit in the same place every run,
// which is what makes the rollback path testable at all.

#[test]
fn a_destination_that_appears_mid_copy_fails_the_commit_and_rolls_back_what_was_added() {
    let p = project();
    let source = p.outside.join("incoming");
    write(&source.join("src/added.ts"), b"added");
    write(&source.join("zz-last.txt"), b"incoming");

    let root = p.root.clone();
    let planted = AtomicUsize::new(0);
    let error = import_local_sources(
        &p.root,
        "",
        &[ImportSource::unwrapped(&source)],
        "tx-rollback-1",
        &limits(),
        |_| {
            // Once, partway through the copy: put a file exactly where one of
            // the staged top-level destinations is about to land.
            if planted.fetch_add(1, Ordering::SeqCst) == 0 {
                std::fs::write(root.join("zz-last.txt"), "arrived first").expect("plant");
            }
        },
        || false,
    )
    .expect_err("the commit must refuse to overwrite");

    assert!(matches!(error, FileError::AlreadyExists(_)), "{error:?}");

    // The file that arrived first is the one that stays, byte for byte.
    assert_eq!(read(&p.root.join("zz-last.txt")), "arrived first");

    // Everything this import had already merged into place is gone again...
    assert!(
        !p.root.join("src/added.ts").exists(),
        "the rollback must remove the files this import added",
    );

    // ...and everything that was already in the project is untouched.
    assert_eq!(read(&p.root.join("src/app.ts")), "original app\n");
    assert_eq!(read(&p.root.join("index.js")), "original index\n");
    assert_eq!(read(&p.root.join("README.md")), "# original\n");
    assert!(
        p.root.join("src").is_dir(),
        "a folder that was already here is not removed by the rollback",
    );
    assert_no_staging(&p.root);
}

#[test]
fn a_failed_commit_leaves_no_half_merged_folder_behind() {
    let p = project();
    let source = p.outside.join("incoming");
    write(&source.join("fresh/one.txt"), b"one");
    write(&source.join("fresh/two.txt"), b"two");
    write(&source.join("zz-last.txt"), b"incoming");

    let root = p.root.clone();
    let planted = AtomicUsize::new(0);
    let error = import_local_sources(
        &p.root,
        "",
        &[ImportSource::unwrapped(&source)],
        "tx-rollback-2",
        &limits(),
        |_| {
            if planted.fetch_add(1, Ordering::SeqCst) == 0 {
                std::fs::write(root.join("zz-last.txt"), "arrived first").expect("plant");
            }
        },
        || false,
    )
    .expect_err("the commit must refuse");

    assert!(matches!(error, FileError::AlreadyExists(_)), "{error:?}");
    // `fresh/` did not exist before this import, so the rollback takes all of
    // it — not just the files, leaving an empty folder nobody asked for.
    assert!(
        !p.root.join("fresh").exists(),
        "a folder this import created is removed whole",
    );
    assert_no_staging(&p.root);
}

#[test]
fn a_rollback_does_not_touch_a_file_the_import_never_planned_to_write() {
    let p = project();
    write(&p.root.join("src/precious.ts"), b"do not touch");

    let source = p.outside.join("incoming");
    write(&source.join("src/added.ts"), b"added");
    write(&source.join("zz-last.txt"), b"incoming");

    let root = p.root.clone();
    let planted = AtomicUsize::new(0);
    let _ = import_local_sources(
        &p.root,
        "",
        &[ImportSource::unwrapped(&source)],
        "tx-rollback-3",
        &limits(),
        |_| {
            if planted.fetch_add(1, Ordering::SeqCst) == 0 {
                std::fs::write(root.join("zz-last.txt"), "arrived first").expect("plant");
            }
        },
        || false,
    )
    .expect_err("the commit must refuse");

    assert_eq!(
        read(&p.root.join("src/precious.ts")),
        "do not touch",
        "a pre-existing file that was never part of the plan is not considered by the rollback",
    );
}
