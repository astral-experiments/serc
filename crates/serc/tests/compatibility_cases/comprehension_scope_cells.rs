use crate::common;
use crate::common::indent;

#[test]
fn generator_inline_cell_slot_order_matches_cpython() {
    let source = r"g = (locals() for x, y in [(1, 2)] if [lambda: x for x in [1]])
print(list(next(g)))
";
    common::assert_executes(source, "['.0', 'y', 'x']\n");
    common::assert_matches_cpython_marshal(source, "generator_inline_cell_slot_order.py");

    for (index, expression) in [
        "(locals() for x, y in [(1, 2)] if [lambda: y for y in [1]])",
        "(locals() for x, y in [(1, 2)] if [lambda: x for x in [1]] if [lambda: y for y in [1]])",
        "(locals() for x, y, z in [(1, 2, 3)] if [lambda: z for z in [1]] if [lambda: x for x in [1]])",
        "([lambda: x for x in [1]] for x, y in [(1, 2)] if [lambda: y for y in [1]])",
        "(locals() for x in [1] if [lambda: z for z in [1]] for y in [2])",
        "(locals() for x in [1] for y in [2] if [lambda: x for x in [1]])",
        "(locals() for x, y in [(1, 2)] if [lambda: (x, y) for x in [1]])",
        "(locals() for x, y in [(1, 2)] if [lambda: x for z in [1]])",
        "(locals() for x, y in [(1, 2)] if [x for x in [1]])",
        "(locals() for x, y in [(1, 2)] if [lambda: x for x in [lambda: y for y in [1]]])",
        "(locals() for x, y in [(1, 2)] if [lambda: y for y in [lambda: x for x in [1]]])",
        "(locals() for x, y in [(1, 2)] for box[[lambda: x for x in [1]]] in [3])",
        "(locals() for x, box[[lambda: y for y in [1]]], z, y in [values])",
        "(locals() for z in [1] for y in [x] for x in [2])",
        "(locals() for a in [1] if [x for y in [1]] for z in [2] for x in [3])",
        "(lambda: x for x in [1])",
        "(lambda: (x, z) for x, z in [(1, 2)])",
        "(locals() for x, a, z, y in [values] if [y for a in [1] if [lambda: y for y in [1]] if [lambda: z for z in [1]]])",
    ]
    .iter()
    .enumerate()
    {
        let source = format!("g = {expression}\n");
        let filename = format!("generator_inline_cell_slot_order_{index}.py");
        common::assert_matches_cpython_marshal(&source, &filename);
    }

    let source = r"g = (lambda: x for x, z in [(1, 2)] if [x for x in [1]] for z in [2])
print(next(g)())
";
    common::assert_executes(source, "1\n");
    common::assert_matches_cpython_marshal(source, "generator_shadowed_cell_slot.py");

    let source = r"async def f(it):
    return (locals() async for x, y in it if [lambda: x for x in [1]])
";
    common::assert_matches_cpython_marshal(source, "async_generator_inline_cell_slot.py");

    let source = r"values = (1, 2)
class C:
    g = (lambda: __x for y, _C__x in [values] if [__x for __x in [1]] for z in [2])
print(C.g.gi_code.co_varnames)
";
    common::assert_executes(source, "('.0', 'y', '_C__x', 'z')\n");
}

fn comprehension(kind: &str, element: &str, target: &str) -> String {
    match kind {
        "list" => format!("[{element} for {target} in values]"),
        "set" => format!("{{{element} for {target} in values}}"),
        "dict" => format!("{{{target}: {element} for {target} in values}}"),
        _ => unreachable!(),
    }
}

#[test]
fn nested_generator_reference_resolves_global_after_first_iterable() {
    let source = r"x = 'global'
def outer():
    def f():
        return (x for y in [x for x in [1]])
    return next(f())
print(outer())
";
    common::assert_executes(source, "global\n");
    common::assert_matches_cpython_marshal(source, "nested_generator_global.py");
}

#[test]
fn nested_generator_temporary_preserves_enclosing_binding_rules() {
    let cases = [
        (
            r"x = 'global'
def outer():
    x = 'outer'
    def f():
        return (x for y in [x for x in [1]])
    return next(f())
try:
    print(outer())
except NameError:
    print('unbound')
",
            "unbound\n",
        ),
        (
            r"x = 'global'
def outer():
    x: int
    def f():
        return (x for y in [x for x in [1]])
    return next(f())
try:
    print(outer())
except NameError:
    print('unbound')
",
            "unbound\n",
        ),
        (
            r"x = 'global'
def outer():
    [(x := i) for i in [1]]
    def f():
        return (x for y in [x for x in [1]])
    return next(f())
try:
    print(outer())
except NameError:
    print('unbound')
",
            "unbound\n",
        ),
        (
            r"x = 'global'
def outer():
    x = 'outer'
    def middle():
        nonlocal x
        def f():
            return (x for y in [x for x in [1]])
        return next(f())
    return middle()
try:
    print(outer())
except NameError:
    print('unbound')
",
            "unbound\n",
        ),
        (
            r"x = 'global'
def outer():
    x = 'outer'
    def middle():
        global x
        def f():
            return (x for y in [x for x in [1]])
        return next(f())
    return middle()
print(outer())
",
            "global\n",
        ),
        (
            r"x = 'global'
def outer():
    [x for x in [2]]
    def f():
        return (x for y in [x for x in [1]])
    return next(f())
print(outer())
",
            "global\n",
        ),
        (
            r"x = 'global'
def outer():
    def f():
        return (x for y in [(lambda: x) for x in [1]])
    return next(f())
print(outer())
",
            "global\n",
        ),
        (
            r"x = 'global'
z = 'also global'
def outer():
    def f():
        first = 0
        return ((x, z, first) for y in [(x, z) for x, z in [(1, 2)]])
    return next(f())
print(outer())
",
            "('global', 'also global', 0)\n",
        ),
        (
            r"x = 'global'
def outer():
    x = 'outer'
    class C:
        global x
        def f(self):
            return (x for y in [x for x in [1]])
    return next(C().f())
try:
    print(outer())
except NameError:
    print('unbound')
",
            "unbound\n",
        ),
        (
            r"_C__x = 'global'
class C:
    def outer(self):
        def f():
            return (__x for y in [__x for __x in [1]])
        return next(f())
print(C().outer())
",
            "global\n",
        ),
        (
            r"_C__x = 'global'
class C:
    def outer(self):
        def f():
            return (__x for y in [_C__x for __x in [1]])
        return next(f())
print(C().outer())
",
            "global\n",
        ),
    ];
    for (source, expected) in cases {
        common::assert_executes(source, expected);
        common::assert_matches_cpython_marshal(source, "nested_generator_scope.py");
    }
}

#[test]
fn nested_generator_global_matches_other_cpython_targets() {
    use serc::{CompileOptions, PythonVersion, compile_with_options};

    let oracle = r"
import importlib.util, marshal, sys
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]
code = compile(sys.stdin.read(), sys.argv[3], 'exec', dont_inherit=True, optimize=0)
sys.stdout.buffer.write(marshal.dumps(code))
";
    let filename = "nested_generator_scope.py";
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py315,
    ] {
        let target = version.target();
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for source in [
            "def outer():\n    def f():\n        return (x for y in [x for x in [1]])\n",
            "def outer():\n    x = 1\n    def f():\n        return (x for y in [x for x in [1]])\n",
            "def outer():\n    def f():\n        return (x for y in [(lambda: x) for x in [1]])\n",
        ] {
            let expected = crate::common::cpython::run_with_input(
                crate::common::cpython::command(version)
                    .args(["-I", "-S", "-X", "utf8", "-c", oracle])
                    .args([
                        target.tag.strip_prefix('v').unwrap(),
                        &target.magic_hex(),
                        filename,
                    ]),
                source.as_bytes(),
            );
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

#[test]
fn matches_cpython_external_closures_in_comprehension_assignment_targets() {
    let source = r#"
def collections(key, values):
    holder = {}
    lists = [key for holder[(lambda: key)()] in values]
    sets = {key for holder[(lambda: key)()] in values}
    dictionaries = {key: key for holder[(lambda: key)()] in values}
    generators = tuple(key for holder[(lambda: key)()] in values)
    direct_cells = [(lambda: item)() for item in values]
    return lists, sets, dictionaries, generators, direct_cells, holder

print(collections("index", [10, 20]))
"#;
    common::assert_matches_cpython_marshal(source, "comprehension_assignment_closures.py");
    common::assert_executes(
        source,
        "(['index', 'index'], {'index'}, {'index': 'index'}, ('index', 'index'), [10, 20], {'index': 20})\n",
    );
}

#[test]
fn matches_cpython_child_globals_outside_comprehension_targets() {
    for kind in ["list", "set", "dict"] {
        for element in ["item", "(lambda: item)"] {
            for child in [
                "child = lambda: item\n",
                "def child():\n    return item\n",
                "class child:\n    @item\n    def method(self):\n        pass\n",
            ] {
                for prefix in ["", "global item\n"] {
                    for before in [false, true] {
                        let expression =
                            format!("result = {}\n", comprehension(kind, element, "item"));
                        let body = if before {
                            format!("{prefix}{expression}{child}return child\n")
                        } else {
                            format!("{prefix}{child}{expression}return child\n")
                        };
                        let source = format!("def outer(values, parts):\n{}", indent(&body));
                        common::assert_matches_cpython_marshal(&source, "comprehension_globals.py");
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_captured_comprehension_cells_in_module_headers() {
    for kind in ["list", "set", "dict"] {
        for element in ["(lambda: item)", "tuple(item for part in parts)"] {
            let expression = comprehension(kind, element, "item");
            for header in [
                "@decorate(EXPR)\ndef f():\n    pass\n",
                "def f(value=EXPR):\n    pass\n",
                "def f(*, value=EXPR):\n    pass\n",
                "@decorate(EXPR)\nclass C:\n    pass\n",
                "class C(*EXPR):\n    pass\n",
                "class C(metaclass=make(EXPR)):\n    pass\n",
            ] {
                let source = format!(
                    "{}external = lambda: item\n",
                    header.replace("EXPR", &expression)
                );
                common::assert_matches_cpython_marshal(&source, "module_header_cells.py");
            }
        }
    }
}

#[test]
fn matches_cpython_generator_comprehension_fast_cell_order() {
    for kind in ["list", "set", "dict"] {
        for (outer, target) in [("aa", "zz"), ("zz", "aa")] {
            for capture_outer in [false, true] {
                for lambda in [false, true] {
                    for position in ["element", "filter", "later_iter", "first_iter"] {
                        let value = if capture_outer && position != "first_iter" {
                            format!("({outer}, {target})")
                        } else {
                            target.to_string()
                        };
                        let element = if lambda {
                            format!("(lambda: {value})")
                        } else {
                            format!("tuple({value} for part in parts)")
                        };
                        let expression = comprehension(kind, &element, target);
                        let generator = match position {
                            "element" => format!("({expression} for {outer} in values)"),
                            "filter" => format!("({outer} for {outer} in values if {expression})"),
                            "later_iter" => {
                                format!("({outer} for {outer} in values for tail in {expression})")
                            }
                            "first_iter" => format!("({outer} for {outer} in {expression})"),
                            _ => unreachable!(),
                        };
                        let source = format!("def f(values, parts):\n    return {generator}\n");
                        common::assert_matches_cpython_marshal(&source, "generator_fast_cells.py");
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_nested_comprehension_target_restoration() {
    for kind in ["list", "set", "dict"] {
        for inner in [
            "[item for item in parts]",
            "[lambda: item for item in parts]",
        ] {
            let expression = comprehension(kind, &format!("({inner}, lambda: item)"), "item");
            for template in [
                "result = EXPR\n",
                "def function(result=EXPR):\n    return result\n",
                "def function(values, parts):\n    return EXPR\n",
                "function = lambda: EXPR\n",
            ] {
                common::assert_matches_cpython_marshal(
                    &template.replace("EXPR", &expression),
                    "nested_target_restoration.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_outer_comprehension_reads_shadowed_only_by_a_child() {
    for expression in [
        "[(item, [item for item in parts]) for outer in values]",
        "[[item for item in parts] for outer in values if item]",
        "[item for outer in values if [item for item in parts]]",
        "[[item for item in parts] for holder[item] in values]",
        "[item for holder[[item for item in parts][0]] in values]",
        "{item for outer in values if {item for item in parts}}",
        "{item: outer for outer in values if {item: item for item in parts}}",
        "[[item for item in parts] for outer in values]",
        "[(item, [item for item in parts]) for item in values]",
    ] {
        for body in [
            format!("result = {expression}\n"),
            format!("class Class:\n    result = {expression}\n"),
            format!("def function(values, parts, holder):\n    return {expression}\n"),
            format!(
                "def function(values, parts, holder):\n    item = 2\n    return {expression}\n"
            ),
            format!(
                "def function(values, parts, holder):\n    global item\n    return {expression}\n"
            ),
            format!("def function(values, parts, holder, item):\n    return {expression}\n"),
            format!(
                "def outer(item):\n    def function(values, parts, holder):\n        return {expression}\n    return function\n"
            ),
        ] {
            common::assert_matches_cpython_marshal(&body, "outer_comprehension_read.py");
        }
    }

    let source = r"
item = 0
def function(values, parts, holder):
    one = [(item, [item for item in parts]) for outer in values]
    two = [[item for item in parts] for holder[item] in values]
    three = [item for holder[[item for item in parts][0]] in values]
    return one, two, three, holder
print(function([1, 2], [1, 2], [0, 0, 0]))
";
    common::assert_executes(
        source,
        "([(0, [1, 2]), (0, [1, 2])], [[1, 2], [1, 2]], [0, 0], [2, 2, 0])\n",
    );
}

#[test]
fn matches_cpython_comprehension_temporaries_in_type_parameter_thunks() {
    for kind in ["list", "set", "dict"] {
        for element in ["item", "(lambda: item)"] {
            let expression = comprehension(kind, element, "item");
            let inner = comprehension(kind, "inner", "inner");
            for expression in [
                expression.replace("in values", &format!("in {inner}")),
                comprehension(kind, &format!("({expression}, lambda: outer)"), "outer"),
                comprehension(kind, &format!("({expression}, lambda: item)"), "item"),
                format!("(item, {expression})"),
                format!("({expression}, item)"),
                expression.replace("in values", "in item"),
            ] {
                for template in [
                    "def function[T: EXPR]():\n    pass\n",
                    "def function[T = EXPR]():\n    pass\n",
                    "class Class[T: EXPR]:\n    pass\n",
                    "class Class[T = EXPR]:\n    pass\n",
                ] {
                    common::assert_matches_cpython_marshal(
                        &template.replace("EXPR", &expression),
                        "type_parameter_comprehension.py",
                    );
                }
            }
        }
    }
}
