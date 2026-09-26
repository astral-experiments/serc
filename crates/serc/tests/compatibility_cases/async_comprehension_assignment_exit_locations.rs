use crate::common;

#[test]
fn matches_cpython_async_comprehension_assignment_exit_locations() {
    for expression in [
        "[item async for item in items]",
        "{item async for item in items}",
        "{item: item async for item in items}",
    ] {
        let source = format!(
            "async def f(items):\n    try:\n        values = {expression}\n    except Error:\n        recover()\n    finish(values)\n"
        );
        common::assert_matches_cpython_marshal(
            &source,
            "async_comprehension_assignment_exit_locations.py",
        );
    }
}

#[test]
fn matches_cpython_comprehension_assignment_successor_locations() {
    for source in [
        "async def probe(streams):\n    for stream in streams:\n        result = [item async for item in stream]\n",
        "def probe(stream):\n    result = [\n        item for item in stream\n    ]\n    summary = (\n        \"prefix %s\"\n        % (len(result),)\n    )\n    return summary\n",
    ] {
        common::assert_matches_cpython_marshal(
            source,
            "comprehension_assignment_successor_locations.py",
        );
    }
}

#[test]
fn terminal_async_comprehension_assertions_match_cpython_312() {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    use serc::{CompileOptions, PythonVersion, compile_with_options};

    let version = PythonVersion::Py312;
    let target = version.target();
    let python = std::env::var_os("SERC_TEST_PYTHON_312").unwrap_or_else(|| "python3.12".into());
    let options = CompileOptions {
        python_version: version,
        ..CompileOptions::default()
    };
    let filename = "async_comprehension_assertion.py";
    let oracle = r"
import importlib.util, marshal, sys
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]
code = compile(sys.stdin.read(), sys.argv[3], 'exec', dont_inherit=True, optimize=0)
sys.stdout.buffer.write(marshal.dumps(code))
";
    for source in [
        "async def f(xs):\n    if xs:\n        assert [x async for x in xs]\n",
        "async def f(xs):\n    if xs:\n        assert {x async for x in xs}\n",
        "async def f(xs):\n    if xs:\n        assert {x: x async for x in xs}\n",
        "async def f(xs):\n    if xs:\n        assert [x async for x in xs], 'empty'\n",
        "async def f(xs):\n    if xs:\n        assert [x async for x in xs if x]\n",
        "async def f(xs):\n    if xs:\n        assert [x async for x in xs] == [1]\n",
        "async def f(xs):\n    if xs:\n        assert [1] == [x async for x in xs]\n",
        "async def f(xs):\n    if xs:\n        assert {x async for x in xs} != set()\n",
        "async def f(xs):\n    if xs:\n        assert 1 in [x async for x in xs]\n",
        "async def f(xs):\n    if xs:\n        assert [] == [x async for x in xs] == []\n",
        "async def f(xs):\n    if xs:\n        assert True == ([x async for x in xs] == [])\n",
        "async def f(xs):\n    if xs:\n        assert not ([x async for x in xs] == [])\n",
        "async def f(xs):\n    if xs:\n        assert False and ([x async for x in xs] == [])\n",
        "async def f(xs):\n    if xs:\n        assert True or ([x async for x in xs] == [])\n",
        "async def f(xs):\n    if xs:\n        assert False and (True == ([x async for x in xs] == []))\n",
        "async def f(xs):\n    if xs:\n        return 1\n        assert [x async for x in xs] == []\n",
        "async def f(xs):\n    if xs:\n        assert [x for x in xs] == []\n",
        "async def f(xs, flag):\n    if xs:\n        assert [x async for x in xs] and flag\n",
        "async def f(xs, flag):\n    if xs:\n        assert [x async for x in xs] or flag\n",
        "async def f(xs):\n    if xs:\n        assert not [x async for x in xs]\n",
        "async def f(xs):\n    if xs:\n        assert True and [x async for x in xs]\n",
        "async def f(xs):\n    if xs:\n        assert False or [x async for x in xs]\n",
        "async def f(xs):\n    if xs:\n        assert False and [x async for x in xs]\n",
        "async def f(xs):\n    if xs:\n        assert True or [x async for x in xs]\n",
        "async def f(xs):\n    if xs:\n        return 1\n        assert [x async for x in xs]\n",
        "async def f(xs):\n    if xs:\n        raise ValueError\n        assert [x async for x in xs]\n",
        "async def f(xs):\n    if xs:\n        assert [x async for x in xs]\n    else:\n        pass\n",
        "async def f(xs):\n    assert [x async for x in xs]\n",
        "async def f(xs):\n    if xs:\n        assert [x for x in xs]\n",
        "async def f(xs):\n    if xs:\n        assert [x async for x in xs]\n    return None\n",
    ] {
        let mut child = Command::new(&python)
            .args(["-I", "-S", "-X", "utf8", "-c", oracle])
            .args([
                target.tag.strip_prefix('v').unwrap(),
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
            "{version}: {}",
            String::from_utf8_lossy(&expected.stderr)
        );
        assert_eq!(
            compile_with_options(source, filename, &options)
                .unwrap()
                .marshal(),
            expected.stdout,
            "{version}: {source}"
        );
    }
}
