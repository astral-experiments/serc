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
    use std::process::Command;
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
        if !output.status.success() {
            return Err(format!("CPython reference identity failed: {output:?}"));
        }
        let actual: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("invalid CPython reference identity: {error}"))?;
        if actual != expected_identity() {
            return Err(format!("unexpected CPython reference identity: {actual}"));
        }
        Ok(executable)
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
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("reference");
        let write_identity = |identity: &serde_json::Value| {
            fs_err::write(&path, format!("#!/bin/sh\nprintf '%s\\n' '{identity}'\n")).unwrap();
            fs_err::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        write_identity(&expected_identity());
        assert_eq!(discover(Some(path.as_os_str())).unwrap(), path);
        for (field, value) in [
            ("protocol", serde_json::json!("other")),
            ("protocol", serde_json::json!("minimal-v1")),
            ("implementation", serde_json::json!("other")),
            ("version", serde_json::json!("3.14.4")),
            ("release_level", serde_json::json!("candidate")),
            ("release_serial", serde_json::json!(1)),
            ("magic", serde_json::json!("00000000")),
        ] {
            let mut identity = expected_identity();
            identity[field] = value;
            write_identity(&identity);
            assert!(discover(Some(path.as_os_str())).is_err(), "{field}");
        }
    }
}
