use std::io::Write as _;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_with_options};

fn matches_python312(source: &str, case: &str) {
    let target = PythonVersion::Py312.target();
    let python = std::env::var_os("SERC_TEST_PYTHON_312").unwrap_or_else(|| "python3.12".into());
    let filename = "native_source_jump_width.py";
    let oracle = "import importlib.util, marshal, sys; assert sys.implementation.name == 'cpython'; assert sys.version.split()[0] == sys.argv[1]; assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]; code = compile(sys.stdin.read(), sys.argv[3], 'exec', dont_inherit=True, optimize=0); sys.stdout.buffer.write(marshal.dumps(code))";
    let mut child = Command::new(python)
        .args(["-B", "-I", "-S", "-c", oracle])
        .args([
            target.tag.trim_start_matches('v'),
            &target.magic_hex(),
            filename,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    let expected = child.wait_with_output().unwrap();
    assert!(
        expected.status.success(),
        "{case}: {}",
        String::from_utf8_lossy(&expected.stderr)
    );
    let options = CompileOptions {
        python_version: PythonVersion::Py312,
        ..CompileOptions::default()
    };
    assert_eq!(
        compile_with_options(source, filename, &options)
            .unwrap()
            .marshal(),
        expected.stdout,
        "{case}"
    );
}

#[test]
fn repeated_while_conditions_preserve_one_byte_jump_boundaries() {
    for prefix in [0, 60, 250, 256] {
        for padding in [105, 106, 107, 113, 114, 115] {
            let mut source = String::from("async def f(flag):\n");
            source.push_str(&"    if flag:\n        padding = 0\n".repeat(prefix));
            source.push_str("    while await ready():\n        pass\n");
            source.push_str(&"    padding = 0\n".repeat(padding));
            source.push_str("    await done()\n");
            matches_python312(&source, &format!("{prefix} branches, {padding} stores"));
        }
    }
}

#[test]
fn expanded_native_routes_preserve_deferred_await_jump_boundaries() {
    for (name, route) in [
        ("binary", "    if flag & 1:\n        work()\n"),
        (
            "pattern",
            "    match flag:\n        case Color.RED | Color.BLUE:\n            work()\n        case _:\n            finish()\n",
        ),
        (
            "async_comp",
            "    values = [await transform(item) async for item in items if item]\n",
        ),
        (
            "mixed_comp",
            "    values = [(x, y) for x in items async for y in stream(x) if y]\n",
        ),
        (
            "nested_comp",
            "    values = [[y async for y in stream(x)] for x in items]\n",
        ),
        (
            "singleton_comp",
            "    values = {name: (value if value is not None else flag) for name in items for value in (lookup(name),)}\n",
        ),
        (
            "filtered_singleton",
            "    values = [value for item in items for value in [lookup(item)] if value or flag]\n",
        ),
        (
            "nested_singleton",
            "    values = [[value for item in row for value in (lookup(item),)] for row in items]\n",
        ),
        (
            "singleton_unpack",
            "    values = [(left, right) for item in items for left, right in [(lookup(item), flag)]]\n",
        ),
        (
            "inner_manager",
            "    try:\n        async with manager():\n            if flag:\n                return value\n    finally:\n        await finish()\n",
        ),
        (
            "nested_finally",
            "    try:\n        try:\n            if flag:\n                return value\n        finally:\n            await first()\n    finally:\n        await second()\n",
        ),
        (
            "cleanup_manager",
            "    try:\n        if flag:\n            return value\n    finally:\n        async with manager():\n            await finish()\n",
        ),
        (
            "interleaved",
            "    async with outer():\n        try:\n            async with inner():\n                try:\n                    if flag:\n                        return value\n                finally:\n                    await first()\n        finally:\n            await second()\n",
        ),
        ("terminal_if", ""),
    ] {
        for padding in [100, 101, 102, 109, 110, 111, 115, 116, 117] {
            let mut source = String::from("async def f(flag, items, value):\n");
            // More than the uniform bound, while native source IDs still fit in one byte.
            source.push_str(&"    if flag:\n        padding = 0\n".repeat(60));
            source.push_str(route);
            source.push_str("    await begin()\n");
            source.push_str(&"    padding = 0\n".repeat(padding));
            source.push_str("    await done()\n");
            if name == "terminal_if" {
                source.push_str("    if flag:\n        work()\n    else:\n        finish()\n");
            }
            matches_python312(&source, &format!("{name}, {padding} stores"));
        }
    }
}

#[test]
fn repeated_while_conditions_preserve_two_byte_jump_boundaries() {
    for padding in [32745, 32746, 32747, 32753, 32754, 32755] {
        let mut source = String::from("async def f(flag):\n");
        source.push_str(&"    if flag:\n        padding = 0\n".repeat(60));
        // Physical instruction indices exceed 65535, but native source labels do not.
        source.push_str(&"    padding = 0\n".repeat(32800));
        source.push_str("    while await ready():\n        pass\n");
        source.push_str(&"    padding = 0\n".repeat(padding));
        source.push_str("    await done()\n");
        matches_python312(&source, &format!("large while, {padding} stores"));
    }
}
