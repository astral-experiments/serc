mod common;

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

#[test]
fn negated_constant_condition_before_module_annotations_matches_cpython() {
    let cases = [
        "if not True:\n    pass\nx: int\n",
        "if not True: pass\nx: int\n",
        "if not (\n    True\n):\n    pass\nx: int\n",
        "if not True:\n    pass\nelse:\n    value = 1\nx: int\n",
        "if not True:\n    value = 1\nelif flag:\n    value = 2\nx: int\n",
        "if not True:\n    value = (\"dead\", 123)\nx: int\n",
        "if not True: x: int\n",
        "if not True:\n    x: int\n",
        "if not True:\n    pass\nelif flag:\n    a = 1\nelif other:\n    a = 2\nelse:\n    a = 3\nx: int\n",
        "if not True:\n    pass\nx: int\nif not True:\n    pass\n",
        "if not (True): pass\nx: int\n",
        "if (not True): pass\nx: int\n",
        "if not True: pass; x: int\n",
        "if not True:\n    pass\nif not True:\n    pass\nx: int\n",
        "if not True:\n    pass\nx: int = 1\n",
        "x: int\nif not True:\n    pass\n",
        "\"example\"\nif not True:\n    pass\nx: int\n",
        "from __future__ import annotations\nif not True:\n    pass\nx: int\n",
        "if not True:\n    pass\nx = 1\n",
        "class C:\n    if not True:\n        pass\n    x: int\n",
        "if not True:\n    pass\nvalue = [x for x in (1, 2)]\nx: int\n",
        "if not True:\n    pass\nfor i in (1, 2):\n    value = i\nx: int\n",
        "if not True:\n    pass\ntry:\n    value = f()\nexcept Exception:\n    value = 1\nx: int\n",
        "if not True:\n    pass\nwith manager():\n    value = 1\nx: int\n",
        "if not True:\n    pass\nassert flag\nx: int\n",
        "if not True:\n    pass\ndef f(y):\n    return [y + x for x in (1, 2)]\nx: int\n",
        "if not True:\n    pass\n__conditional_annotations__ = []\nvalue = __conditional_annotations__\nx: int\n",
        "if not True:\n    pass\nif not True:\n    pass\nvalue = [x + 1 for x in (1, 2)]\nx: int\n",
        "if not True:\n    pass\nwhile False:\n    pass\nvalue = [x + 1 for x in (1, 2)]\nx: int\n",
        "if not True:\n    pass\nif True:\n    value = [x + 1 for x in (1, 2)]\nx: int\n",
        "if not True:\n    pass\nvalue = (x for x in (1, 2))\nx: int\n",
    ];
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("negated_conditional_annotations.py");
    let filename = source_path.to_string_lossy();
    let mut mismatches = Vec::new();
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for source in cases {
            fs_err::write(&source_path, source).unwrap();
            let mut native = match version {
                PythonVersion::Py312 => common::python_312_command(),
                PythonVersion::Py313 => common::python_313_command(),
                PythonVersion::Py314 => common::python_3145_command(),
                PythonVersion::Py315 => common::python_315_command(),
            };
            let native = native
                .args([
                    "-c",
                    "import marshal, pathlib, sys; p = pathlib.Path(sys.argv[1]); code = compile(p.read_bytes(), str(p), 'exec', dont_inherit=True, optimize=0); sys.stdout.buffer.write(marshal.dumps(code))",
                ])
                .arg(&source_path)
                .output()
                .unwrap();
            assert!(native.status.success(), "{version}: {source}: {native:?}");
            let options = CompileOptions {
                python_version: version,
                ..CompileOptions::default()
            };
            let text = compile_with_options(source, &filename, &options).unwrap();
            let bytes = compile_bytes_with_options(source.as_bytes(), &filename, &options).unwrap();
            if text.marshal() != native.stdout {
                mismatches.push(format!("text {version}: {source:?}"));
            }
            if bytes.marshal() != native.stdout {
                mismatches.push(format!("bytes {version}: {source:?}"));
            }
        }
    }
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}
