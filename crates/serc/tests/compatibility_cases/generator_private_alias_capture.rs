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
    explicit_target_private_capture,
    "class C:\n    g = (lambda: __x for _C__x in [2])\nprint(next(C.g)())\n",
    "2\n"
);

check!(
    private_target_explicit_capture,
    "class C:\n    g = (lambda: _C__x for __x in [2])\nprint(next(C.g)())\n",
    "2\n"
);

check!(
    generic_class_capture,
    "class C[__T]:\n    g = (lambda: __x for _C__x in [2])\nprint(next(C.g)())\n",
    "2\n"
);

check!(
    generic_class_type_parameter_shadow,
    "class C[__T]:\n    g = (lambda: _C__T for __T in [2])\nprint(next(C.g)())\n",
    "2\n"
);

check!(
    generator_target_shadows_module_global,
    "_C__x = 99\nclass C:\n    g = (lambda: _C__x for __x in [2])\nprint(next(C.g)())\n",
    "2\n"
);

check!(
    generator_target_shadows_method_parameter,
    "class C:\n    def f(self, __x):\n        return (lambda: _C__x for __x in [2])\nprint(next(C().f(99))())\n",
    "2\n"
);

check!(
    multiple_captured_targets,
    "class C:\n    g = (lambda: (__x, _C__y) for _C__x, __y in [(2, 3)] for z in [5])\nprint(next(C.g)())\n",
    "(2, 3)\n"
);

check!(
    captured_cell_order_uses_mangled_names,
    "class C:\n    g = (lambda: (__a, __x) for _C__x, __a in [(2, 3)])\nprint(next(C.g)())\n",
    "(3, 2)\n"
);

check!(
    captured_cell_order_in_reversed_spelling,
    "class C:\n    g = (lambda: (_C__a, _C__x) for __x, _C__a in [(2, 3)])\nprint(next(C.g)())\n",
    "(3, 2)\n"
);

check!(
    lambda_parameter_shadow_control,
    "class C:\n    g = (lambda __x: _C__x for _C__x in [2])\nprint(next(C.g)(4))\n",
    "4\n"
);

check!(
    lambda_parameter_shadow_control_reversed,
    "class C:\n    g = (lambda _C__x: __x for __x in [2])\nprint(next(C.g)(4))\n",
    "4\n"
);

check!(
    first_iterable_is_outside_generator,
    "_C__x = [1]\nclass C:\n    g = (_C__x for __x in (lambda: __x)())\nprint(list(C.g))\n",
    "[1]\n"
);

check!(
    lambda_default_control,
    "class C:\n    g = (lambda v=__x: v for _C__x in [2])\nprint(next(C.g)())\n",
    "2\n"
);

check!(
    aliased_repeated_generator_target,
    "class C:\n    g = (lambda: __x for _C__x in [2] for __x in [3])\nprint(next(C.g)())\n",
    "3\n"
);

check!(
    lambda_shadowing_control,
    "class C:\n    g = (lambda: (lambda __x: _C__x)(7) for _C__x in [2])\nprint(next(C.g)())\n",
    "7\n"
);

check!(
    nested_lambda_capture,
    "class C:\n    g = (lambda: (lambda: __x) for _C__x in [2])\nprint(next(C.g)()())\n",
    "2\n"
);

check!(
    captured_target_inlined_shadow,
    "class C:\n    g = (lambda: __x for _C__x in [2] if [lambda: __x for __x in [1]])\nprint(next(C.g)())\n",
    "2\n"
);

check!(
    inlined_list_target_shadows_generator_target,
    "class C:\n    g = ([lambda: __x for _C__x in [7]] for __x in [2])\nprint(list(C.g)[0][0]())\n",
    "7\n"
);

check!(
    inlined_list_target_shadows_generator_target_reversed,
    "class C:\n    g = ([lambda: _C__x for __x in [7]] for _C__x in [2])\nprint(list(C.g)[0][0]())\n",
    "7\n"
);

check!(
    captured_inlined_set_target,
    "class C:\n    g = ({lambda: __x for _C__x in [7]} for t in [1])\nprint(next(iter(list(C.g)[0]))())\n",
    "7\n"
);

check!(
    captured_inlined_dict_target,
    "class C:\n    g = ({i: lambda: __x for _C__x in [7] for i in [0]} for t in [1])\nprint(list(C.g)[0][0]())\n",
    "7\n"
);

check!(
    nested_generator_captures_outer_target,
    "class C:\n    g = ((lambda: __x for t in [1]) for _C__x in [2])\nprint([[f() for f in h] for h in C.g])\n",
    "[[2]]\n"
);

check!(
    late_bound_generator_target,
    "class C:\n    g = (lambda: __x for _C__x in [1, 2])\nprint([f() for f in list(C.g)])\n",
    "[2, 2]\n"
);

check!(
    captured_target_slot_order,
    "values = (1, 2)\nclass C:\n    g = (lambda: __x for y, _C__x in [values] if [__x for __x in [1]] for z in [2])\nprint(C.g.gi_code.co_varnames, next(C.g)())\n",
    "('.0', 'y', '_C__x', 'z') 2\n"
);

check!(
    immediate_nested_lambda,
    "class C:\n    g = ((lambda: __x)() for _C__x in [2])\nprint(list(C.g))\n",
    "[2]\n"
);

check!(
    async_generator_target,
    "import asyncio\nasync def items():\n    yield 2\nclass C:\n    g = (lambda: __x async for _C__x in items())\nasync def run():\n    return [(f()) async for f in C.g]\nprint(asyncio.run(run()))\n",
    "[2]\n"
);

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
    let filename = "generator_private_alias_capture.py";
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
            "class C:\n    g = (lambda: __x for _C__x in [2])\n",
            "class C:\n    g = (lambda: _C__x for __x in [2])\n",
            "class C[__T]:\n    g = (lambda: __x for _C__x in [2])\n",
            "class C[__T]:\n    g = (lambda: _C__x for __x in [2])\n",
            "class C[__T]:\n    g = (lambda: __T for _C__T in [2])\n",
            "class C[__T]:\n    g = (lambda: _C__T for __T in [2])\n",
            "class C:\n    def f(self, __x):\n        return (lambda: _C__x for __x in [2])\n",
            "class C:\n    g = (lambda: (__a, __x) for _C__x, __a in [(2, 3)])\n",
            "class C:\n    g = (lambda __x: _C__x for _C__x in [2])\n",
            "class C:\n    g = (lambda _C__x: __x for __x in [2])\n",
            "class C:\n    g = (lambda v=__x: v for _C__x in [2])\n",
            "_C__x = [1]\nclass C:\n    g = (_C__x for __x in (lambda: __x)())\n",
            "class C:\n    g = (lambda: __x for _C__x in [2] for __x in [3])\n",
            "class C:\n    g = ((lambda: __x for _C__x in [2]) for a in [1])\n",
            "class C:\n    g = ([lambda: __x for _C__x in [7]] for __x in [2])\n",
            "class C:\n    g = ({lambda: __x for _C__x in [7]} for t in [1])\n",
            "class C:\n    g = ({i: lambda: __x for _C__x in [7] for i in [0]} for t in [1])\n",
            "class C:\n    g = ((lambda: __x for t in [1]) for _C__x in [2])\n",
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
