use crate::{CompileOptions, PythonVersion, compile_with_options};
use serc_bytecode::marshal::reader::read;

#[test]
fn all_targets_and_headers() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        let source = "def f():\n    return ('shared', 10**20)\nx = 'shared'\n";
        let compiled = compile_with_options(
            source,
            "fixture.py",
            &CompileOptions {
                python_version: version,
                ..CompileOptions::default()
            },
        )
        .unwrap();
        for bytes in [
            compiled.to_timestamp_pyc(123, 42),
            compiled.to_hash_pyc(source.as_bytes(), true),
            compiled.to_hash_pyc(source.as_bytes(), false),
        ] {
            let module = read(&bytes).unwrap();
            let root = module.code(module.root).unwrap();
            assert_eq!(module.text(root.name).unwrap(), "<module>");
            let constants = module.tuple(root.constants).unwrap();
            assert!(constants.iter().any(|id| module.code(*id).is_ok()));
            for end in 0..bytes.len() {
                assert!(read(&bytes[..end]).is_err());
            }
        }
    }
}
