use serc::{CompileOptions, PythonVersion, compile_with_options};

use crate::cpython::{command, run_with_input};

pub(crate) fn assert_matches_cpython(version: PythonVersion, name: &str, source: &str) -> Vec<u8> {
    const ORACLE: &str = "import importlib.util, marshal, sys; assert sys.implementation.name == 'cpython'; assert sys.version.split()[0] == sys.argv[1]; assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]; code = compile(sys.stdin.read(), 'private_type_parameters.py', 'exec', dont_inherit=True, optimize=0); sys.stdout.buffer.write(marshal.dumps(code))";
    let target = version.target();
    let expected = run_with_input(
        command(version)
            .args(["-I", "-S", "-c", ORACLE])
            .args([target.tag.trim_start_matches('v'), &target.magic_hex()]),
        source.as_bytes(),
    );
    assert!(
        expected.status.success(),
        "{version:?} {name}: {}",
        String::from_utf8_lossy(&expected.stderr)
    );
    let options = CompileOptions {
        python_version: version,
        ..CompileOptions::default()
    };
    let actual = compile_with_options(source, "private_type_parameters.py", &options)
        .unwrap_or_else(|error| panic!("{version:?} {name}: {error:?}"))
        .marshal();
    let first_difference = actual
        .iter()
        .zip(&expected.stdout)
        .position(|(a, b)| a != b);
    assert!(
        actual == expected.stdout,
        "{version:?} {name}: first difference {first_difference:?}, lengths {} and {}",
        actual.len(),
        expected.stdout.len()
    );
    actual
}
