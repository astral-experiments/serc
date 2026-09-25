use crate::common::{assert_executes, assert_matches_cpython_marshal};

macro_rules! check {
    ($name:ident, $source:literal, $expected:literal) => {
        #[test]
        fn $name() {
            assert_executes($source, $expected);
            assert_matches_cpython_marshal($source, concat!(stringify!($name), ".py"));
        }
    };
}

check!(
    external_global_load,
    "y = 42\ng = ([y for x in [1] if [lambda: y for y in [7]]] for a in [1])\nprint(list(g))\n",
    "[[42]]\n"
);

check!(
    external_global_uncaptured_target,
    "y = 42\ng = ([y for x in [1] if [y for y in [7]]] for a in [1])\nprint(list(g))\n",
    "[[42]]\n"
);

check!(
    external_global_lambda,
    "y = 42\ng = ([lambda: y for x in [1] if [lambda: y for y in [7]]] for a in [1])\nprint(list(g)[0][0]())\n",
    "42\n"
);

check!(
    external_free_lambda,
    "def f(y):\n    return ([lambda: y for x in [1] if [lambda: y for y in [7]]] for a in [1])\ng = f(42)\ntry:\n    print(list(g)[0][0]())\nexcept NameError as error:\n    print(type(error).__name__)\n",
    "NameError\n"
);

check!(
    sibling_bound_first,
    "y = 42\ng = ([([y for y in [1]], [y for z in [1]]) for x in [1]] for a in [1])\nprint(list(g))\n",
    "[[([1], [42])]]\n"
);

check!(
    sibling_external_first,
    "y = 42\ng = ([([y for z in [1]], [y for y in [1]]) for x in [1]] for a in [1])\nprint(list(g))\n",
    "[[([42], [1])]]\n"
);

check!(
    global_lambda_after_uncaptured_target,
    "y = 42\ng = ([([y for y in [1]], (lambda: y)()) for x in [1]] for a in [1])\nprint(list(g))\n",
    "[[([1], 42)]]\n"
);

check!(
    global_lambda_before_uncaptured_target,
    "y = 42\ng = ([((lambda: y)(), [y for y in [1]]) for x in [1]] for a in [1])\nprint(list(g))\n",
    "[[(42, [1])]]\n"
);

check!(
    free_lambda_after_uncaptured_target,
    "def f(y):\n    return ([([y for y in [1]], (lambda: y)()) for x in [1]] for a in [1])\ntry:\n    print(list(f(42)))\nexcept NameError as error:\n    print(type(error).__name__)\n",
    "NameError\n"
);

check!(
    free_lambda_before_uncaptured_target,
    "def f(y):\n    return ([((lambda: y)(), [y for y in [1]]) for x in [1]] for a in [1])\ntry:\n    print(list(f(42)))\nexcept NameError as error:\n    print(type(error).__name__)\n",
    "NameError\n"
);

check!(
    nested_generator_external_lambda,
    "y = 42\ng = ([(f() for z in [1] for f in [lambda: y for u in [1] if [lambda: y for y in [1]]]) for w in [1] if [lambda: y for y in [1]]] for a in [1])\nprint(list(list(g)[0][0]))\n",
    "[42]\n"
);

check!(
    nested_generator_parent_cell,
    "def f(y):\n    return (([lambda: y for x in [1] if [lambda: y for y in [1]]] for z in [1]) for a in [1])\ninner = list(f(42))[0]\ntry:\n    print(list(inner)[0][0]())\nexcept NameError as error:\n    print(type(error).__name__)\n",
    "NameError\n"
);

check!(
    nested_generator_parent_cell_in_comprehension,
    "def f(y):\n    return ([([lambda: y for x in [1] if [lambda: y for y in [1]]] for z in [1]) for w in [1]] for a in [1])\ninner = list(f(42))[0][0]\ntry:\n    print(list(inner)[0][0]())\nexcept NameError as error:\n    print(type(error).__name__)\n",
    "NameError\n"
);

check!(
    global_walrus_in_sibling,
    "y = 42\ng = ([([(y := 1) for z in [1]], [lambda: y for y in [7]]) for x in [1]] for a in [1])\nprint(list(g)[0][0][0], y)\n",
    "[1] 1\n"
);

check!(
    sibling_free_bound_first,
    "def f(y):\n    return ([([lambda: y for y in [7]], [y for z in [1]]) for x in [1]] for a in [1])\ng = f(42)\ntry:\n    print(list(g))\nexcept UnboundLocalError as error:\n    print(type(error).__name__)\n",
    "UnboundLocalError\n"
);

check!(
    external_global_walrus,
    "y = 42\ng = ([(y := 1) for x in [1] if [lambda: y for y in [7]]] for a in [1])\nprint(list(g), y)\n",
    "[[1]] 1\n"
);

check!(
    external_free_captured_target,
    "def f(y):\n    return ([y for x in [1] if [lambda: y for y in [7]]] for a in [1])\nprint(list(f(42)))\n",
    "[[7]]\n"
);

check!(
    external_free_uncaptured_target,
    "def f(y):\n    return ([y for x in [1] if [y for y in [7]]] for a in [1])\nprint(list(f(42)))\n",
    "[[42]]\n"
);

check!(
    external_private_name,
    "_C__y = 42\nclass C:\n    g = ([__y for x in [1] if [lambda: __y for __y in [7]]] for a in [1])\nprint(list(C.g))\n",
    "[[42]]\n"
);

check!(
    mixed_private_closure_alias,
    "class C:\n    def f(__y):\n        return ([([lambda: _C__y for _C__y in [1]], (lambda: __y)()) for x in [1]] for a in [1])\ntry:\n    print(list(C.f(42)))\nexcept NameError as error:\n    print(type(error).__name__)\n",
    "NameError\n"
);

check!(
    mixed_private_global_alias,
    "_C__y = 42\nclass C:\n    g = ([([_C__y for _C__y in [1]], (lambda: __y)()) for x in [1]] for a in [1])\nprint(list(C.g))\n",
    "[[([1], 42)]]\n"
);

check!(
    mixed_private_global_alias_reversed,
    "_C__y = 42\nclass C:\n    g = ([([__y for __y in [1]], (lambda: _C__y)()) for x in [1]] for a in [1])\nprint(list(C.g))\n",
    "[[([1], 42)]]\n"
);

check!(
    mixed_private_active_target,
    "_C__y = 42\nclass C:\n    g = ([(_C__y, [_C__y for __y in [7]]) for x in [1]] for a in [1])\nprint(list(C.g))\n",
    "[[(42, [7])]]\n"
);

check!(
    mixed_private_generator_targets,
    "class C:\n    g = ([(__y, _C__y) for __y in [7] for _C__y in [8]] for a in [1])\nprint(list(C.g))\n",
    "[[(8, 8)]]\n"
);

check!(
    direct_generator_target,
    "y = 99\ng = ([y for x in [1] if [lambda: y for y in [1]]] for y in [42])\nprint(list(g))\n",
    "[[42]]\n"
);

check!(
    direct_comprehension_target,
    "y = 99\ng = ([y for y in [2] if [lambda: y for y in [1]]] for x in [1])\nprint(list(g))\n",
    "[[2]]\n"
);

check!(
    outer_comprehension_first_iterable,
    "y = 42\ng = ([y for x in [y] if [lambda: y for y in [1]]] for a in [1])\nprint(list(g))\n",
    "[[42]]\n"
);

check!(
    async_generator_external_global,
    "import asyncio\ny = 42\nasync def items():\n    yield 1\nasync def f():\n    return [value async for value in ([y for x in [1] if [lambda: y for y in [7]]] async for a in items())]\nprint(asyncio.run(f()))\n",
    "[[42]]\n"
);

check!(
    inline_target_and_external_lambda_capture_the_same_name,
    "def f(x):\n    g = (([lambda: x for x in [1]], lambda: x) for y in [1])\n    inner, outer = next(g)\n    print(inner[0]())\n    try:\n        print(outer())\n    except NameError as error:\n        print(type(error).__name__)\nf(4)\n",
    "1\nNameError\n"
);

#[test]
fn deeply_nested_generator_and_lambda_scopes() {
    for shape in 0..4 {
        let mut expression = "x".to_string();
        for index in 0..16 {
            expression = match shape {
                0 => format!("(lambda: {expression} for a{index} in [1])"),
                1 => format!("((lambda: {expression}, [z for z in [1]]) for a{index} in [1])"),
                2 => format!("([lambda: {expression} for z{index} in [1]] for a{index} in [1])"),
                _ => format!(
                    "(([lambda: x for x in [1]], lambda: x, lambda: {expression}) for a{index} in [1])"
                ),
            };
        }
        let source = format!("def f(x):\n    return {expression}\n");
        assert_matches_cpython_marshal(&source, &format!("nested_scope_{shape}.py"));
    }
}

#[test]
fn matches_other_cpython_targets() {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    use serc::{CompileOptions, PythonVersion, compile_with_options};

    let oracle = r"
import importlib.util, marshal, sys
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]
code = compile(sys.stdin.read(), sys.argv[3], 'exec', dont_inherit=True, optimize=0)
sys.stdout.buffer.write(marshal.dumps(code))
";
    let filename = "generator_external_shadow.py";
    for (version, variable, default) in [
        (PythonVersion::Py312, "SERC_TEST_PYTHON_312", "python3.12"),
        (PythonVersion::Py313, "SERC_TEST_PYTHON_313", "python3.13"),
        (PythonVersion::Py315, "SERC_TEST_PYTHON_315", "python3.15"),
    ] {
        let python = std::env::var_os(variable).unwrap_or_else(|| default.into());
        let target = version.target();
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for source in [
            "y = 42\ng = ([y for x in [1] if [lambda: y for y in [7]]] for a in [1])\n",
            "y = 42\ng = ([lambda: y for x in [1] if [lambda: y for y in [7]]] for a in [1])\n",
            "def f(y):\n    return ([lambda: y for x in [1] if [lambda: y for y in [7]]] for a in [1])\n",
            "def f(y):\n    return (([lambda: y for x in [1] if [lambda: y for y in [1]]] for z in [1]) for a in [1])\n",
            "class C:\n    def f(__y):\n        return ([([lambda: _C__y for _C__y in [1]], (lambda: __y)()) for x in [1]] for a in [1])\n",
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
}
