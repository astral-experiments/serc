mod common;

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

#[test]
fn constant_condition_after_module_annotation_setup_matches_cpython() {
    let cases = [
        "if False:\n    pass\nx: int\n",
        "if True:\n    pass\nx: int\n",
        "if 0:\n    pass\nx: int\n",
        "if 1:\n    pass\nx: int\n",
        "if 1-1:\n    pass\nx: int\n",
        "if (\n    False\n):\n    pass\nx: int\n",
        "if False:\n    pass\nelse:\n    value = 1\nx: int\n",
        "if True:\n    value = 1\nelse:\n    pass\nx: int\n",
        "if False: pass\nx: int\n",
        "if True: pass\nx: int\n",
        "if False:\n    pass\nx: int = 1\n",
        "x: int\nif False:\n    pass\n",
        "\"example\"\nif False:\n    pass\nx: int\n",
        "from __future__ import annotations\nif False:\n    pass\nx: int\n",
        "if False:\n    pass\ndef f(x: int):\n    pass\n",
        "if False:\n    pass\nx = 1\n",
        "class C:\n    if False:\n        pass\n    x: int\n",
        "if True:\n    if False:\n        pass\nx: int\n",
        "if False:\n    pass\nif False:\n    pass\nx: int\n",
        "if False:\n    pass\ny = [x for x in (1, 2)]\nx: int\n",
        "if False:\n    pass\nobj.x: int\n",
        "if False:\n    pass\nfor i in (1, 2):\n    value = i\nx: int\n",
        "if False:\n    pass\ntry:\n    value = f()\nexcept Exception:\n    value = 1\nx: int\n",
        "if True:\n    value = 1\ntry:\n    f()\nfinally:\n    g()\nx: int\n",
        "if True:\n    for i in (1, 2):\n        value = i\nx: int\n",
        "if False:\n    pass\nvalue = [(x, y) for x in (1, 2) for y in (3, 4)]\nx: int\n",
        "if True:\n    pass\nvalue = (x for x in (1, 2))\nx: int\n",
        "if False:\n    value = (\"dead\", 123, (4, 5))\nelse:\n    value = \"live\"\nx: int\n",
        "if True: pass\nif True: pass\nx: int\n",
        "if False: pass\nif False: pass\nx: int\n",
        "if True:\n    try:\n        value = f()\n    finally:\n        value = 1\nx: int\n",
        "if False:\n    value: int\nvalue = 1\n",
        "if True:\n    try:\n        f()\n    except Exception:\n        g()\nx: int\n",
        "if False:\n    value = 0\nelse:\n    value = 1\n    other = 2\nx: int\n",
        "if True:\n    value = 1\n    other = 2\nelse:\n    value = 0\nx: int\n",
        "if False:\n    pass\ntry:\n    value = (x for x in (1, 2))\nexcept Exception:\n    value = None\nx: int\n",
        "if (\n    True\n):\n    value = 1\nx: int\n",
    ];
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("conditional_annotations.py");
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
