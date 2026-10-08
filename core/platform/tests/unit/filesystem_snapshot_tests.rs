use super::*;

#[test]
fn aliases_sample_once_and_preserve_device_identity() {
    let mut calls = 0;
    let sampled = collect_filesystem_usage(
        [
            PathBuf::from("first"),
            PathBuf::from("alias"),
            PathBuf::from("second"),
        ],
        |path| Some(if path == Path::new("second") { 2 } else { 1 }),
        |_| {
            calls += 1;
            Some((1000, 500))
        },
    );
    assert_eq!(calls, 2);
    assert_eq!(
        sampled.iter().map(|fs| fs.device_id).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(sampled[0].path, "first");
    assert_eq!(sampled[0].total_bytes, 1000);
    assert_eq!(sampled[0].free_bytes, 500);
}

#[test]
fn unavailable_path_is_not_zero_and_failed_usage_allows_another_alias() {
    let sampled = collect_filesystem_usage(
        ["missing", "failed", "alias", "other"].map(PathBuf::from),
        |path| {
            if path == Path::new("missing") {
                None
            } else {
                Some(1)
            }
        },
        |path| {
            if path == Path::new("failed") {
                None
            } else {
                Some((100, 25))
            }
        },
    );
    assert_eq!(sampled.len(), 1);
    assert_eq!(sampled[0].path, "alias");
    assert_eq!(sampled[0].free_bytes, 25);
    assert!(collect_filesystem_usage([PathBuf::from("missing")], |_| None, |_| None).is_empty());
}

#[test]
fn native_snapshot_deduplicates_working_paths_and_mount_aliases() {
    let cwd = std::env::current_dir().unwrap();
    let device = filesystem_device_id(&cwd).expect("working directory device");
    let sampled = filesystem_usage_snapshot(&[cwd.clone(), cwd.join(".")]);
    assert_eq!(
        sampled.iter().filter(|fs| fs.device_id == device).count(),
        1
    );
    let unique: std::collections::HashSet<_> = sampled.iter().map(|fs| fs.device_id).collect();
    assert_eq!(unique.len(), sampled.len());
    assert!(sampled
        .iter()
        .any(|fs| fs.device_id == device && fs.total_bytes > 0));
}

/// A real kernel mount/unmount regression, not a synthetic topology sample.
/// Uses only its own small image; it never formats or writes an existing disk.
#[cfg(target_os = "macos")]
#[test]
fn macos_snapshot_tracks_real_disk_image_mount_and_unmount() {
    use std::process::Command;
    struct Image {
        image: PathBuf,
        mount: PathBuf,
        attached: bool,
    }
    impl Image {
        fn detach(&mut self) {
            if self.attached {
                let status = command_status(
                    Command::new("/usr/bin/hdiutil")
                        .arg("detach")
                        .arg(&self.mount)
                        .arg("-quiet"),
                );
                assert!(
                    status.is_ok_and(|status| status.success()),
                    "test image detach failed: {}",
                    self.mount.display()
                );
                self.attached = false;
            }
        }
    }
    impl Drop for Image {
        fn drop(&mut self) {
            if self.attached {
                // Only this fixture's mount; no broad disk cleanup or force eject.
                if !command_status(
                    Command::new("/usr/bin/hdiutil")
                        .arg("detach")
                        .arg(&self.mount)
                        .arg("-quiet"),
                )
                .is_ok_and(|status| status.success())
                {
                    eprintln!("test image remains mounted at {}", self.mount.display());
                    return;
                }
            }
            let _ = std::fs::remove_file(&self.image);
        }
    }
    let unique = format!(
        "timem-fs-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let mut fixture = Image {
        image: std::env::temp_dir().join(format!("{unique}.dmg")),
        mount: PathBuf::from("/Volumes").join(&unique),
        attached: false,
    };
    let before = filesystem_usage_snapshot(&[]);
    let output = command_output(
        Command::new("/usr/bin/hdiutil")
            .args(["create", "-size", "16m", "-fs", "HFS+", "-volname", &unique])
            .arg(&fixture.image),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "image create: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = command_output(
        Command::new("/usr/bin/hdiutil")
            .args(["attach", "-nobrowse", "-quiet"])
            .arg(&fixture.image),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "image attach: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    fixture.attached = true;
    let device = filesystem_device_id(&fixture.mount).expect("mounted image identity");
    assert!(!before.iter().any(|fs| fs.device_id == device));
    // No working-path hint: mount discovery itself must see the attached disk.
    let mounted = filesystem_usage_snapshot(&[]);
    assert_eq!(
        mounted.iter().filter(|fs| fs.device_id == device).count(),
        1
    );
    assert!(mounted
        .iter()
        .any(|fs| fs.device_id == device && fs.total_bytes > 0 && fs.free_bytes > 0));
    let aliases = filesystem_usage_snapshot(&[fixture.mount.clone(), fixture.mount.join(".")]);
    assert_eq!(
        aliases.iter().filter(|fs| fs.device_id == device).count(),
        1
    );
    fixture.detach();
    assert!(!filesystem_usage_snapshot(&[])
        .iter()
        .any(|fs| fs.device_id == device));
}

#[cfg(target_os = "macos")]
#[test]
fn macos_mount_snapshots_are_safe_under_concurrent_session_startup() {
    let barrier = std::sync::Barrier::new(8);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                let mut missing_root = false;
                for _ in 0..200 {
                    barrier.wait();
                    let mounts = local_filesystem_mount_points();
                    missing_root |= !mounts.iter().any(|path| path == Path::new("/"));
                }
                assert!(
                    !missing_root,
                    "concurrent mount enumeration lost the root mount"
                );
            });
        }
    });
}
