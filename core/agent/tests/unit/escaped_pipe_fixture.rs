//! Real fork/setsid fixture shared by macOS output-capture tests.
//! Compilation and the first native launch are setup, not part of the deadline.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

pub(crate) struct NativeEscapedPipeFixture {
    dir: PathBuf,
}

impl NativeEscapedPipeFixture {
    pub(crate) fn new(root: &Path) -> Self {
        let dir = root.join("escaped-pipe-fixture");
        fs::create_dir_all(&dir).unwrap();
        let source = dir.join("fixture.c");
        let executable = dir.join("fixture");
        fs::write(&source, include_str!("escaped_pipe_fixture.c")).unwrap();
        let output = Command::new("cc")
            .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .output()
            .expect("compile native escaped-pipe fixture");
        assert!(output.status.success(), "fixture compiler: {output:?}");
        assert!(Command::new(&executable)
            .arg("--probe")
            .status()
            .expect("probe native escaped-pipe fixture")
            .success());
        Self { dir }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.dir
    }

    pub(crate) fn command(&self, keep_launcher: bool) -> String {
        let quote =
            |path: &Path| format!("'{}'", path.display().to_string().replace('\'', "'\\''"));
        format!(
            "printf shell_started > {}\n{} {} {}",
            quote(&self.dir.join("phase")),
            quote(&self.dir.join("fixture")),
            quote(&self.dir),
            if keep_launcher { "keep" } else { "exit" }
        )
    }
}

impl NativeEscapedPipeFixture {
    pub(crate) fn release(&self) {
        // Private cooperative release only; never signal a guessed descendant.
        let _ = fs::write(self.dir.join("release"), "");
        let deadline = Instant::now() + Duration::from_secs(9);
        while self.dir.join("ready").exists()
            && !self.dir.join("done").exists()
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

impl Drop for NativeEscapedPipeFixture {
    fn drop(&mut self) {
        self.release();
    }
}
