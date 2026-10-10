pub(crate) fn assert_matches(source: &str, filename: &str) {
    #[cfg(target_os = "linux")]
    native::assert_matches(source, filename);
    #[cfg(not(target_os = "linux"))]
    crate::common::assert_matches_cpython_marshal(source, filename);
}

#[cfg(target_os = "linux")]
mod native {
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};
    use std::sync::OnceLock;

    use serc::{CPYTHON_TARGET, compile};

    static REFERENCE: OnceLock<Result<PathBuf, String>> = OnceLock::new();

    fn command(executable: &Path) -> Command {
        let mut command = Command::new(executable);
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("LANG", "C.UTF-8");
        command
    }

    fn expected_identity() -> serde_json::Value {
        serde_json::json!({
            "protocol": "minimal-v2",
            "implementation": CPYTHON_TARGET.implementation,
            "version": CPYTHON_TARGET.version_string(),
            "release_level": "final",
            "release_serial": 0,
            "magic": CPYTHON_TARGET.magic_hex(),
        })
    }

    fn discover(configured: Option<&OsStr>) -> Result<PathBuf, String> {
        let configured = configured.ok_or_else(|| {
            "startup-sensitive tests require SERC_TEST_CPYTHON_REFERENCE; build it with scripts/build-cpython-reference.py".to_string()
        })?;
        let executable = Path::new(configured)
            .canonicalize()
            .map_err(|error| format!("invalid SERC_TEST_CPYTHON_REFERENCE: {error}"))?;
        let output = command(&executable)
            .arg("--identity")
            .output()
            .map_err(|error| format!("failed to run CPython reference identity: {error}"))?;
        validate_identity(&output)?;
        Ok(executable)
    }

    fn validate_identity(output: &Output) -> Result<(), String> {
        if !output.status.success() {
            return Err(format!("CPython reference identity failed: {output:?}"));
        }
        let actual: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("invalid CPython reference identity: {error}"))?;
        if actual != expected_identity() {
            return Err(format!("unexpected CPython reference identity: {actual}"));
        }
        Ok(())
    }

    pub(super) fn assert_matches(source: &str, filename: &str) {
        let executable = REFERENCE
            .get_or_init(|| discover(std::env::var_os("SERC_TEST_CPYTHON_REFERENCE").as_deref()))
            .as_ref()
            .unwrap_or_else(|error| panic!("{error}"));
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(Path::new(filename).file_name(), Some(OsStr::new(filename)));
        fs_err::write(directory.path().join(filename), source).unwrap();
        // The native startup profile excludes Python wrapper imports. Keep the
        // original relative co_filename while the native helper reads its input.
        let expected = command(executable)
            .current_dir(directory.path())
            .args(["--optimize", "0", filename])
            .output()
            .unwrap();
        assert!(
            expected.status.success(),
            "native oracle failed: {expected:?}"
        );
        assert_eq!(
            compile(source, filename).unwrap().marshal(),
            expected.stdout,
            "native startup marshal mismatch for {filename}",
        );
    }

    #[test]
    fn missing_reference_is_an_error() {
        assert!(
            discover(None)
                .unwrap_err()
                .contains("SERC_TEST_CPYTHON_REFERENCE")
        );
        let directory = tempfile::tempdir().unwrap();
        assert!(discover(Some(directory.path().join("missing").as_os_str())).is_err());
    }

    #[test]
    fn wrong_reference_identity_is_an_error() {
        use std::os::unix::process::ExitStatusExt as _;
        use std::process::ExitStatus;

        let output = |identity: &serde_json::Value| Output {
            status: ExitStatus::from_raw(0),
            stdout: serde_json::to_vec(identity).unwrap(),
            stderr: Vec::new(),
        };
        assert!(validate_identity(&output(&expected_identity())).is_ok());
        for (field, value) in [
            ("protocol", serde_json::json!("other")),
            ("protocol", serde_json::json!("minimal-v1")),
            ("implementation", serde_json::json!("other")),
            ("version", serde_json::json!("3.14.4")),
            ("release_level", serde_json::json!("candidate")),
            ("release_serial", serde_json::json!(1)),
            ("magic", serde_json::json!("00000000")),
            ("extra", serde_json::json!(true)),
        ] {
            let mut identity = expected_identity();
            identity[field] = value;
            assert!(
                validate_identity(&output(&identity))
                    .unwrap_err()
                    .starts_with("unexpected CPython reference identity:"),
                "{field}",
            );
        }
        let mut invalid = output(&expected_identity());
        invalid.stdout = b"not JSON".to_vec();
        assert!(
            validate_identity(&invalid)
                .unwrap_err()
                .starts_with("invalid CPython reference identity:")
        );
        let mut failed = output(&expected_identity());
        failed.status = ExitStatus::from_raw(1 << 8);
        assert!(
            validate_identity(&failed)
                .unwrap_err()
                .starts_with("CPython reference identity failed:")
        );
    }
}
