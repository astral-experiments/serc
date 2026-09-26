use crate::code_object::{CodeObject, Constant};
use crate::target::code_flags::{CO_GENERATOR, CO_HAS_DOCSTRING, CO_METHOD};
use crate::target::opcodes::{MAKE_FUNCTION, SET_FUNCTION_ATTRIBUTE};
use crate::{CompileOptions, PythonVersion, compile_with_options};

fn compile(source: &str, python_version: PythonVersion) -> CodeObject {
    compile_with_options(
        source,
        "legacy_annotations.py",
        &CompileOptions {
            python_version,
            ..CompileOptions::default()
        },
    )
    .unwrap()
    .code
}

fn child<'a>(code: &'a CodeObject, name: &str) -> &'a CodeObject {
    code.constants
        .iter()
        .find_map(|constant| match constant {
            Constant::Code(child) if child.name == name => Some(child.as_ref()),
            _ => None,
        })
        .unwrap()
}

#[test]
fn legacy_ast_folds_boolean_inversion_inside_literal_subscripts() {
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        for (expression, expected) in [
            (r#""abc"[~False]"#, "'c'"),
            (r#""abc"[~True]"#, "'b'"),
            (r#"b"abc"[~False]"#, "99"),
            ("(10, 20, 30)[~False]", "30"),
            ("(10, 20, 30)[-~False]", "20"),
            (r#""abc"[~(True & False)]"#, "'c'"),
            (r#""abc"[(1, ~False)[1]]"#, "'c'"),
        ] {
            assert_folded_expression(version, expression, expected);
        }
    }
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        let actual = compile(r#"result = "abc"[~False]"#, version);
        let folded = compile("result = 'c'", version);
        assert_ne!(actual.bytecode, folded.bytecode, "{version:?}");
    }
}

#[test]
fn legacy_ast_folds_logical_not_after_folding_its_operand() {
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        for (expression, expected) in [
            ("not (1 + 2)", "False"),
            ("not (-1)", "False"),
            ("not ~0", "False"),
            ("not +True", "False"),
            ("not ~False", "False"),
            ("not (~False + 1)", "True"),
            ("not not ~True", "True"),
            (r#"not "abc"[~False]"#, "False"),
            ("not (True,)[~False]", "False"),
        ] {
            assert_folded_expression(version, expression, expected);
        }
    }
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        for (expression, expected) in [("not (1 + 2)", "False"), ("not ~False", "False")] {
            let actual = compile(&format!("result = {expression}"), version);
            let folded = compile(&format!("result = {expected}"), version);
            assert_ne!(
                actual.bytecode, folded.bytecode,
                "{version:?}: {expression}"
            );
        }
    }
}

fn assert_folded_expression(version: PythonVersion, expression: &str, expected: &str) {
    let options = [
        (
            format!("result = {expression}"),
            format!("result = {expected}"),
            false,
        ),
        (
            format!("def f():\n    return {expression}"),
            format!("def f():\n    return {expected}"),
            true,
        ),
    ];
    for (actual, expected, function) in options {
        let actual = compile(&actual, version);
        let expected = compile(&expected, version);
        let (actual, expected) = if function {
            (child(&actual, "f"), child(&expected, "f"))
        } else {
            (&actual, &expected)
        };
        assert_eq!(
            actual.bytecode, expected.bytecode,
            "{version:?}: {expression}"
        );
        assert_eq!(actual.constants.len(), expected.constants.len());
        assert!(
            actual
                .constants
                .iter()
                .zip(&expected.constants)
                .all(|(actual, expected)| {
                    super::constant::python_constant_values_equal(actual, expected)
                }),
            "{version:?}: {expression}"
        );
    }
}

#[test]
fn legacy_class_comprehensions_reserve_nested_free_captures() {
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        for (source, expected) in [
            (
                "def outer(offset):\n    class C:\n        values = [lambda: offset for x in data]\n    return C\n",
                &[".0", "x", "offset"][..],
            ),
            (
                "def outer(offset):\n    class C:\n        values = [(offset for y in more) for x in data]\n    return C\n",
                &[".0", "x", "more", "offset"][..],
            ),
            (
                "def outer(offset):\n    class C:\n        values = [lambda offset: offset for x in data]\n    return C\n",
                &[".0", "x"][..],
            ),
            (
                "def outer(offset):\n    class C:\n        values = [(offset for offset in more) for x in data]\n    return C\n",
                &[".0", "x", "more"][..],
            ),
        ] {
            let module = compile(source, version);
            let class = child(child(&module, "outer"), "C");
            let fast_names = class
                .locals
                .iter()
                .zip(&class.local_kinds)
                .filter(|(_, kind)| **kind & crate::target::local_kinds::CO_FAST_LOCAL != 0)
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>();
            assert_eq!(fast_names, expected, "{version:?}: {source}");
        }
    }
}

#[test]
fn legacy_not_comparison_exits_use_the_folded_inner_span() {
    for (version, lambda_lines, assertion_lines) in [
        (
            PythonVersion::Py312,
            &[128, 0, 168, 49, 176, 1, 169, 54][..],
            &[
                128, 0, 216, 14, 15, 136, 105, 216, 19, 20, 152, 2, 145, 55, 208, 8, 26, 144, 55,
                216, 19, 20, 144, 57, 216, 12, 17, 224, 11, 12, 128, 72, 244, 3, 0, 13, 17, 144,
                18, 139, 72, 136, 1, 240, 9, 0, 15, 16, 137, 105, 240, 10, 0, 12, 13, 128, 72,
            ][..],
        ),
        (
            PythonVersion::Py313,
            &[128, 0, 168, 49, 170, 54][..],
            &[
                128, 0, 216, 14, 15, 137, 105, 216, 19, 20, 147, 55, 208, 8, 26, 144, 55, 216, 19,
                20, 145, 57, 216, 12, 17, 224, 11, 12, 128, 72, 244, 3, 0, 13, 17, 144, 18, 139,
                72, 136, 1, 240, 9, 0, 15, 16, 139, 105, 240, 10, 0, 12, 13, 128, 72,
            ][..],
        ),
    ] {
        let module = compile(
            "type Alias = (not A is B, lambda x, y: not x in y)\n",
            version,
        );
        assert_eq!(
            child(child(&module, "Alias"), "<lambda>").line_table,
            lambda_lines,
            "{version:?}"
        );
        let module = compile(
            "def f(x, xs):\n    while not x is None:\n        assert not x in xs\n        if not not x is None:\n            break\n        x = next(xs)\n    return x\n",
            version,
        );
        assert_eq!(
            child(&module, "f").line_table,
            assertion_lines,
            "{version:?}"
        );
    }
}

#[test]
fn legacy_not_comparisons_match_native_opcode_polarity_and_inner_locations() {
    // Captured from the pinned CPython 3.12.14 and 3.13.15 compilers.
    for (version, source, bytecode, line_table) in [
        (
            PythonVersion::Py312,
            "def f(x, y):\n    return (not x is y, not x is not y, not x in y, not x not in y)\n",
            &[
                151, 0, 124, 0, 124, 1, 117, 1, 124, 0, 124, 1, 117, 0, 124, 0, 124, 1, 118, 1,
                124, 0, 124, 1, 118, 0, 102, 4, 83, 0,
            ][..],
            &[
                128, 0, 216, 16, 17, 144, 81, 144, 6, 152, 65, 160, 81, 152, 74, 168, 65, 176, 17,
                168, 70, 184, 1, 192, 17, 184, 10, 208, 11, 67, 208, 4, 67,
            ][..],
        ),
        (
            PythonVersion::Py312,
            "def f(x, y):\n    if not (\n        x\n        in y\n    ):\n        return not (\n            x is\n            y\n        )\n    return not not (\n        x is not None\n    )\n",
            &[
                151, 0, 124, 0, 124, 1, 118, 1, 114, 4, 124, 0, 124, 1, 117, 1, 83, 0, 124, 0, 100,
                0, 117, 1, 83, 0,
            ][..],
            &[
                128, 0, 224, 8, 9, 216, 11, 12, 241, 3, 1, 9, 13, 240, 8, 0, 13, 14, 216, 12, 13,
                240, 3, 1, 13, 14, 240, 3, 3, 9, 10, 240, 10, 0, 9, 10, 144, 20, 136, 13, 240, 3,
                2, 5, 6,
            ][..],
        ),
        (
            PythonVersion::Py313,
            "def f(x, y):\n    return (not x is y, not x is not y, not x in y, not x not in y)\n",
            &[
                149, 0, 88, 1, 76, 1, 88, 1, 76, 0, 88, 1, 59, 1, 0, 0, 88, 1, 59, 0, 0, 0, 52, 4,
                36, 0,
            ][..],
            &[
                128, 0, 216, 16, 17, 144, 6, 152, 65, 152, 74, 168, 65, 169, 70, 184, 1, 185, 10,
                208, 11, 67, 208, 4, 67,
            ][..],
        ),
        (
            PythonVersion::Py313,
            "def f(x, y):\n    if not (\n        x\n        in y\n    ):\n        return not (\n            x is\n            y\n        )\n    return not not (\n        x is not None\n    )\n",
            &[
                149, 0, 85, 0, 85, 1, 59, 1, 0, 0, 97, 4, 0, 0, 85, 0, 85, 1, 76, 1, 36, 0, 85, 0,
                83, 0, 76, 1, 36, 0,
            ][..],
            &[
                128, 0, 224, 8, 9, 216, 11, 12, 243, 3, 1, 9, 13, 240, 8, 0, 13, 14, 216, 12, 13,
                240, 3, 1, 13, 14, 240, 3, 3, 9, 10, 240, 10, 0, 9, 10, 144, 20, 136, 13, 240, 3,
                2, 5, 6,
            ][..],
        ),
    ] {
        let module = compile(source, version);
        let function = child(&module, "f");
        assert_eq!(function.bytecode, bytecode, "{version:?}: {source}");
        assert_eq!(function.line_table, line_table, "{version:?}: {source}");
    }
}

#[test]
fn legacy_unreachable_module_annotations_retain_names_without_class_scopes() {
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        let module = compile("raise Exception\nx: \"T\"\nafter\n", version);
        assert_eq!(module.names, ["Exception", "__annotations__", "after"]);
        assert!(matches!(module.constants.as_slice(), [Constant::String(value)] if value == "T"));
        for source in [
            "raise Exception\nobj.x: \"T\"\n",
            "raise Exception\nobj[1]: \"T\"\n",
        ] {
            assert_eq!(compile(source, version).names, ["Exception", "obj"]);
        }
    }
}

#[test]
fn legacy_function_payloads_match_native_attribute_order_and_admission() {
    let source = "def outer():\n    x = 1\n    def f(a: A = 2, *, b: B = 3) -> R:\n        return x\n    return f\n";
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        let module = compile(source, version);
        let outer = child(&module, "outer");
        // Captured from the pinned 3.12.14 and 3.13.15 compilers. The defaults
        // tuple is created by the CFG optimizer after the body code object.
        assert!(
            matches!(
                &outer.constants[..7],
                [Constant::None, Constant::Int(1), Constant::Int(3), Constant::Tuple(keys), Constant::String(a), Constant::String(b), Constant::String(ret)]
                    if matches!(keys.as_slice(), [Constant::String(key)] if key == "b")
                        && a == "a" && b == "b" && ret == "return"
            ),
            "{version:?}: {:?}",
            outer.constants
        );
        assert!(
            matches!(outer.constants.last(), Some(Constant::Tuple(values)) if matches!(values.as_slice(), [Constant::Int(2)]))
        );
        let make_args = outer
            .bytecode
            .as_chunks::<2>()
            .0
            .iter()
            .filter(|instruction| instruction[0] == MAKE_FUNCTION.code_for(version))
            .map(|instruction| instruction[1])
            .collect::<Vec<_>>();
        assert_eq!(
            make_args,
            [if version == PythonVersion::Py312 {
                15
            } else {
                0
            }]
        );
        if version == PythonVersion::Py313 {
            let attributes = outer
                .bytecode
                .as_chunks::<2>()
                .0
                .iter()
                .filter(|instruction| instruction[0] == SET_FUNCTION_ATTRIBUTE.code_for(version))
                .map(|instruction| instruction[1])
                .collect::<Vec<_>>();
            assert_eq!(attributes, [8, 4, 2, 1]);
        }
        assert!(outer.constants.iter().all(
            |constant| !matches!(constant, Constant::Code(code) if code.name == "__annotate__")
        ));
    }
}

#[test]
fn legacy_class_annotations_omit_newer_metadata_and_flags() {
    let source = "class C:\n    x: int\n    def f(self, value: int):\n        \"docstring\"\n        return value\n";
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        let module = compile(source, version);
        let class = child(&module, "C");
        assert!(class.names.iter().any(|name| name == "__annotations__"));
        assert!(!class.names.iter().any(|name| name == "__annotate_func__"));
        for name in ["__firstlineno__", "__static_attributes__"] {
            assert_eq!(
                class.names.iter().any(|candidate| candidate == name),
                version == PythonVersion::Py313
            );
        }
        let method = child(class, "f");
        assert_eq!(method.flags & (CO_METHOD | CO_HAS_DOCSTRING), 0);
        assert_eq!(method.local_kinds, [0x20, 0x20]);
        assert!(
            matches!(method.constants.first(), Some(Constant::String(value)) if value == "docstring")
        );
    }
}

#[test]
fn legacy_type_alias_evaluators_have_no_format_argument() {
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        let module = compile("type Alias = int\n", version);
        let alias = child(&module, "Alias");
        assert_eq!(alias.arg_count, 0);
        assert!(alias.locals.is_empty());
        assert!(matches!(alias.constants.first(), Some(Constant::None)));
    }
}

#[test]
fn eager_generic_signatures_only_capture_type_parameters_in_nested_functions() {
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        let module = compile(
            "def direct[T](x: T): pass\ndef nested[T](x: lambda: T): pass\n",
            version,
        );
        let direct = child(&module, "<generic parameters of direct>");
        assert_eq!(direct.locals, [".defaults", "T"]);
        assert_eq!(direct.local_kinds, [0x20, 0x20]);
        let nested = child(&module, "<generic parameters of nested>");
        assert_eq!(nested.locals, [".defaults", "T"]);
        assert_eq!(nested.local_kinds, [0x20, 0x40]);
        assert_eq!(child(nested, "<lambda>").qualified_name, "<lambda>");
    }
}

#[test]
fn legacy_constant_returns_have_version_specific_stack_minima() {
    for (version, expected) in [(PythonVersion::Py312, 0), (PythonVersion::Py313, 1)] {
        let module = compile("def f(): return 1\n", version);
        assert_eq!(child(&module, "f").stack_size, expected);
        let module = compile("async def f(): return 1\n", version);
        assert_eq!(child(&module, "f").stack_size, 2);
    }
}

#[test]
fn python_312_type_parameter_helpers_use_the_parameter_span() {
    let module = compile("type B[T: int] = list[T]\n", PythonVersion::Py312);
    let wrapper = child(&module, "<generic parameters of B>");
    // Exact location tables from CPython 3.12.14. The helper's expression uses
    // `int`'s span; its RETURN_VALUE and creation use the enclosing `T: int`.
    assert_eq!(
        wrapper.line_table,
        [248, 128, 0, 208, 0, 24, 134, 118, 214, 0, 24]
    );
    assert_eq!(child(wrapper, "T").line_table, [128, 0, 140, 35, 128, 118]);
}

#[test]
fn python_312_generator_entry_pop_has_no_source_location() {
    let module = compile("def f():\n    yield 1\n", PythonVersion::Py312);
    // RETURN_GENERATOR has a line-only location, POP_TOP has none, and RESUME
    // starts at columns 0:0, as in the pinned 3.12.14 compiler.
    assert!(
        child(&module, "f")
            .line_table
            .starts_with(&[232, 0, 248, 128, 0])
    );
}

#[test]
fn python_312_function_body_nops_do_not_share_the_resume_block() {
    for (source, expected) in [
        ("def f(): pass\n", &[128, 0, 136, 20][..]),
        ("def f(): x = 1; pass\n", &[128, 0, 136, 81, 137, 17][..]),
    ] {
        assert_eq!(
            child(&compile(source, PythonVersion::Py312), "f").line_table,
            expected
        );
    }
}

#[test]
fn eager_generic_annotation_children_keep_the_lexical_scope_of_global_definitions() {
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        let module = compile(
            "def outer():\n    global f\n    def f[T](x: lambda: T): pass\n",
            version,
        );
        let wrapper = child(child(&module, "outer"), "<generic parameters of f>");
        assert_eq!(child(wrapper, "f").qualified_name, "f");
        assert_eq!(
            child(wrapper, "<lambda>").qualified_name,
            "outer.<locals>.<lambda>"
        );
    }
}

#[test]
fn legacy_defaults_fold_in_order_with_future_annotation_tuples() {
    let source = "from __future__ import annotations\ndef f(a: A) -> B:\n    pass\ndef g(\n    b: C = None,\n) -> D:\n    pass\n";
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        let module = compile(source, version);
        assert!(
            matches!(&module.constants[5..], [Constant::Tuple(first), Constant::Tuple(defaults), Constant::Tuple(second)]
                if matches!(first.as_slice(), [Constant::String(name), ..] if name == "a")
                    && matches!(defaults.as_slice(), [Constant::None])
                    && matches!(second.as_slice(), [Constant::String(name), ..] if name == "b")
            )
        );
    }
}

#[test]
fn legacy_synthetic_tuples_follow_native_cfg_admission_order() {
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        let module = compile(
            "from __future__ import annotations\ndef outer():\n    def annotated(value: Thing): pass\n    callback = lambda item=None: item\n    return callback\n",
            version,
        );
        assert!(matches!(&child(&module, "outer").constants[3..],
            [Constant::Tuple(annotation), Constant::Tuple(defaults)]
                if matches!(annotation.as_slice(), [Constant::String(name), Constant::String(value)] if name == "value" && value == "Thing")
                    && matches!(defaults.as_slice(), [Constant::None])));

        let module = compile(
            "from __future__ import annotations\ndef annotated(value: Thing): pass\nresult = call(**options)\n",
            version,
        );
        assert!(matches!(&module.constants[4..],
            [Constant::Tuple(annotation), Constant::Tuple(arguments)]
                if annotation.len() == 2 && arguments.is_empty()));

        for arguments in ["'first'", "'first', 'second'"] {
            let module = compile(
                &format!("def f():\n    return call({arguments}, **options, keyword='later')\n"),
                version,
            );
            assert!(matches!(&child(&module, "f").constants[..],
                [Constant::None, Constant::String(key), Constant::String(value), Constant::Tuple(_)]
                    if key == "keyword" && value == "later"));
        }
    }
}

#[test]
fn legacy_class_qualified_names_remember_discarded_cache_admissions() {
    for (prefix, expected) in [
        ("", [true, true]),
        ("if False:\n    earlier = 'Outer.Inner'\n", [false, false]),
        (
            "if False:\n    earlier = 'Outer.' + 'Inner'\n",
            [false, false],
        ),
        ("if False:\n    earlier: 'Outer.Inner'\n", [false, false]),
        (
            "if False:\n    class Outer:\n        class Inner: pass\n",
            [false, true],
        ),
    ] {
        let source = format!("{prefix}class Outer:\n    class Inner: pass\n");
        for (version, expected) in [PythonVersion::Py312, PythonVersion::Py313]
            .into_iter()
            .zip(expected)
        {
            let module = compile(&source, version);
            assert_eq!(
                child(child(&module, "Outer"), "Inner")
                    .marshal_provenance
                    .qualified_name_is_canonical,
                Some(expected),
                "{version:?}: {source}"
            );
        }
    }
}

#[test]
fn legacy_qualified_names_distinguish_metadata_tuple_hits_from_scalar_admissions() {
    let seed = "def seed():\n    import Outer.Inner\n";
    let value = "value = ('Outer.Inner',)\n";
    let class = "class Outer:\n    class Inner: pass\n";
    for (source, expected) in [
        (format!("{value}{class}"), [false, false]),
        (format!("{seed}{value}{class}"), [true, false]),
        (format!("{value}{seed}{class}"), [false, false]),
        (format!("{seed}{class}"), [true, false]),
        (format!("{class}{seed}"), [true, false]),
    ] {
        for (version, expected) in [PythonVersion::Py312, PythonVersion::Py313]
            .into_iter()
            .zip(expected)
        {
            let module = compile(&source, version);
            // The composed class constant is a fresh string, even when an import
            // supplied an equal parser-interned spelling for its code metadata.
            assert!(
                !module
                    .marshal_provenance
                    .canonical_constant_strings
                    .contains("Outer.Inner"),
                "{version:?}: {source}"
            );
            assert_eq!(
                child(child(&module, "Outer"), "Inner")
                    .marshal_provenance
                    .qualified_name_is_canonical,
                Some(expected),
                "{version:?}: {source}"
            );
        }
    }
}

#[test]
fn legacy_loop_control_locations_match_native() {
    // Exact line tables from the pinned 3.12.14 and 3.13.15 compilers.
    for (version, source, function, expected) in [
        (
            PythonVersion::Py312,
            "def f(xs):\n for x in xs:\n  try: break\n  finally: work(x)\n",
            "f",
            &[
                128, 0, 219, 10, 12, 128, 81, 216, 7, 12, 220, 11, 15, 144, 1, 142, 55, 241, 5, 0,
                11, 13, 248, 228, 11, 15, 144, 1, 141, 55, 250,
            ][..],
        ),
        (
            PythonVersion::Py313,
            "async def f(xs):\n async for x in xs:\n  if x: break\n",
            "f",
            &[
                233, 0, 128, 0, 217, 16, 18, 247, 0, 1, 2, 14, 136, 49, 223, 5, 6, 128, 81, 137, 5,
                241, 3, 1, 2, 14, 145, 2, 249,
            ][..],
        ),
        (
            PythonVersion::Py313,
            "def visit_present(values, visit):\n    for value in values:\n        if value is not None:\n            visit(value)\n",
            "visit_present",
            &[
                128, 0, 219, 17, 23, 136, 5, 216, 11, 16, 211, 11, 28, 217, 12, 17, 144, 37, 142,
                76, 242, 5, 0, 18, 24,
            ][..],
        ),
    ] {
        assert_eq!(
            child(&compile(source, version), function).line_table,
            expected,
            "{version:?}: {source}"
        );
    }
}

#[test]
fn legacy_annotation_yields_belong_to_the_eager_enclosing_scope() {
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        for (future, signature, is_generator) in [
            (false, "(x: (yield 1))", true),
            (false, "() -> (yield from 1)", true),
            (false, "(x: (lambda: (yield 1)))", false),
            (true, "(x: (lambda: (yield 1)))", false),
            (false, "(x=(yield 1))", true),
            (true, "(x=(yield 1))", true),
        ] {
            let prefix = if future {
                "from __future__ import annotations\n"
            } else {
                ""
            };
            let source = format!("{prefix}def outer():\n    def inner{signature}: pass\n");
            let module = compile(&source, version);
            assert_eq!(
                child(&module, "outer").flags & CO_GENERATOR != 0,
                is_generator,
                "{version:?}: {source}"
            );
            assert_eq!(
                child(child(&module, "outer"), "inner").flags & CO_GENERATOR,
                0
            );
        }
    }
}

#[test]
fn legacy_while_tests_and_implicit_exits_match_native() {
    // Exact bytecode and line tables from the pinned 3.12.14 and 3.13.15 compilers.
    for (version, source, bytecode, line_table) in [
        (
            PythonVersion::Py312,
            "def f(value):\n    while value:\n        value -= 1\n",
            &[
                151, 0, 124, 0, 114, 9, 124, 0, 100, 1, 122, 23, 0, 0, 125, 0, 124, 0, 114, 1, 140,
                8, 121, 0, 121, 0,
            ][..],
            &[
                128, 0, 217, 10, 15, 216, 8, 13, 144, 17, 137, 10, 136, 5, 244, 3, 0, 11, 16,
            ][..],
        ),
        (
            PythonVersion::Py312,
            "def f(left, right):\n    while left and right:\n        left -= 1\n",
            &[
                151, 0, 124, 0, 114, 15, 124, 1, 114, 12, 124, 0, 100, 1, 122, 23, 0, 0, 125, 0,
                124, 0, 114, 4, 124, 1, 114, 1, 140, 10, 121, 0, 121, 0, 121, 0, 121, 0,
            ][..],
            &[
                128, 0, 217, 10, 14, 145, 53, 216, 8, 12, 144, 1, 137, 9, 136, 4, 241, 3, 0, 11,
                15, 147, 53, 136, 36, 144, 53, 136, 36,
            ][..],
        ),
        (
            PythonVersion::Py313,
            "def f(value):\n    while value:\n        value -= 1\n",
            &[
                149, 0, 85, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 15, 0, 0, 85, 0, 83, 1, 45, 23, 0, 0,
                110, 0, 85, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 2, 0, 0, 77, 14, 0, 0, 103, 0, 103, 0,
            ][..],
            &[
                128, 0, 222, 10, 15, 216, 8, 13, 144, 17, 137, 10, 136, 5, 247, 3, 0, 11, 16, 138,
                37,
            ][..],
        ),
        (
            PythonVersion::Py313,
            "def f(left, right):\n    while left and right:\n        left -= 1\n",
            &[
                149, 0, 85, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 31, 0, 0, 85, 1, 40, 0, 0, 0, 0, 0, 0,
                0, 97, 23, 0, 0, 85, 0, 83, 1, 45, 23, 0, 0, 110, 0, 85, 0, 40, 0, 0, 0, 0, 0, 0,
                0, 97, 10, 0, 0, 85, 1, 40, 0, 0, 0, 0, 0, 0, 0, 97, 2, 0, 0, 77, 21, 0, 0, 103, 0,
                103, 0, 103, 0, 103, 0,
            ][..],
            &[
                128, 0, 222, 10, 14, 150, 53, 216, 8, 12, 144, 1, 137, 9, 136, 4, 246, 3, 0, 11,
                15, 151, 53, 145, 53, 136, 36, 144, 53, 136, 36,
            ][..],
        ),
    ] {
        let module = compile(source, version);
        let function = child(&module, "f");
        assert_eq!(function.bytecode, bytecode, "{version:?}: {source}");
        assert_eq!(function.line_table, line_table, "{version:?}: {source}");
    }
}

#[test]
fn legacy_dynamic_while_breaks_retest_before_the_backedge() {
    // Exact native 3.12.14 output; both return copies and protected spans matter.
    for (source, bytecode, line_table, exception_table) in [
        (
            "def f(left, right, items):\n    while left and right:\n        try:\n            item = next(items)\n        except StopIteration:\n            break\n        work(item)\n",
            &[
                151, 0, 124, 0, 114, 33, 124, 1, 114, 30, 9, 0, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0,
                124, 2, 171, 1, 0, 0, 0, 0, 0, 0, 125, 3, 116, 5, 0, 0, 0, 0, 0, 0, 0, 0, 124, 3,
                171, 1, 0, 0, 0, 0, 0, 0, 1, 0, 124, 0, 114, 4, 124, 1, 114, 1, 140, 28, 121, 0,
                121, 0, 121, 0, 121, 0, 35, 0, 116, 2, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 3, 1, 0,
                89, 0, 121, 0, 119, 0, 120, 3, 89, 0, 119, 1,
            ][..],
            &[
                128, 0, 217, 10, 14, 145, 53, 240, 2, 3, 9, 18, 220, 19, 23, 152, 5, 147, 59, 136,
                68, 244, 6, 0, 9, 13, 136, 84, 140, 10, 241, 11, 0, 11, 15, 147, 53, 136, 36, 144,
                53, 136, 36, 248, 244, 6, 0, 16, 29, 242, 0, 1, 9, 18, 217, 12, 17, 240, 3, 1, 9,
                18, 250,
            ][..],
            &[134, 11, 37, 0, 165, 9, 49, 3, 176, 1, 49, 3][..],
        ),
        (
            "def f(value):\n    while value:\n        value -= 1\n        if value == 2:\n            break\n",
            &[
                151, 0, 124, 0, 114, 15, 124, 0, 100, 1, 122, 23, 0, 0, 125, 0, 124, 0, 100, 2,
                107, 40, 0, 0, 114, 1, 121, 0, 124, 0, 114, 1, 140, 14, 121, 0, 121, 0,
            ][..],
            &[
                128, 0, 217, 10, 15, 216, 8, 13, 144, 17, 137, 10, 136, 5, 216, 11, 16, 144, 65,
                138, 58, 216, 12, 17, 244, 7, 0, 11, 16,
            ][..],
            &[][..],
        ),
    ] {
        let module = compile(source, PythonVersion::Py312);
        let function = child(&module, "f");
        assert_eq!(function.bytecode, bytecode, "{source}");
        assert_eq!(function.line_table, line_table, "{source}");
        assert_eq!(function.exception_table, exception_table, "{source}");
    }
}

#[test]
fn legacy_class_comprehensions_save_the_iterator_parameter_and_external_symbols() {
    for (source, expected) in [
        (
            "class C:\n    value = [x for x in data]\n",
            &[".0", "x"][..],
        ),
        (
            "class C:\n    value = [x + offset for x in data]\n",
            &[".0", "x", "offset"][..],
        ),
        (
            "class C:\n    value = [x for x in [y for y in data]]\n",
            &[".0", "y", "x"][..],
        ),
    ] {
        for version in [PythonVersion::Py312, PythonVersion::Py313] {
            let module = compile(source, version);
            assert_eq!(
                child(&module, "C").locals,
                expected,
                "{version:?}: {source}"
            );
        }
    }
}

#[test]
fn legacy_match_success_copy_counts_its_original_jump() {
    // Pinned CPython 3.12.14/3.13.15: the copied case-success JUMP still
    // occupies a NOP slot before OR-success edges consider the four-slot limit.
    let source = "def f(value):\n    match value:\n        case (\"a\"\n            | \"b\"\n            | \"c\"):\n            pass\n        case _:\n            if value != \"d\":\n                pass\n            elif value == \"d\":\n                pass\n";
    for (version, bytecode, line_table) in [
        (
            PythonVersion::Py312,
            &[
                151, 0, 124, 0, 120, 1, 100, 1, 107, 40, 0, 0, 114, 2, 1, 0, 121, 0, 120, 1, 100,
                2, 107, 40, 0, 0, 114, 3, 9, 0, 1, 0, 121, 0, 120, 1, 100, 3, 107, 40, 0, 0, 114,
                3, 9, 0, 1, 0, 121, 0, 1, 0, 9, 0, 124, 0, 100, 4, 107, 55, 0, 0, 114, 1, 121, 0,
                124, 0, 100, 4, 107, 40, 0, 0, 114, 1, 121, 0, 121, 0,
            ][..],
            &[
                128, 0, 216, 10, 15, 220, 14, 17, 240, 0, 2, 15, 18, 240, 6, 0, 13, 17, 245, 5, 0,
                15, 18, 240, 3, 2, 15, 18, 240, 6, 0, 13, 17, 245, 3, 0, 15, 18, 240, 5, 2, 15, 18,
                240, 6, 0, 13, 17, 240, 7, 2, 15, 18, 240, 8, 0, 14, 15, 216, 15, 20, 152, 3, 138,
                124, 216, 16, 20, 216, 17, 22, 152, 35, 146, 28, 216, 16, 20, 240, 3, 0, 18, 30,
            ][..],
        ),
        (
            PythonVersion::Py313,
            &[
                149, 0, 85, 0, 61, 1, 83, 1, 58, 88, 0, 0, 97, 1, 0, 0, 79, 16, 61, 1, 83, 2, 58,
                88, 0, 0, 97, 1, 0, 0, 79, 9, 61, 1, 83, 3, 58, 88, 0, 0, 97, 1, 0, 0, 79, 2, 32,
                0, 79, 2, 32, 0, 103, 0, 30, 0, 85, 0, 83, 4, 58, 119, 0, 0, 97, 1, 0, 0, 103, 0,
                85, 0, 83, 4, 58, 88, 0, 0, 97, 1, 0, 0, 103, 0, 103, 0,
            ][..],
            &[
                128, 0, 216, 10, 15, 222, 14, 17, 222, 14, 17, 222, 14, 17, 242, 5, 2, 15, 18, 240,
                6, 0, 13, 17, 216, 13, 14, 216, 15, 20, 152, 3, 139, 124, 216, 16, 20, 216, 17, 22,
                152, 35, 147, 28, 216, 16, 20, 240, 3, 0, 18, 30,
            ][..],
        ),
    ] {
        let module = compile(source, version);
        let function = child(&module, "f");
        assert_eq!(function.bytecode, bytecode, "{version:?}");
        assert_eq!(function.line_table, line_table, "{version:?}");
        assert!(function.exception_table.is_empty());
    }
}

#[test]
fn legacy_class_generators_propagate_only_free_references() {
    use crate::target::local_kinds::{CO_FAST_CELL, CO_FAST_FREE};

    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        for (source, expected) in [
            (
                "def outer(offset):\n    class C:\n        values = [(offset for offset in more) for x in data]\n    return C\n",
                &[][..],
            ),
            (
                "def outer(offset):\n    class C:\n        values = [(offset for y in more) for x in data]\n    return C\n",
                &["offset"][..],
            ),
        ] {
            let module = compile(source, version);
            let outer = child(&module, "outer");
            let class = child(outer, "C");
            for (code, kind) in [(outer, CO_FAST_CELL), (class, CO_FAST_FREE)] {
                let captures = code
                    .locals
                    .iter()
                    .zip(&code.local_kinds)
                    .filter(|(_, flags)| **flags & kind != 0)
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>();
                assert_eq!(captures, expected, "{version:?}: {}: {source}", code.name);
            }
        }
    }
}

#[test]
fn python_312_exception_cleanup_matches_native_copy_phases() {
    // Pinned CPython 3.12.14: shared named-handler cleanup duplicates late,
    // while except-star POP_BLOCK prevents early assertion-success threading.
    type NativeCase<'a> = (&'a str, Option<&'a str>, &'a [u8], &'a [u8], &'a [u8]);
    let cases: &[NativeCase<'_>] = &[
        (
            "def f(flag):\n    try:\n        start()\n    except Error as error:\n        if flag:\n            for value in values:\n                use(error, value)\n",
            Some("f"),
            &[
                151, 0, 9, 0, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 121,
                0, 35, 0, 116, 2, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 40, 125, 1, 124, 0, 114, 28,
                116, 4, 0, 0, 0, 0, 0, 0, 0, 0, 68, 0, 93, 14, 0, 0, 125, 2, 116, 7, 0, 0, 0, 0, 0,
                0, 0, 0, 124, 1, 124, 2, 171, 2, 0, 0, 0, 0, 0, 0, 1, 0, 140, 16, 4, 0, 89, 0, 100,
                0, 125, 1, 126, 1, 121, 0, 89, 0, 100, 0, 125, 1, 126, 1, 121, 0, 100, 0, 125, 1,
                126, 1, 119, 1, 119, 0, 120, 3, 89, 0, 119, 1,
            ],
            &[130, 10, 13, 0, 141, 9, 62, 3, 150, 25, 57, 3, 185, 5, 62, 3],
            &[
                128, 0, 240, 2, 5, 5, 34, 220, 8, 13, 141, 7, 248, 220, 11, 16, 242, 0, 3, 5, 34,
                217, 11, 15, 223, 25, 31, 144, 5, 220, 16, 19, 144, 69, 152, 53, 213, 16, 33, 245,
                3, 0, 26, 32, 244, 3, 0, 12, 16, 251, 240, 3, 3, 5, 34, 250,
            ],
        ),
        (
            "try:\n    pass\nexcept* Error:\n    assert flag\n",
            None,
            &[
                151, 0, 9, 0, 121, 0, 35, 0, 103, 0, 120, 2, 101, 0, 37, 0, 120, 1, 129, 10, 1, 0,
                101, 1, 115, 2, 74, 0, 130, 1, 110, 3, 145, 3, 1, 0, 110, 2, 110, 1, 1, 0, 145, 1,
                174, 1, 120, 1, 128, 3, 1, 0, 89, 0, 121, 0, 99, 2, 89, 0, 119, 0, 120, 3, 89, 0,
                119, 1,
            ],
            &[131, 8, 31, 3, 139, 4, 16, 9, 143, 11, 31, 3],
            &[
                240, 3, 1, 1, 1, 240, 2, 3, 1, 16, 216, 4, 8, 248, 241, 2, 1, 1, 16, 136, 5, 243,
                0, 1, 1, 16, 217, 11, 15, 128, 75, 137, 52, 250, 136, 52, 240, 3, 1, 1, 16, 255,
                252,
            ],
        ),
    ];
    for &(source, name, bytecode, exceptions, line_table) in cases {
        let module = compile(source, PythonVersion::Py312);
        let code = name.map_or(&module, |name| child(&module, name));
        assert_eq!(code.bytecode, bytecode, "{source}");
        assert_eq!(code.exception_table, exceptions, "{source}");
        assert_eq!(code.line_table, line_table, "{source}");
    }
}

#[test]
fn legacy_dead_terminal_body_has_no_exit_predecessor() {
    // Exact pinned CPython 3.12.14 output for this public reducer.
    let module = compile(
        "if (a == b or a == c) and None:\n    d\n",
        PythonVersion::Py312,
    );
    let code = &module;
    assert_eq!(
        code.bytecode,
        &[
            151, 0, 101, 0, 101, 1, 107, 40, 0, 0, 115, 5, 101, 0, 101, 2, 107, 40, 0, 0, 114, 1,
            121, 0, 121, 0,
        ]
    );
    assert_eq!(
        code.line_table,
        &[
            240, 3, 1, 1, 1, 216, 4, 5, 136, 17, 130, 70, 136, 97, 144, 49, 138, 102, 152, 36, 136,
            102,
        ]
    );
    assert!(code.exception_table.is_empty());
}

#[test]
fn legacy_metadata_labels_do_not_block_nop_donation() {
    // Exact pinned CPython 3.12.14 output for this public reducer.
    let module = compile(
        "match x:\n    case {0: 0 | 1 | 2}:\n        pass\n",
        PythonVersion::Py312,
    );
    let code = &module;
    assert_eq!(
        code.bytecode,
        &[
            151, 0, 101, 0, 31, 0, 114, 42, 30, 0, 100, 0, 107, 92, 0, 0, 114, 37, 100, 4, 33, 0,
            120, 1, 129, 31, 92, 1, 0, 0, 120, 1, 100, 1, 107, 40, 0, 0, 114, 4, 1, 0, 1, 0, 1, 0,
            121, 2, 120, 1, 100, 0, 107, 40, 0, 0, 114, 4, 1, 0, 1, 0, 1, 0, 121, 2, 120, 1, 100,
            3, 107, 40, 0, 0, 114, 4, 1, 0, 1, 0, 1, 0, 121, 2, 1, 0, 110, 1, 1, 0, 1, 0, 1, 0,
            121, 2,
        ]
    );
    assert_eq!(
        code.line_table,
        &[
            240, 3, 1, 1, 1, 216, 6, 7, 223, 9, 23, 140, 30, 140, 81, 136, 89, 137, 30, 216, 8, 12,
            244, 3, 0, 18, 19, 136, 89, 137, 30, 216, 8, 12, 244, 3, 0, 22, 23, 136, 89, 137, 30,
            216, 8, 12, 241, 3, 0, 14, 23, 139, 30,
        ]
    );
    assert!(code.exception_table.is_empty());
}

#[test]
fn legacy_nested_assignment_preserves_original_join() {
    // Exact pinned CPython 3.13.15 output for this public reducer.
    let module = compile(
        "def f():\n    target.value = (\n        (first\n         if inner\n         else second)\n        if outer\n        else third\n    )\n",
        PythonVersion::Py313,
    );
    let code = child(&module, "f");
    assert_eq!(
        code.bytecode,
        &[
            149, 0, 91, 0, 0, 0, 0, 0, 0, 0, 0, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 44, 0, 0, 91, 2, 0,
            0, 0, 0, 0, 0, 0, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 16, 0, 0, 91, 4, 0, 0, 0, 0, 0, 0, 0,
            0, 91, 10, 0, 0, 0, 0, 0, 0, 0, 0, 108, 6, 0, 0, 0, 0, 0, 0, 0, 0, 103, 0, 91, 6, 0, 0,
            0, 0, 0, 0, 0, 0, 30, 0, 91, 10, 0, 0, 0, 0, 0, 0, 0, 0, 108, 6, 0, 0, 0, 0, 0, 0, 0,
            0, 103, 0, 91, 8, 0, 0, 0, 0, 0, 0, 0, 0, 91, 10, 0, 0, 0, 0, 0, 0, 0, 0, 108, 6, 0, 0,
            0, 0, 0, 0, 0, 0, 103, 0,
        ]
    );
    assert_eq!(
        code.line_table,
        &[
            128, 0, 247, 10, 0, 12, 17, 138, 53, 247, 5, 0, 13, 18, 138, 69, 244, 3, 0, 10, 15,
            244, 3, 0, 5, 11, 133, 76, 245, 6, 0, 15, 21, 244, 7, 0, 5, 11, 133, 76, 244, 10, 0,
            14, 19, 244, 11, 0, 5, 11, 133, 76,
        ]
    );
    assert!(code.exception_table.is_empty());
}

#[test]
fn legacy_private_class_capture_preserves_target_and_enclosing_cells() {
    use crate::target::local_kinds::{CO_FAST_CELL, CO_FAST_FREE};
    use crate::target::opcodes::{BUILD_TUPLE, LOAD_CLOSURE, LOAD_FAST};

    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        let module = compile(
            "def outer(_C__offset):\n    class C:\n        a = [lambda: __offset for __offset in data]\n        b = [lambda: __offset for x in data]\n    return C\n",
            version,
        );
        let outer = child(&module, "outer");
        let class = child(outer, "C");
        for (code, kind) in [(outer, CO_FAST_CELL), (class, CO_FAST_FREE)] {
            let captures = code
                .locals
                .iter()
                .zip(&code.local_kinds)
                .filter(|(_, flags)| **flags & kind != 0)
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>();
            assert_eq!(captures, ["_C__offset"], "{version:?}: {}", code.name);
        }
        let closure_load = if version == PythonVersion::Py312 {
            LOAD_CLOSURE
        } else {
            LOAD_FAST
        };
        let slots = class
            .bytecode
            .as_chunks::<2>()
            .0
            .windows(2)
            .filter(|pair| {
                pair[0][0] == closure_load.code_for(version)
                    && pair[1] == [BUILD_TUPLE.code_for(version), 1]
            })
            .map(|pair| pair[0][1])
            .collect::<Vec<_>>();
        assert_eq!(slots, [1, 3], "{version:?}");
    }
}

#[test]
fn legacy_copied_finally_conditions_match_native_without_branch_markers() {
    // Pinned CPython 3.12.14 and 3.13.15: both condition forms omit NOT_TAKEN.
    type NativeCase = (
        PythonVersion,
        &'static str,
        &'static [u8],
        &'static [u8],
        &'static [u8],
    );
    let cases: &[NativeCase] = &[
        (
            PythonVersion::Py312,
            "def f(flag):\n    try:\n        return 1\n    finally:\n        if flag:\n            cleanup()\n",
            &[
                151, 0, 9, 0, 9, 0, 124, 0, 114, 11, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0,
                0, 0, 0, 0, 1, 0, 121, 1, 121, 1, 35, 0, 124, 0, 114, 11, 116, 1, 0, 0, 0, 0, 0, 0,
                0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 119, 0, 119, 0, 120, 3, 89, 0, 119, 1,
            ],
            &[145, 15, 32, 3],
            &[
                128, 0, 240, 2, 4, 5, 22, 216, 15, 16, 225, 11, 15, 220, 12, 19, 141, 73, 240, 3,
                0, 12, 16, 248, 137, 52, 220, 12, 19, 141, 73, 240, 3, 0, 12, 16, 250,
            ],
        ),
        (
            PythonVersion::Py312,
            "def f(value):\n    try:\n        return 1\n    finally:\n        if value is not None:\n            cleanup()\n",
            &[
                151, 0, 9, 0, 9, 0, 124, 0, 129, 11, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0,
                0, 0, 0, 0, 1, 0, 121, 1, 121, 1, 35, 0, 124, 0, 129, 11, 116, 1, 0, 0, 0, 0, 0, 0,
                0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 119, 0, 119, 0, 120, 3, 89, 0, 119, 1,
            ],
            &[145, 15, 32, 3],
            &[
                128, 0, 240, 2, 4, 5, 22, 216, 15, 16, 224, 11, 16, 208, 11, 28, 220, 12, 19, 141,
                73, 240, 3, 0, 12, 29, 248, 136, 53, 208, 11, 28, 220, 12, 19, 141, 73, 240, 3, 0,
                12, 29, 250,
            ],
        ),
        (
            PythonVersion::Py313,
            "def f(flag):\n    try:\n        return 1\n    finally:\n        if flag:\n            cleanup()\n",
            &[
                149, 0, 30, 0, 30, 0, 85, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 11, 0, 0, 91, 1, 0, 0, 0,
                0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 103, 1, 103, 1, 33, 0, 85, 0, 40, 0,
                0, 0, 0, 0, 0, 0, 97, 11, 0, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0,
                0, 0, 32, 0, 102, 0, 102, 0, 61, 3, 31, 0, 102, 1,
            ],
            &[150, 20, 42, 3],
            &[
                128, 0, 240, 2, 4, 5, 22, 216, 15, 16, 230, 11, 15, 220, 12, 19, 141, 73, 240, 3,
                0, 12, 16, 248, 142, 52, 220, 12, 19, 141, 73, 240, 3, 0, 12, 16, 250,
            ],
        ),
        (
            PythonVersion::Py313,
            "def f(value):\n    try:\n        return 1\n    finally:\n        if value is not None:\n            cleanup()\n",
            &[
                149, 0, 30, 0, 30, 0, 85, 0, 98, 11, 0, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0,
                0, 0, 0, 0, 0, 32, 0, 103, 1, 103, 1, 33, 0, 85, 0, 98, 11, 0, 0, 91, 1, 0, 0, 0,
                0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 102, 0, 102, 0, 61, 3, 31, 0, 102,
                1,
            ],
            &[146, 16, 34, 3],
            &[
                128, 0, 240, 2, 4, 5, 22, 216, 15, 16, 224, 11, 16, 209, 11, 28, 220, 12, 19, 141,
                73, 240, 3, 0, 12, 29, 248, 136, 53, 209, 11, 28, 220, 12, 19, 141, 73, 240, 3, 0,
                12, 29, 250,
            ],
        ),
    ];
    for &(version, source, bytecode, exceptions, line_table) in cases {
        let module = compile(source, version);
        let code = child(&module, "f");
        assert_eq!(code.bytecode, bytecode, "{version:?}: {source}");
        assert_eq!(code.exception_table, exceptions, "{version:?}: {source}");
        assert_eq!(code.line_table, line_table, "{version:?}: {source}");
    }
}

#[test]
fn python_312_loop_footers_keep_native_exit_duplication() {
    // Native CPython 3.12.14 separates loop exits from the implicit footer.
    for (source, bytecode, line_table, exception_table) in [
        (
            "async def f(flag):\n    if flag:\n        async for value in values:\n            work(value)\n",
            &[
                75, 0, 1, 0, 151, 0, 124, 0, 114, 28, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 50, 0, 51, 0,
                100, 0, 123, 3, 0, 0, 150, 3, 151, 3, 134, 5, 5, 0, 125, 1, 116, 3, 0, 0, 0, 0, 0,
                0, 0, 0, 124, 1, 171, 1, 0, 0, 0, 0, 0, 0, 1, 0, 140, 21, 121, 0, 121, 0, 55, 0,
                140, 18, 54, 0, 140, 6, 173, 3, 119, 1,
            ][..],
            &[
                232, 0, 248, 128, 0, 217, 7, 11, 221, 27, 33, 247, 0, 1, 9, 24, 144, 37, 220, 12,
                16, 144, 21, 141, 75, 248, 240, 5, 0, 8, 12, 240, 2, 1, 9, 24, 248, 152, 54, 250,
            ][..],
            &[
                130, 9, 38, 1, 139, 4, 36, 2, 143, 1, 34, 6, 144, 3, 36, 2, 147, 15, 38, 1, 162, 1,
                36, 2, 164, 1, 38, 1,
            ][..],
        ),
        (
            "def f(flag, active):\n    if flag:\n        while active:\n            work()\n",
            &[
                151, 0, 124, 0, 114, 17, 124, 1, 114, 14, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0,
                0, 0, 0, 0, 0, 0, 1, 0, 124, 1, 114, 1, 140, 13, 121, 0, 121, 0, 121, 0,
            ][..],
            &[
                128, 0, 217, 7, 11, 217, 14, 20, 220, 12, 16, 140, 70, 244, 3, 0, 15, 21, 240, 3,
                0, 8, 12,
            ][..],
            &[][..],
        ),
        (
            "def f(flag):\n    try:\n        while flag:\n            work()\n    except Error:\n        recover()\n",
            &[
                151, 0, 9, 0, 124, 0, 114, 14, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0,
                0, 0, 1, 0, 124, 0, 114, 1, 140, 13, 121, 0, 121, 0, 35, 0, 116, 2, 0, 0, 0, 0, 0,
                0, 0, 0, 36, 0, 114, 13, 1, 0, 116, 5, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0,
                0, 0, 1, 0, 89, 0, 121, 0, 119, 0, 120, 3, 89, 0, 119, 1,
            ][..],
            &[
                128, 0, 240, 2, 4, 5, 18, 217, 14, 18, 220, 12, 16, 140, 70, 244, 3, 0, 15, 19,
                248, 228, 11, 16, 242, 0, 1, 5, 18, 220, 8, 15, 142, 9, 240, 3, 1, 5, 18, 250,
            ][..],
            &[130, 14, 19, 0, 147, 19, 41, 3, 168, 1, 41, 3][..],
        ),
    ] {
        let module = compile(source, PythonVersion::Py312);
        let code = child(&module, "f");
        assert_eq!(code.bytecode, bytecode, "{source}");
        assert_eq!(code.line_table, line_table, "{source}");
        assert_eq!(code.exception_table, exception_table, "{source}");
    }
}

#[test]
fn python_312_nested_handler_exits_keep_the_original_outer_owner() {
    // Exact table from pinned CPython 3.12.14: original inner exit jumps
    // remain protected by the outer handler's cleanup before retargeting.
    let module = compile(
        "def f():\n    try:\n        first()\n    except Exception:\n        try:\n            second()\n        except ValueError:\n            third()\n    finish()\n",
        PythonVersion::Py312,
    );
    assert_eq!(
        child(&module, "f").exception_table,
        [
            130, 10, 23, 0, 151, 9, 65, 18, 3, 161, 10, 44, 2, 171, 1, 65, 18, 3, 172, 19, 65, 2,
            5, 191, 2, 65, 18, 3, 193, 1, 1, 65, 2, 5, 193, 2, 3, 65, 18, 3, 193, 17, 1, 65, 18, 3
        ]
    );
}

#[test]
fn legacy_chained_comparison_value_keeps_its_exception_owner() {
    // Exact bytes from pinned CPython 3.12.14 and 3.13.15.
    for (version, code, exceptions, lines) in [
        (
            PythonVersion::Py312,
            &[
                75, 0, 1, 0, 151, 0, 124, 0, 93, 20, 0, 0, 125, 1, 100, 0, 124, 1, 99, 2, 120, 2,
                107, 2, 0, 0, 120, 1, 114, 5, 1, 0, 100, 1, 107, 2, 0, 0, 110, 2, 99, 2, 1, 0, 150,
                1, 151, 1, 1, 0, 140, 22, 4, 0, 121, 2, 173, 3, 119, 1,
            ][..],
            &[130, 26, 28, 1][..],
            &[
                232, 0, 248, 128, 0, 208, 11, 43, 161, 85, 152, 84, 136, 65, 144, 4, 142, 76, 144,
                113, 143, 76, 136, 76, 161, 85, 249,
            ][..],
        ),
        (
            PythonVersion::Py313,
            &[
                35, 0, 32, 0, 149, 0, 85, 0, 19, 0, 72, 26, 0, 0, 110, 1, 83, 0, 85, 1, 115, 2, 61,
                2, 58, 2, 0, 0, 61, 1, 40, 0, 0, 0, 0, 0, 0, 0, 97, 5, 0, 0, 32, 0, 83, 1, 58, 2,
                0, 0, 79, 2, 115, 2, 32, 0, 118, 0, 149, 5, 32, 0, 77, 28, 0, 0, 11, 0, 32, 0, 103,
                2, 55, 3, 102, 1,
            ][..],
            &[130, 34, 36, 1][..],
            &[
                233, 0, 128, 0, 208, 11, 43, 162, 85, 152, 84, 136, 65, 144, 4, 143, 76, 139, 76,
                144, 113, 143, 76, 137, 76, 162, 85, 249,
            ][..],
        ),
    ] {
        let module = compile(
            "def f(items):
    return (0 < item < 9 for item in items)
",
            version,
        );
        let value = child(child(&module, "f"), "<genexpr>");
        assert_eq!(value.bytecode, code, "{version:?}");
        assert_eq!(value.exception_table, exceptions, "{version:?}");
        assert_eq!(value.line_table, lines, "{version:?}");
    }
}

#[test]
fn python_312_empty_else_exits_keep_original_missing_location() {
    // Native CPython 3.12.14 threads the unlocated normal exit before line donation.
    for (source, bytecode, line_table, exception_table) in [
        (
            "def f(value):\n    try:\n        value = prepare(value)\n    except Failure:\n        raise Other\n    else:\n        if value is None:\n            raise Invalid\n    return value\n",
            &[
                151, 0, 9, 0, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 124, 0, 171, 1, 0, 0, 0, 0, 0, 0,
                125, 0, 124, 0, 128, 6, 116, 2, 0, 0, 0, 0, 0, 0, 0, 0, 130, 1, 124, 0, 83, 0, 35,
                0, 116, 4, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 7, 1, 0, 116, 6, 0, 0, 0, 0, 0, 0,
                0, 0, 130, 1, 119, 0, 120, 3, 89, 0, 119, 1,
            ][..],
            &[
                128, 0, 240, 2, 6, 5, 26, 220, 16, 23, 152, 5, 147, 14, 136, 5, 240, 8, 0, 12, 17,
                136, 61, 220, 18, 25, 136, 77, 216, 11, 16, 128, 76, 248, 244, 11, 0, 12, 19, 242,
                0, 1, 5, 20, 220, 14, 19, 136, 11, 240, 3, 1, 5, 20, 250,
            ][..],
            &[130, 11, 23, 0, 151, 16, 39, 3][..],
        ),
        (
            "def f(items):\n    for value in items:\n        try:\n            value = prepare(value)\n        except Failure:\n            value = None\n        else:\n            if first(value):\n                value = 1\n            elif second(value):\n                value = 2\n            else:\n                raise Invalid\n        consume(value)\n",
            &[
                151, 0, 124, 0, 68, 0, 93, 59, 0, 0, 125, 1, 9, 0, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0,
                124, 1, 171, 1, 0, 0, 0, 0, 0, 0, 125, 1, 116, 3, 0, 0, 0, 0, 0, 0, 0, 0, 124, 1,
                171, 1, 0, 0, 0, 0, 0, 0, 114, 3, 100, 1, 125, 1, 110, 20, 116, 5, 0, 0, 0, 0, 0,
                0, 0, 0, 124, 1, 171, 1, 0, 0, 0, 0, 0, 0, 114, 3, 100, 2, 125, 1, 110, 6, 116, 6,
                0, 0, 0, 0, 0, 0, 0, 0, 130, 1, 116, 11, 0, 0, 0, 0, 0, 0, 0, 0, 124, 1, 171, 1, 0,
                0, 0, 0, 0, 0, 1, 0, 140, 61, 4, 0, 121, 0, 35, 0, 116, 8, 0, 0, 0, 0, 0, 0, 0, 0,
                36, 0, 114, 5, 1, 0, 100, 0, 125, 1, 89, 0, 140, 27, 119, 0, 120, 3, 89, 0, 119, 1,
            ][..],
            &[
                128, 0, 219, 17, 22, 136, 5, 240, 2, 10, 9, 30, 220, 20, 27, 152, 69, 147, 78, 136,
                69, 244, 8, 0, 16, 21, 144, 85, 140, 124, 216, 24, 25, 145, 5, 220, 17, 23, 152, 5,
                148, 29, 216, 24, 25, 145, 5, 228, 22, 29, 144, 13, 220, 8, 15, 144, 5, 141, 14,
                241, 25, 0, 18, 23, 248, 244, 6, 0, 16, 23, 242, 0, 1, 9, 25, 216, 20, 24, 138, 69,
                240, 3, 1, 9, 25, 250,
            ][..],
            &[
                135, 11, 65, 2, 2, 193, 2, 11, 65, 16, 5, 193, 15, 1, 65, 16, 5,
            ][..],
        ),
        (
            "def f(items):\n    for value in items:\n        try:\n            value = prepare(value)\n        except Failure:\n            value = None\n        else:\n            match value:\n                case 1:\n                    value = 2\n                case 3:\n                    raise Invalid\n        consume(value)\n",
            &[
                151, 0, 124, 0, 68, 0, 93, 45, 0, 0, 125, 1, 9, 0, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0,
                124, 1, 171, 1, 0, 0, 0, 0, 0, 0, 125, 1, 124, 1, 120, 1, 100, 1, 107, 40, 0, 0,
                114, 4, 1, 0, 100, 2, 125, 1, 110, 10, 100, 3, 107, 40, 0, 0, 114, 6, 116, 2, 0, 0,
                0, 0, 0, 0, 0, 0, 130, 1, 116, 7, 0, 0, 0, 0, 0, 0, 0, 0, 124, 1, 171, 1, 0, 0, 0,
                0, 0, 0, 1, 0, 140, 47, 4, 0, 121, 0, 35, 0, 116, 4, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0,
                114, 5, 1, 0, 100, 0, 125, 1, 89, 0, 140, 27, 119, 0, 120, 3, 89, 0, 119, 1,
            ][..],
            &[
                128, 0, 219, 17, 22, 136, 5, 240, 2, 9, 9, 34, 220, 20, 27, 152, 69, 147, 78, 136,
                69, 240, 8, 0, 19, 24, 221, 21, 22, 216, 28, 29, 145, 69, 219, 21, 22, 220, 26, 33,
                144, 77, 220, 8, 15, 144, 5, 141, 14, 241, 23, 0, 18, 23, 248, 244, 6, 0, 16, 23,
                242, 0, 1, 9, 25, 216, 20, 24, 138, 69, 240, 3, 1, 9, 25, 250,
            ][..],
            &[135, 11, 52, 2, 180, 11, 65, 2, 5, 193, 1, 1, 65, 2, 5][..],
        ),
    ] {
        let module = compile(source, PythonVersion::Py312);
        let code = child(&module, "f");
        assert_eq!(code.bytecode, bytecode, "{source}");
        assert_eq!(code.line_table, line_table, "{source}");
        assert_eq!(code.exception_table, exception_table, "{source}");
    }
}

// Native bytecode, line and exception tables captured with pinned CPython 3.12.14/3.13.15.
#[test]
fn legacy_original_exit_locations_and_slots_follow_native_phases() {
    type NativeExitCase = (
        PythonVersion,
        &'static str,
        &'static [u8],
        &'static [u8],
        &'static [u8],
    );
    let cases: &[NativeExitCase] = &[
        (
            PythonVersion::Py312,
            "def f(a,b):\n    try:\n        setup()\n    except Error:\n        result = 0\n    else:\n        if a:\n            result = 1\n        elif b:\n            result = 2\n        else:\n            result = 3\n    use(result)\n",
            &[
                151, 0, 9, 0, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 124,
                0, 114, 3, 100, 1, 125, 2, 110, 8, 124, 1, 114, 3, 100, 2, 125, 2, 110, 3, 100, 3,
                125, 2, 9, 0, 116, 5, 0, 0, 0, 0, 0, 0, 0, 0, 124, 2, 171, 1, 0, 0, 0, 0, 0, 0, 1,
                0, 121, 0, 35, 0, 116, 2, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 5, 1, 0, 100, 4, 125,
                2, 89, 0, 140, 25, 119, 0, 120, 3, 89, 0, 119, 1,
            ],
            &[130, 10, 37, 0, 165, 11, 51, 3, 178, 1, 51, 3],
            &[
                128, 0, 240, 2, 10, 5, 23, 220, 8, 13, 140, 7, 241, 8, 0, 12, 13, 216, 21, 22, 137,
                70, 217, 13, 14, 216, 21, 22, 137, 70, 224, 21, 22, 137, 70, 220, 4, 7, 136, 6,
                133, 75, 248, 244, 19, 0, 12, 17, 242, 0, 1, 5, 19, 216, 17, 18, 138, 6, 240, 3, 1,
                5, 19, 250,
            ],
        ),
        (
            PythonVersion::Py312,
            "def f(a,b):\n    if True:\n        if a:\n            result = 1\n        elif b:\n            result = 2\n        else:\n            result = 3\n    else:\n        result = 4\n    use(result)\n",
            &[
                151, 0, 9, 0, 124, 0, 114, 3, 100, 1, 125, 2, 110, 8, 124, 1, 114, 3, 100, 2, 125,
                2, 110, 3, 100, 3, 125, 2, 9, 0, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 124, 2, 171, 1, 0,
                0, 0, 0, 0, 0, 1, 0, 121, 0,
            ],
            &[],
            &[
                128, 0, 216, 7, 11, 217, 11, 12, 216, 21, 22, 137, 70, 217, 13, 14, 216, 21, 22,
                137, 70, 224, 21, 22, 137, 70, 244, 6, 0, 5, 8, 136, 6, 133, 75,
            ],
        ),
        (
            PythonVersion::Py313,
            "def f(a,b):\n    if True:\n        if a:\n            result = 1\n        elif b:\n            result = 2\n        else:\n            result = 3\n    else:\n        result = 4\n    use(result)\n",
            &[
                149, 0, 30, 0, 85, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 3, 0, 0, 83, 1, 110, 2, 79, 13,
                85, 1, 40, 0, 0, 0, 0, 0, 0, 0, 97, 3, 0, 0, 83, 2, 110, 2, 79, 3, 83, 3, 110, 2,
                30, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0, 85, 2, 53, 1, 0, 0, 0, 0, 0, 0, 32, 0, 103,
                0,
            ],
            &[],
            &[
                128, 0, 216, 7, 11, 222, 11, 12, 216, 21, 22, 137, 70, 222, 13, 14, 216, 21, 22,
                137, 70, 224, 21, 22, 137, 70, 244, 6, 0, 5, 8, 136, 6, 133, 75,
            ],
        ),
        (
            PythonVersion::Py313,
            "def f(a,b):\n    if a:\n        if b:\n            first()\n        else:\n            second()\n        pass\n    else:\n        other()\n    after()\n",
            &[
                149, 0, 85, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 29, 0, 0, 85, 1, 40, 0, 0, 0, 0, 0, 0,
                0, 97, 11, 0, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 79,
                10, 91, 3, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 79, 10, 91, 5,
                0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 91, 7, 0, 0, 0, 0, 0, 0, 0,
                0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 103, 0,
            ],
            &[],
            &[
                128, 0, 222, 7, 8, 222, 11, 12, 220, 12, 17, 141, 71, 228, 12, 18, 140, 72, 216, 8,
                12, 228, 8, 13, 140, 7, 220, 4, 9, 133, 71,
            ],
        ),
        (
            PythonVersion::Py313,
            "def f(a,b):\n    try:\n        setup()\n    except Error:\n        result = 0\n    else:\n        if a:\n            result = 1\n        elif b:\n            result = 2\n        else:\n            result = 3\n        pass\n    use(result)\n",
            &[
                149, 0, 30, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 85,
                0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 3, 0, 0, 83, 1, 110, 2, 79, 12, 85, 1, 40, 0, 0, 0,
                0, 0, 0, 0, 97, 3, 0, 0, 83, 2, 110, 2, 79, 2, 83, 3, 110, 2, 30, 0, 91, 5, 0, 0,
                0, 0, 0, 0, 0, 0, 85, 2, 53, 1, 0, 0, 0, 0, 0, 0, 32, 0, 103, 0, 33, 0, 91, 2, 0,
                0, 0, 0, 0, 0, 0, 0, 7, 0, 97, 5, 0, 0, 32, 0, 83, 4, 110, 2, 31, 0, 78, 26, 102,
                0, 61, 3, 31, 0, 102, 1,
            ],
            &[130, 10, 47, 0, 175, 12, 62, 3, 189, 1, 62, 3],
            &[
                128, 0, 240, 2, 11, 5, 13, 220, 8, 13, 140, 7, 246, 8, 0, 12, 13, 216, 21, 22, 137,
                70, 222, 13, 14, 216, 21, 22, 137, 70, 224, 21, 22, 136, 70, 216, 8, 12, 220, 4, 7,
                136, 6, 133, 75, 248, 244, 21, 0, 12, 17, 243, 0, 1, 5, 19, 216, 17, 18, 138, 6,
                240, 3, 1, 5, 19, 250,
            ],
        ),
        (
            PythonVersion::Py313,
            "def f(a, b, c):\n    try:\n        value = a < b < c\n    except Exception:\n        value = False\n    return value\n",
            &[
                149, 0, 30, 0, 88, 1, 115, 2, 61, 2, 58, 2, 0, 0, 61, 1, 40, 0, 0, 0, 0, 0, 0, 0,
                97, 5, 0, 0, 32, 0, 85, 2, 58, 2, 0, 0, 79, 2, 115, 2, 32, 0, 110, 3, 85, 3, 36, 0,
                33, 0, 91, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 97, 6, 0, 0, 32, 0, 83, 1, 110, 3, 31,
                0, 85, 3, 36, 0, 102, 0, 61, 3, 31, 0, 102, 1,
            ],
            &[130, 20, 24, 0, 152, 12, 40, 3, 167, 1, 40, 3],
            &[
                128, 0, 240, 2, 3, 5, 22, 216, 16, 17, 151, 9, 147, 9, 152, 1, 148, 9, 136, 5, 240,
                6, 0, 12, 17, 128, 76, 248, 244, 5, 0, 12, 21, 243, 0, 1, 5, 22, 216, 16, 21, 137,
                5, 216, 11, 16, 128, 76, 240, 5, 1, 5, 22, 250,
            ],
        ),
        (
            PythonVersion::Py313,
            "def f(a, b, c):\n    try:\n        value = a if b else c\n    except Exception:\n        value = False\n    return value\n",
            &[
                149, 0, 30, 0, 85, 1, 40, 0, 0, 0, 0, 0, 0, 0, 97, 2, 0, 0, 85, 0, 79, 1, 85, 2,
                110, 3, 85, 3, 36, 0, 33, 0, 91, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 97, 6, 0, 0, 32,
                0, 83, 1, 110, 3, 31, 0, 85, 3, 36, 0, 102, 0, 61, 3, 31, 0, 102, 1,
            ],
            &[130, 11, 15, 0, 143, 12, 31, 3, 158, 1, 31, 3],
            &[
                128, 0, 240, 2, 3, 5, 22, 222, 21, 22, 145, 1, 152, 65, 136, 5, 240, 6, 0, 12, 17,
                128, 76, 248, 244, 5, 0, 12, 21, 243, 0, 1, 5, 22, 216, 16, 21, 137, 5, 216, 11,
                16, 128, 76, 240, 5, 1, 5, 22, 250,
            ],
        ),
    ];
    for &(version, source, bytecode, exceptions, line_table) in cases {
        let module = compile(source, version);
        let code = child(&module, "f");
        assert_eq!(code.bytecode, bytecode, "{version:?}: {source}");
        assert_eq!(code.exception_table, exceptions, "{version:?}: {source}");
        assert_eq!(code.line_table, line_table, "{version:?}: {source}");
    }
}

#[test]
fn python_312_normal_finally_jump_keeps_its_manager_owner() {
    // Pinned CPython 3.12.14 retains the original normal-finally jump owner
    // when cold relocation converts it into a NOP before manager cleanup.
    for (source, exception_table) in [
        (
            "def f(cm, flag):\n    with cm:\n        try:\n            yield\n        finally:\n            if flag:\n                finish()\n",
            &[
                130, 3, 63, 1, 133, 1, 51, 3, 135, 4, 33, 2, 139, 13, 51, 3, 152, 9, 63, 1, 161,
                15, 48, 5, 176, 3, 51, 3, 179, 5, 60, 7, 184, 7, 63, 1,
            ][..],
        ),
        (
            "def f(cm, flag):\n    with cm:\n        try:\n            work()\n        finally:\n            if flag:\n                finish()\n",
            &[
                131, 1, 55, 3, 133, 10, 37, 2, 143, 13, 55, 3, 165, 15, 52, 5, 180, 3, 55, 3, 183,
                5, 65, 0, 7,
            ][..],
        ),
    ] {
        let module = compile(source, PythonVersion::Py312);
        assert_eq!(
            child(&module, "f").exception_table,
            exception_table,
            "{source}"
        );
    }
}

// Pinned CPython 3.12.14 bytes from existing saved public-controls evidence.
#[test]
fn python312_match_joins_preserve_the_normal_try_cleanup_copy_budget() {
    for (source, bytecode, line_table, exception_table) in [
        (
            "def f(value):\n    try:\n        match value:\n            case 1:\n                work()\n            case 2:\n                pass\n        finish()\n    except Failure:\n        recover()\n",
            &[
                151, 0, 9, 0, 124, 0, 120, 1, 100, 1, 107, 40, 0, 0, 114, 12, 1, 0, 116, 1, 0, 0,
                0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 110, 5, 100, 2, 107, 40, 0, 0,
                114, 1, 9, 0, 116, 3, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 121,
                0, 35, 0, 116, 4, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 13, 1, 0, 116, 7, 0, 0, 0, 0,
                0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 89, 0, 121, 0, 119, 0, 120, 3, 89, 0,
                119, 1,
            ][..],
            &[
                128, 0, 240, 2, 8, 5, 18, 216, 14, 19, 221, 17, 18, 220, 16, 20, 149, 6, 219, 17,
                18, 216, 16, 20, 220, 8, 14, 141, 8, 248, 220, 11, 18, 242, 0, 1, 5, 18, 220, 8,
                15, 142, 9, 240, 3, 1, 5, 18, 250,
            ][..],
            &[130, 33, 36, 0, 164, 19, 58, 3, 185, 1, 58, 3][..],
        ),
        (
            "def f(value):\n    try:\n        match value:\n            case 1:\n                pass\n            case _:\n                work()\n        finish()\n    except Failure:\n        recover()\n",
            &[
                151, 0, 9, 0, 124, 0, 100, 1, 107, 40, 0, 0, 114, 1, 110, 11, 9, 0, 116, 1, 0, 0,
                0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 116, 3, 0, 0, 0, 0, 0, 0, 0, 0,
                171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0, 35, 0, 116, 4, 0, 0, 0, 0, 0, 0, 0, 0, 36,
                0, 114, 13, 1, 0, 116, 7, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0,
                89, 0, 121, 0, 119, 0, 120, 3, 89, 0, 119, 1,
            ][..],
            &[
                128, 0, 240, 2, 8, 5, 18, 216, 14, 19, 219, 17, 18, 216, 16, 20, 216, 17, 18, 220,
                16, 20, 148, 6, 220, 8, 14, 141, 8, 248, 220, 11, 18, 242, 0, 1, 5, 18, 220, 8, 15,
                142, 9, 240, 3, 1, 5, 18, 250,
            ][..],
            &[130, 27, 30, 0, 158, 19, 52, 3, 179, 1, 52, 3][..],
        ),
        (
            "def f(value):\n    try:\n        match value:\n            case 1: pass\n            case _: work()\n        finish()\n    except Failure:\n        recover()\n",
            &[
                151, 0, 9, 0, 124, 0, 100, 1, 107, 40, 0, 0, 114, 1, 110, 10, 116, 1, 0, 0, 0, 0,
                0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 116, 3, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0,
                0, 0, 0, 0, 0, 0, 1, 0, 121, 0, 35, 0, 116, 4, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114,
                13, 1, 0, 116, 7, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 89, 0,
                121, 0, 119, 0, 120, 3, 89, 0, 119, 1,
            ][..],
            &[
                128, 0, 240, 2, 6, 5, 18, 216, 14, 19, 219, 17, 18, 144, 68, 220, 20, 24, 148, 70,
                220, 8, 14, 141, 8, 248, 220, 11, 18, 242, 0, 1, 5, 18, 220, 8, 15, 142, 9, 240, 3,
                1, 5, 18, 250,
            ][..],
            &[130, 26, 29, 0, 157, 19, 51, 3, 178, 1, 51, 3][..],
        ),
        (
            "def f(value):\n    try:\n        match value:\n            case (False, error):\n                return error\n            case (True, first, second):\n                pass\n            case _:\n                raise Failure\n        finish()\n    except Failure:\n        recover()\n",
            &[
                151, 0, 9, 0, 124, 0, 120, 1, 32, 0, 114, 14, 30, 0, 100, 1, 107, 40, 0, 0, 114, 9,
                92, 2, 0, 0, 100, 2, 117, 0, 114, 4, 125, 1, 1, 0, 124, 1, 83, 0, 1, 0, 32, 0, 114,
                14, 30, 0, 100, 3, 107, 40, 0, 0, 114, 9, 92, 3, 0, 0, 100, 4, 117, 0, 114, 3, 125,
                2, 125, 3, 110, 9, 1, 0, 1, 0, 9, 0, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 130, 1, 116,
                3, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0, 35, 0, 116, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 13, 1, 0, 116, 5, 0, 0, 0, 0, 0, 0, 0, 0, 171,
                0, 0, 0, 0, 0, 0, 0, 1, 0, 89, 0, 121, 0, 119, 0, 120, 3, 89, 0, 119, 1,
            ][..],
            &[
                128, 0, 240, 2, 10, 5, 18, 216, 14, 19, 223, 17, 31, 145, 30, 146, 37, 145, 30,
                216, 23, 28, 144, 12, 240, 3, 0, 18, 32, 231, 17, 38, 208, 17, 38, 146, 36, 209,
                17, 38, 216, 16, 20, 241, 3, 0, 18, 39, 224, 17, 18, 220, 22, 29, 144, 13, 220, 8,
                14, 141, 8, 248, 220, 11, 18, 242, 0, 1, 5, 18, 220, 8, 15, 142, 9, 240, 3, 1, 5,
                18, 250,
            ][..],
            &[
                130, 17, 56, 0, 148, 35, 56, 0, 184, 19, 65, 14, 3, 193, 13, 1, 65, 14, 3,
            ][..],
        ),
    ] {
        let module = compile(source, PythonVersion::Py312);
        let function = child(&module, "f");
        assert_eq!(function.bytecode, bytecode, "{source}");
        assert_eq!(function.line_table, line_table, "{source}");
        assert_eq!(function.exception_table, exception_table, "{source}");
    }
}

#[test]
fn python_312_async_match_exhaustion_keeps_original_jump_owner() {
    // Pinned CPython 3.12.14 retains the original case-success NOP's scope
    // owner; the separately inserted cold bridge remains ownerless.
    let module = compile(
        "async def f(key, items):\n    while True:\n        match key:\n            case 0:\n                work()\n            case 1:\n                async for item in items:\n                    await consume(item)\n        await finish()\n",
        PythonVersion::Py312,
    );
    assert_eq!(
        child(&module, "f").exception_table,
        [
            130, 26, 65, 21, 1, 156, 4, 65, 16, 2, 160, 1, 65, 12, 6, 161, 3, 65, 16, 2, 164, 15,
            65, 21, 1, 179, 1, 65, 14, 6, 180, 18, 65, 21, 1, 193, 6, 1, 65, 19, 4, 193, 7, 5, 65,
            21, 1, 193, 12, 1, 65, 16, 2, 193, 14, 1, 65, 21, 1, 193, 16, 2, 65, 21, 1, 193, 19, 1,
            65, 21, 1
        ]
    );
}

#[test]
fn python_312_yield_depth_counts_one_scope_fallback() {
    // Original SETUP stack depth, before owner-range exclusions and copies.
    for (source, expected) in [
        (
            "async def f(manager, items):\n    try:\n        async with manager:\n            async for value in items:\n                yield value\n    except Exception:\n        raise\n",
            &[
                75, 0, 1, 0, 151, 0, 9, 0, 124, 0, 52, 0, 131, 1, 100, 0, 123, 3, 0, 0, 150, 3,
                151, 3, 134, 5, 5, 0, 1, 0, 124, 1, 50, 0, 51, 0, 100, 0, 123, 3, 0, 0, 150, 5,
                151, 3, 134, 5, 5, 0, 125, 2, 124, 2, 173, 4, 150, 3, 151, 1, 1, 0, 140, 15, 55, 0,
                140, 21, 55, 0, 140, 12, 54, 0, 100, 0, 100, 0, 100, 0, 171, 2, 0, 0, 0, 0, 0, 0,
                131, 2, 100, 0, 123, 4, 0, 0, 150, 3, 151, 3, 134, 5, 55, 0, 5, 0, 1, 0, 121, 0,
                35, 0, 49, 0, 131, 2, 100, 0, 123, 4, 0, 0, 150, 4, 151, 3, 134, 5, 55, 0, 5, 0,
                115, 1, 119, 2, 1, 0, 89, 0, 1, 0, 1, 0, 121, 0, 120, 3, 89, 0, 119, 1, 35, 0, 116,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 2, 1, 0, 130, 0, 119, 0, 120, 3, 89, 0, 119,
                1, 173, 3, 119, 1,
            ][..],
        ),
        (
            "async def f(condition):\n    try:\n        try:\n            return work()\n        finally:\n            await close()\n    finally:\n        if condition:\n            await finish()\n",
            &[
                75, 0, 1, 0, 151, 0, 9, 0, 9, 0, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0,
                0, 0, 0, 116, 3, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 131, 0, 100, 0,
                123, 3, 0, 0, 150, 3, 151, 3, 134, 5, 5, 0, 1, 0, 124, 0, 114, 19, 116, 5, 0, 0, 0,
                0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 131, 0, 100, 0, 123, 3, 0, 0, 150, 2, 151,
                3, 134, 5, 5, 0, 1, 0, 83, 0, 83, 0, 55, 0, 140, 26, 55, 0, 140, 8, 35, 0, 116, 3,
                0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 131, 0, 100, 0, 123, 4, 0, 0,
                150, 4, 151, 3, 134, 5, 55, 0, 5, 0, 1, 0, 119, 0, 120, 3, 89, 0, 119, 1, 35, 0,
                124, 0, 114, 20, 116, 5, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 131, 0,
                100, 0, 123, 4, 0, 0, 150, 3, 151, 3, 134, 5, 55, 0, 5, 0, 1, 0, 119, 0, 119, 0,
                120, 3, 89, 0, 119, 1, 173, 3, 119, 1,
            ][..],
        ),
    ] {
        let module = compile(source, PythonVersion::Py312);
        assert_eq!(child(&module, "f").bytecode, expected);
    }
}

#[test]
fn python_313_while_footers_keep_distinct_false_exits() {
    // Exact bytecode and tables from pinned CPython 3.13.15.
    for (source, bytecode, line_table, exception_table) in [
        (
            "def f(flag, active):\n    if flag:\n        while active:\n            work()\n",
            &[
                149, 0, 85, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 28, 0, 0, 85, 1, 40, 0, 0, 0, 0, 0, 0,
                0, 97, 20, 0, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 85,
                1, 40, 0, 0, 0, 0, 0, 0, 0, 97, 2, 0, 0, 77, 19, 0, 0, 103, 0, 103, 0, 103, 0,
            ][..],
            &[
                128, 0, 222, 7, 11, 222, 14, 20, 220, 12, 16, 140, 70, 247, 3, 0, 15, 21, 138, 102,
                240, 3, 0, 8, 12,
            ][..],
            &[][..],
        ),
        (
            "def f(flag):\n    try:\n        while flag:\n            work()\n    except Error:\n        recover()\n",
            &[
                149, 0, 30, 0, 85, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 20, 0, 0, 91, 1, 0, 0, 0, 0, 0,
                0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 85, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 2, 0,
                0, 77, 19, 0, 0, 103, 0, 103, 0, 33, 0, 91, 2, 0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 97,
                13, 0, 0, 32, 0, 91, 5, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 31,
                0, 103, 0, 102, 0, 61, 3, 31, 0, 102, 1,
            ][..],
            &[
                128, 0, 240, 2, 4, 5, 18, 222, 14, 18, 220, 12, 16, 140, 70, 247, 3, 0, 15, 19,
                138, 100, 248, 228, 11, 16, 243, 0, 1, 5, 18, 220, 8, 15, 142, 9, 240, 3, 1, 5, 18,
                250,
            ][..],
            &[130, 24, 30, 0, 158, 20, 53, 3, 180, 1, 53, 3][..],
        ),
    ] {
        let module = compile(source, PythonVersion::Py313);
        let code = child(&module, "f");
        assert_eq!(code.bytecode, bytecode, "{source}");
        assert_eq!(code.line_table, line_table, "{source}");
        assert_eq!(code.exception_table, exception_table, "{source}");
    }
}

#[test]
fn python_313_folded_else_loop_keeps_finally_pop_unlocated() {
    // Exact bytecode and tables from pinned CPython 3.13.15.
    {
        let (source, bytecode, line_table, exception_table) = (
            "def f(cleanup):\n    try:\n        work()\n    except Failure:\n        raise\n    else:\n        while True:\n            work()\n            if ready():\n                result()\n                break\n            retry()\n    finally:\n        cleanup()\n",
            &[
                149, 0, 30, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 30,
                0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 91, 3, 0, 0, 0,
                0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 11, 0, 0, 91,
                5, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 79, 12, 91, 7, 0, 0, 0,
                0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 77, 48, 0, 0, 85, 0, 34, 0, 53, 0,
                0, 0, 0, 0, 0, 0, 32, 0, 103, 0, 33, 0, 91, 8, 0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 97, 2,
                0, 0, 32, 0, 101, 0, 102, 0, 61, 3, 31, 0, 102, 1, 33, 0, 85, 0, 34, 0, 53, 0, 0,
                0, 0, 0, 0, 0, 32, 0, 102, 0, 61, 3, 31, 0, 102, 1,
            ][..],
            &[
                128, 0, 240, 2, 12, 5, 18, 220, 8, 12, 140, 6, 240, 8, 0, 15, 19, 220, 12, 16, 140,
                70, 220, 15, 20, 143, 119, 137, 119, 220, 16, 22, 148, 8, 216, 16, 21, 220, 12, 17,
                140, 71, 241, 11, 0, 15, 19, 241, 14, 0, 9, 16, 141, 9, 248, 244, 21, 0, 12, 19,
                243, 0, 1, 5, 14, 216, 8, 13, 240, 3, 1, 5, 14, 251, 241, 20, 0, 9, 16, 141, 9,
                250,
            ][..],
            &[
                130, 10, 65, 5, 0, 140, 49, 65, 20, 0, 193, 5, 12, 65, 17, 3, 193, 17, 3, 65, 20,
                0, 193, 20, 9, 65, 29, 3,
            ][..],
        );
        let module = compile(source, PythonVersion::Py313);
        let code = child(&module, "f");
        assert_eq!(code.bytecode, bytecode, "{source}");
        assert_eq!(code.line_table, line_table, "{source}");
        assert_eq!(code.exception_table, exception_table, "{source}");
    }
}

#[test]
fn python_312_docstrings_preserve_native_surrogate_escapes() {
    // Pinned CPython 3.12.14 values; 3.13 rejects these docstrings while cleaning them.
    type DocCase = (&'static str, Option<&'static str>, usize, &'static [u8]);
    let cases: &[DocCase] = &[
        (
            "(\"raw \" r\"\\udc80\" \" escaped \\udc80\")\nvalue = 1\n",
            None,
            0,
            &[
                114, 97, 119, 32, 92, 117, 100, 99, 56, 48, 32, 101, 115, 99, 97, 112, 101, 100,
                32, 237, 178, 128,
            ],
        ),
        (
            "class C:\n    \"prefix \\udc80 suffix\"\n    value = 1\n",
            Some("C"),
            1,
            &[
                112, 114, 101, 102, 105, 120, 32, 237, 178, 128, 32, 115, 117, 102, 102, 105, 120,
            ],
        ),
        (
            "def f():\n    \"prefix \\ud83d\\udcc5 suffix\"\n    return 1\n",
            Some("f"),
            0,
            &[
                112, 114, 101, 102, 105, 120, 32, 237, 160, 189, 237, 179, 133, 32, 115, 117, 102,
                102, 105, 120,
            ],
        ),
        (
            "\"prefix \\udc80 suffix\"\nvalue = 1\n",
            None,
            0,
            &[
                112, 114, 101, 102, 105, 120, 32, 237, 178, 128, 32, 115, 117, 102, 102, 105, 120,
            ],
        ),
        (
            "\"replacement \\ufffd surrogate \\udc80 replacement \\ufffd\"\nvalue = 1\n",
            None,
            0,
            &[
                114, 101, 112, 108, 97, 99, 101, 109, 101, 110, 116, 32, 239, 191, 189, 32, 115,
                117, 114, 114, 111, 103, 97, 116, 101, 32, 237, 178, 128, 32, 114, 101, 112, 108,
                97, 99, 101, 109, 101, 110, 116, 32, 239, 191, 189,
            ],
        ),
    ];
    for &(source, name, index, expected) in cases {
        let module = compile(source, PythonVersion::Py312);
        let code = name.map_or(&module, |name| child(&module, name));
        let Constant::SurrogateString(actual) = &code.constants[index] else {
            panic!("expected a surrogate string for {source}");
        };
        assert_eq!(actual, expected, "{source}");
    }
}

#[test]
fn python312_unlocated_return_cleanup_keeps_a_line_without_columns() {
    // CPython 3.12.14 assigns the preceding block's line after propagation when
    // except* cleanup leaves an entirely unlocated RETURN_VALUE block.
    let module = compile(
        "def f(work, cleanup):\n    try:\n        return work()\n    finally:\n        try:\n            cleanup()\n        except* Exception:\n            pass\n",
        PythonVersion::Py312,
    );
    let function = child(&module, "f");
    assert_eq!(
        function.line_table,
        [
            128, 0, 240, 2, 6, 5, 17, 217, 15, 19, 139, 118, 240, 4, 3, 9, 17, 217, 12, 19, 141,
            73, 248, 241, 2, 1, 9, 17, 148, 9, 243, 0, 1, 9, 17, 216, 12, 16, 250, 136, 68, 240, 3,
            1, 9, 17, 251, 234, 0, 254, 240, 5, 3, 9, 17, 217, 12, 19, 141, 73, 248, 241, 2, 1, 9,
            17, 148, 9, 243, 0, 1, 9, 17, 216, 12, 16, 250, 136, 68, 240, 3, 1, 9, 17, 255, 255,
        ],
    );
}

#[test]
fn legacy_mapping_keys_fold_after_preserving_literal_locations() {
    // Exact bytecode and line tables from pinned CPython 3.12.14/3.13.15.
    let source = "def f(value):\n    match value:\n        case {\n            \"first\": x,\n            \"second\": y,\n        }:\n            return x, y\n        case _:\n            return None\n";
    let cases: &[(PythonVersion, &[u8], &[u8])] = &[
        (
            PythonVersion::Py312,
            &[
                151, 0, 124, 0, 31, 0, 114, 25, 30, 0, 100, 1, 107, 92, 0, 0, 114, 20, 9, 0, 9, 0,
                100, 2, 33, 0, 120, 1, 129, 12, 92, 2, 0, 0, 9, 0, 9, 0, 125, 1, 125, 2, 1, 0, 1,
                0, 124, 1, 124, 2, 102, 2, 83, 0, 1, 0, 1, 0, 1, 0, 9, 0, 121, 0,
            ],
            &[
                128, 0, 216, 10, 15, 246, 2, 3, 14, 10, 216, 12, 19, 216, 12, 20, 245, 5, 3, 14,
                10, 216, 21, 22, 216, 22, 23, 243, 5, 3, 14, 10, 240, 8, 0, 20, 21, 144, 97, 144,
                52, 136, 75, 242, 9, 3, 14, 10, 240, 10, 0, 14, 15, 216, 19, 23,
            ],
        ),
        (
            PythonVersion::Py313,
            &[
                149, 0, 85, 0, 28, 0, 97, 25, 0, 0, 20, 0, 83, 1, 58, 172, 0, 0, 97, 19, 0, 0, 30,
                0, 30, 0, 83, 2, 27, 0, 61, 1, 98, 10, 0, 0, 117, 2, 0, 0, 30, 0, 30, 0, 112, 18,
                32, 0, 32, 0, 88, 18, 52, 2, 36, 0, 32, 0, 32, 0, 32, 0, 30, 0, 103, 0,
            ],
            &[
                128, 0, 216, 10, 15, 247, 2, 3, 14, 10, 240, 0, 3, 14, 10, 216, 12, 19, 216, 12,
                20, 246, 5, 3, 14, 10, 216, 21, 22, 216, 22, 23, 242, 5, 3, 14, 10, 240, 8, 0, 20,
                21, 144, 52, 136, 75, 242, 9, 3, 14, 10, 240, 10, 0, 14, 15, 216, 19, 23,
            ],
        ),
    ];
    for &(version, bytecode, line_table) in cases {
        let module = compile(source, version);
        let function = child(&module, "f");
        assert_eq!(function.bytecode, bytecode, "{version:?}");
        assert_eq!(function.line_table, line_table, "{version:?}");
    }
}

#[test]
fn python312_iterator_try_exit_inherits_its_own_span() {
    // Native 3.12's unlocated POP_BLOCK at the shared join is copied before
    // line propagation. Iterator exhaustion and the else Pass keep distinct spans.
    let module = compile(
        "def f(values, flag):\n    try:\n        if flag:\n            for value in values:\n                use(value)\n        else:\n            pass\n    except Failure as error:\n        recover(error)\n        return []\n    return values\n",
        PythonVersion::Py312,
    );
    assert_eq!(
        child(&module, "f").line_table,
        [
            128, 0, 240, 2, 8, 5, 18, 217, 11, 15, 219, 25, 31, 144, 5, 220, 16, 19, 144, 69, 149,
            10, 241, 3, 0, 26, 32, 240, 14, 0, 12, 18, 128, 77, 240, 9, 0, 13, 17, 240, 8, 0, 12,
            18, 128, 77, 248, 244, 7, 0, 12, 19, 242, 0, 2, 5, 18, 220, 8, 15, 144, 5, 140, 14,
            216, 15, 17, 141, 9, 251, 240, 5, 2, 5, 18, 250,
        ],
    );
}

#[test]
fn python_312_dynamic_while_keeps_protected_pass() {
    // Exact bytecode and tables from the pinned native CPython target.
    for (source, bytecode, line_table, exception_table) in [
        (
            "async def f():\n    async with manager():\n        while await ready():\n            pass\n",
            &[
                75, 0, 1, 0, 151, 0, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 52,
                0, 131, 1, 100, 0, 123, 3, 0, 0, 150, 2, 151, 3, 134, 5, 5, 0, 1, 0, 116, 3, 0, 0,
                0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 131, 0, 100, 0, 123, 3, 0, 0, 150, 3,
                151, 3, 134, 5, 5, 0, 114, 20, 9, 0, 116, 3, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0,
                0, 0, 0, 0, 131, 0, 100, 0, 123, 3, 0, 0, 150, 3, 151, 3, 134, 5, 5, 0, 114, 1,
                140, 20, 100, 0, 100, 0, 100, 0, 171, 2, 0, 0, 0, 0, 0, 0, 131, 2, 100, 0, 123, 3,
                0, 0, 150, 2, 151, 3, 134, 5, 5, 0, 1, 0, 121, 0, 55, 0, 140, 59, 55, 0, 140, 43,
                55, 0, 140, 26, 55, 0, 140, 11, 35, 0, 49, 0, 131, 2, 100, 0, 123, 4, 0, 0, 150, 3,
                151, 3, 134, 5, 55, 0, 5, 0, 115, 1, 119, 2, 1, 0, 89, 0, 1, 0, 1, 0, 121, 0, 120,
                3, 89, 0, 119, 1, 173, 3, 119, 1,
            ][..],
            &[
                232, 0, 248, 128, 0, 220, 15, 22, 143, 121, 141, 121, 220, 20, 25, 147, 71, 143,
                109, 136, 109, 216, 12, 16, 244, 3, 0, 21, 26, 147, 71, 143, 109, 137, 109, 247, 3,
                0, 16, 25, 143, 121, 137, 121, 248, 216, 14, 27, 248, 136, 109, 248, 240, 3, 0, 16,
                25, 248, 143, 121, 143, 121, 137, 121, 252,
            ][..],
            &[
                130, 15, 65, 42, 1, 145, 1, 65, 13, 6, 146, 3, 65, 42, 1, 149, 14, 65, 21, 3, 163,
                1, 65, 15, 6, 164, 18, 65, 21, 3, 182, 1, 65, 17, 6, 183, 4, 65, 21, 3, 188, 11,
                65, 42, 1, 193, 7, 1, 65, 19, 4, 193, 8, 6, 65, 42, 1, 193, 15, 1, 65, 21, 3, 193,
                17, 1, 65, 21, 3, 193, 19, 1, 65, 42, 1, 193, 21, 6, 65, 39, 7, 193, 27, 1, 65, 30,
                12, 193, 28, 7, 65, 39, 7, 193, 35, 7, 65, 42, 1,
            ][..],
        ),
        (
            "def f():\n    with manager():\n        while ready():\n            pass\n",
            &[
                151, 0, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 53, 0, 1, 0, 116,
                3, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 114, 12, 9, 0, 116, 3, 0, 0,
                0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 114, 1, 140, 12, 100, 0, 100, 0, 100,
                0, 171, 2, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0, 35, 0, 49, 0, 115, 1, 119, 2, 1, 0, 89,
                0, 1, 0, 1, 0, 121, 0, 120, 3, 89, 0, 119, 1,
            ][..],
            &[
                128, 0, 220, 9, 16, 141, 25, 220, 14, 19, 140, 103, 216, 12, 16, 244, 3, 0, 15, 20,
                141, 103, 247, 3, 0, 10, 19, 143, 25, 137, 25, 250,
            ][..],
            &[139, 22, 43, 3, 171, 5, 52, 7][..],
        ),
    ] {
        let module = compile(source, PythonVersion::Py312);
        let code = child(&module, "f");
        assert_eq!(code.bytecode, bytecode, "{source}");
        assert_eq!(code.line_table, line_table, "{source}");
        assert_eq!(code.exception_table, exception_table, "{source}");
    }
}

#[test]
fn python_313_dynamic_while_keeps_protected_pass() {
    // Exact bytecode and tables from the pinned native CPython target.
    for (source, bytecode, line_table, exception_table) in [
        (
            "async def f():\n    async with manager():\n        while await ready():\n            pass\n",
            &[
                35, 0, 32, 0, 149, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 1, 0,
                73, 1, 83, 0, 104, 3, 0, 0, 118, 1, 149, 3, 78, 5, 12, 0, 32, 0, 91, 3, 0, 0, 0, 0,
                0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 73, 0, 83, 0, 104, 3, 0, 0, 118, 1, 149, 3,
                78, 5, 12, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 26, 0, 0, 30, 0, 91, 3, 0, 0, 0, 0, 0,
                0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 73, 0, 83, 0, 104, 3, 0, 0, 118, 1, 149, 3, 78,
                5, 12, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 2, 0, 0, 77, 26, 0, 0, 83, 0, 83, 0, 83, 0,
                53, 2, 0, 0, 0, 0, 0, 0, 73, 2, 83, 0, 104, 3, 0, 0, 118, 1, 149, 3, 78, 5, 12, 0,
                32, 0, 103, 0, 8, 0, 78, 70, 8, 0, 78, 54, 8, 0, 78, 32, 8, 0, 78, 11, 33, 0, 44,
                0, 73, 2, 83, 0, 104, 4, 0, 0, 118, 1, 149, 3, 78, 5, 8, 0, 12, 0, 40, 0, 0, 0, 0,
                0, 0, 0, 100, 1, 0, 0, 102, 2, 32, 0, 31, 0, 32, 0, 32, 0, 103, 0, 61, 3, 31, 0,
                102, 1, 55, 3, 102, 1,
            ][..],
            &[
                233, 0, 128, 0, 220, 15, 22, 143, 121, 141, 121, 220, 20, 25, 147, 71, 143, 109,
                141, 109, 216, 12, 16, 244, 3, 0, 21, 26, 147, 71, 143, 109, 143, 109, 247, 3, 0,
                16, 25, 143, 121, 138, 121, 219, 14, 27, 247, 3, 0, 16, 25, 143, 121, 143, 121,
                136, 121, 252,
            ][..],
            &[
                130, 15, 65, 58, 1, 145, 1, 65, 24, 6, 146, 3, 65, 58, 1, 149, 14, 65, 32, 3, 163,
                1, 65, 26, 6, 164, 23, 65, 32, 3, 187, 1, 65, 28, 6, 188, 9, 65, 32, 3, 193, 7, 11,
                65, 58, 1, 193, 18, 1, 65, 30, 4, 193, 19, 6, 65, 58, 1, 193, 26, 1, 65, 32, 3,
                193, 28, 1, 65, 32, 3, 193, 30, 1, 65, 58, 1, 193, 32, 6, 65, 55, 7, 193, 38, 1,
                65, 41, 12, 193, 39, 12, 65, 55, 7, 193, 51, 7, 65, 58, 1,
            ][..],
        ),
        (
            "def f():\n    with manager():\n        while ready():\n            pass\n",
            &[
                149, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 2, 0, 32, 0, 91, 3,
                0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 18,
                0, 0, 30, 0, 91, 3, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 40, 0, 0, 0,
                0, 0, 0, 0, 97, 2, 0, 0, 77, 18, 0, 0, 83, 0, 83, 0, 83, 0, 53, 2, 0, 0, 0, 0, 0,
                0, 32, 0, 103, 0, 33, 0, 44, 0, 40, 0, 0, 0, 0, 0, 0, 0, 100, 1, 0, 0, 102, 2, 32,
                0, 31, 0, 32, 0, 32, 0, 103, 0, 61, 3, 31, 0, 102, 1,
            ][..],
            &[
                128, 0, 220, 9, 16, 141, 25, 220, 14, 19, 143, 103, 137, 103, 216, 12, 16, 244, 3,
                0, 15, 20, 143, 103, 139, 103, 247, 3, 0, 10, 19, 143, 25, 142, 25, 250,
            ][..],
            &[139, 32, 54, 3, 182, 10, 65, 4, 7][..],
        ),
    ] {
        let module = compile(source, PythonVersion::Py313);
        let code = child(&module, "f");
        assert_eq!(code.bytecode, bytecode, "{source}");
        assert_eq!(code.line_table, line_table, "{source}");
        assert_eq!(code.exception_table, exception_table, "{source}");
    }
}

#[test]
fn python_312_inner_try_return_keeps_original_handler_exit() {
    // Exact bytecode and tables from the pinned native CPython target.
    {
        let (source, bytecode, line_table, exception_table) = (
            "def f(flag, value):\n    try:\n        work()\n    except Failure:\n        try:\n            return value if flag else None\n        except OtherFailure:\n            return None\n",
            &[
                151, 0, 9, 0, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 121,
                0, 35, 0, 116, 2, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 26, 1, 0, 9, 0, 124, 0, 114,
                2, 124, 1, 110, 1, 100, 0, 99, 2, 89, 0, 83, 0, 35, 0, 116, 4, 0, 0, 0, 0, 0, 0, 0,
                0, 36, 0, 114, 4, 1, 0, 89, 0, 89, 0, 121, 0, 119, 0, 120, 3, 89, 0, 119, 1, 119,
                0, 120, 3, 89, 0, 119, 1,
            ][..],
            &[
                128, 0, 240, 2, 6, 5, 24, 220, 8, 12, 141, 6, 248, 220, 11, 18, 242, 0, 4, 5, 24,
                240, 2, 3, 9, 24, 217, 28, 32, 145, 53, 160, 100, 210, 12, 42, 248, 220, 15, 27,
                242, 0, 1, 9, 24, 218, 19, 23, 240, 3, 1, 9, 24, 250, 240, 7, 4, 5, 24, 250,
            ][..],
            &[
                130, 10, 13, 0, 141, 9, 48, 3, 151, 5, 31, 2, 156, 1, 48, 3, 159, 9, 44, 5, 168, 1,
                48, 3, 171, 1, 44, 5, 172, 4, 48, 3,
            ][..],
        );
        let module = compile(source, PythonVersion::Py312);
        let code = child(&module, "f");
        assert_eq!(code.bytecode, bytecode, "{source}");
        assert_eq!(code.line_table, line_table, "{source}");
        assert_eq!(code.exception_table, exception_table, "{source}");
    }
}

#[test]
fn python_313_inner_try_return_keeps_original_handler_exit() {
    // Exact bytecode and tables from the pinned native CPython target.
    {
        let (source, bytecode, line_table, exception_table) = (
            "def f(flag, value):\n    try:\n        work()\n    except Failure:\n        try:\n            return value if flag else None\n        except OtherFailure:\n            return None\n",
            &[
                149, 0, 30, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 103,
                0, 33, 0, 91, 2, 0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 97, 32, 0, 0, 32, 0, 30, 0, 85, 0,
                40, 0, 0, 0, 0, 0, 0, 0, 97, 2, 0, 0, 85, 1, 79, 1, 83, 0, 115, 2, 31, 0, 36, 0,
                33, 0, 91, 4, 0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 97, 4, 0, 0, 32, 0, 31, 0, 31, 0, 103,
                0, 102, 0, 61, 3, 31, 0, 102, 1, 102, 0, 61, 3, 31, 0, 102, 1,
            ][..],
            &[
                128, 0, 240, 2, 6, 5, 24, 220, 8, 12, 141, 6, 248, 220, 11, 18, 243, 0, 4, 5, 24,
                240, 2, 3, 9, 24, 222, 28, 32, 145, 53, 160, 100, 210, 12, 42, 248, 220, 15, 27,
                243, 0, 1, 9, 24, 218, 19, 23, 240, 3, 1, 9, 24, 250, 240, 7, 4, 5, 24, 250,
            ][..],
            &[
                130, 10, 13, 0, 141, 10, 55, 3, 152, 10, 37, 2, 162, 1, 55, 3, 165, 10, 51, 5, 175,
                1, 55, 3, 178, 1, 51, 5, 179, 4, 55, 3,
            ][..],
        );
        let module = compile(source, PythonVersion::Py313);
        let code = child(&module, "f");
        assert_eq!(code.bytecode, bytecode, "{source}");
        assert_eq!(code.line_table, line_table, "{source}");
        assert_eq!(code.exception_table, exception_table, "{source}");
    }
}

#[test]
fn python_313_normal_handler_exit_counts_original_pop() {
    // Exact bytecode and tables from the pinned native CPython target.
    for (source, bytecode, line_table, exception_table) in [
        (
            "def f(flag, value):\n    try:\n        setup()\n    except Failure:\n        if flag:\n            with manager():\n                work()\n        else:\n            with manager():\n                recover()\n    return value\n",
            &[
                149, 0, 30, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 85,
                1, 36, 0, 33, 0, 91, 2, 0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 97, 109, 0, 0, 32, 0, 85, 0,
                40, 0, 0, 0, 0, 0, 0, 0, 97, 47, 0, 0, 91, 5, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0,
                0, 0, 0, 0, 2, 0, 32, 0, 91, 7, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0,
                32, 0, 83, 0, 83, 0, 83, 0, 53, 2, 0, 0, 0, 0, 0, 0, 32, 0, 79, 68, 33, 0, 44, 0,
                40, 0, 0, 0, 0, 0, 0, 0, 100, 1, 0, 0, 102, 2, 32, 0, 31, 0, 32, 0, 32, 0, 79, 54,
                61, 3, 31, 0, 102, 1, 91, 5, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 2, 0,
                32, 0, 91, 9, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 83, 0, 83, 0,
                83, 0, 53, 2, 0, 0, 0, 0, 0, 0, 32, 0, 31, 0, 85, 1, 36, 0, 33, 0, 44, 0, 40, 0, 0,
                0, 0, 0, 0, 0, 100, 1, 0, 0, 102, 2, 32, 0, 31, 0, 32, 0, 32, 0, 31, 0, 85, 1, 36,
                0, 61, 3, 31, 0, 102, 1, 31, 0, 85, 1, 36, 0, 102, 0, 61, 3, 31, 0, 102, 1,
            ][..],
            &[
                128, 0, 240, 2, 8, 5, 26, 220, 8, 13, 140, 7, 240, 16, 0, 12, 17, 128, 76, 248,
                244, 15, 0, 12, 19, 243, 0, 6, 5, 26, 222, 11, 15, 220, 17, 24, 149, 25, 220, 16,
                20, 148, 6, 247, 3, 0, 18, 27, 151, 25, 150, 25, 250, 244, 6, 0, 18, 25, 149, 25,
                220, 16, 23, 148, 9, 247, 3, 0, 18, 27, 144, 25, 224, 11, 16, 128, 76, 247, 5, 0,
                18, 27, 149, 25, 224, 11, 16, 128, 76, 251, 136, 53, 128, 76, 240, 15, 6, 5, 26,
                250,
            ][..],
            &[
                130, 10, 14, 0, 142, 27, 66, 5, 3, 169, 11, 61, 5, 180, 9, 66, 5, 3, 189, 10, 65,
                11, 9, 193, 7, 17, 66, 5, 3, 193, 24, 11, 65, 46, 5, 193, 35, 8, 66, 5, 3, 193, 46,
                10, 65, 62, 9, 193, 56, 3, 66, 5, 3, 193, 62, 3, 66, 5, 3, 194, 4, 1, 66, 5, 3,
            ][..],
        ),
        (
            "def f(flag, value):\n    try:\n        setup()\n    except Failure:\n        try:\n            with manager():\n                work()\n        except OtherFailure:\n            recover()\n    return value\n",
            &[
                149, 0, 30, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 85,
                1, 36, 0, 33, 0, 91, 2, 0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 97, 80, 0, 0, 32, 0, 30, 0,
                91, 5, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 2, 0, 32, 0, 91, 7, 0, 0,
                0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 83, 0, 83, 0, 83, 0, 53, 2, 0, 0,
                0, 0, 0, 0, 32, 0, 79, 45, 33, 0, 44, 0, 40, 0, 0, 0, 0, 0, 0, 0, 100, 1, 0, 0,
                102, 2, 32, 0, 31, 0, 32, 0, 32, 0, 79, 31, 61, 3, 31, 0, 102, 1, 33, 0, 91, 8, 0,
                0, 0, 0, 0, 0, 0, 0, 7, 0, 97, 15, 0, 0, 32, 0, 91, 11, 0, 0, 0, 0, 0, 0, 0, 0, 53,
                0, 0, 0, 0, 0, 0, 0, 32, 0, 31, 0, 31, 0, 85, 1, 36, 0, 102, 0, 61, 3, 31, 0, 102,
                1, 31, 0, 85, 1, 36, 0, 102, 0, 61, 3, 31, 0, 102, 1,
            ][..],
            &[
                128, 0, 240, 2, 7, 5, 22, 220, 8, 13, 140, 7, 240, 14, 0, 12, 17, 128, 76, 248,
                244, 13, 0, 12, 19, 243, 0, 5, 5, 22, 240, 2, 4, 9, 22, 220, 17, 24, 149, 25, 220,
                16, 20, 148, 6, 247, 3, 0, 18, 27, 151, 25, 150, 25, 251, 228, 15, 27, 243, 0, 1,
                9, 22, 220, 12, 19, 142, 73, 216, 11, 16, 128, 76, 240, 5, 1, 9, 22, 251, 224, 11,
                16, 128, 76, 240, 13, 5, 5, 22, 250,
            ][..],
            &[
                130, 10, 14, 0, 142, 10, 65, 40, 3, 153, 10, 65, 8, 2, 163, 11, 55, 5, 174, 8, 65,
                8, 2, 182, 1, 65, 40, 3, 183, 10, 65, 5, 9, 193, 1, 3, 65, 8, 2, 193, 4, 1, 65, 40,
                3, 193, 5, 3, 65, 8, 2, 193, 8, 20, 65, 33, 5, 193, 28, 1, 65, 40, 3, 193, 32, 1,
                65, 33, 5, 193, 33, 3, 65, 40, 3, 193, 39, 1, 65, 40, 3,
            ][..],
        ),
    ] {
        let module = compile(source, PythonVersion::Py313);
        let code = child(&module, "f");
        assert_eq!(code.bytecode, bytecode, "{source}");
        assert_eq!(code.line_table, line_table, "{source}");
        assert_eq!(code.exception_table, exception_table, "{source}");
    }
}

#[test]
fn legacy_later_exception_group_handler_keeps_stack_prefix() {
    // Pinned CPython 3.12 and 3.13 both need nine stack entries for the
    // nested call in the second handler, including its retained group state.
    let source = "def f():\n    try:\n        work()\n    except* First:\n        mark()\n    except* Exception as error:\n        use(type(error), error)\n";
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        let module = compile(source, version);
        assert_eq!(child(&module, "f").stack_size, 9);
    }
}

#[test]
fn python313_with_star_finally_keeps_normal_pop_block() {
    let module = compile(
        "async def f(manager, work):\n    with manager:\n        try:\n            await work()\n        except* Exception:\n            pass\n        finally:\n            finish()\n    report()\n",
        PythonVersion::Py313,
    );
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        [
            35, 0, 32, 0, 149, 0, 85, 0, 2, 0, 32, 0, 30, 0, 85, 1, 34, 0, 53, 0, 0, 0, 0, 0, 0, 0,
            73, 0, 83, 0, 104, 3, 0, 0, 118, 1, 149, 3, 78, 5, 12, 0, 32, 0, 91, 3, 0, 0, 0, 0, 0,
            0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 30, 0, 83, 0, 83, 0, 83, 0, 53, 2, 0, 0, 0, 0,
            0, 0, 32, 0, 91, 5, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 103, 0, 8,
            0, 78, 34, 33, 0, 47, 0, 61, 2, 91, 0, 0, 0, 0, 0, 0, 0, 0, 0, 6, 0, 61, 1, 98, 7, 0,
            0, 32, 0, 80, 1, 79, 6, 80, 3, 32, 0, 80, 1, 79, 2, 32, 0, 80, 1, 56, 1, 61, 1, 99, 3,
            0, 0, 32, 0, 31, 0, 78, 60, 115, 2, 31, 0, 102, 0, 61, 3, 31, 0, 102, 1, 33, 0, 91, 3,
            0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 102, 0, 61, 3, 31, 0, 102, 1,
            33, 0, 44, 0, 40, 0, 0, 0, 0, 0, 0, 0, 100, 1, 0, 0, 102, 2, 32, 0, 31, 0, 32, 0, 32,
            0, 78, 76, 61, 3, 31, 0, 102, 1, 55, 3, 102, 1,
        ],
    );
    assert_eq!(
        function.line_table,
        [
            233, 0, 128, 0, 218, 9, 16, 240, 2, 5, 9, 21, 217, 18, 22, 147, 38, 143, 76, 136, 76,
            244, 8, 0, 13, 19, 141, 72, 247, 13, 0, 10, 17, 244, 14, 0, 5, 11, 133, 72, 241, 11, 0,
            13, 25, 248, 241, 2, 1, 9, 17, 148, 9, 244, 0, 1, 9, 17, 217, 12, 16, 251, 241, 3, 1,
            9, 17, 255, 253, 244, 6, 0, 13, 19, 141, 72, 250, 247, 13, 0, 10, 17, 141, 23, 252,
        ],
    );
    assert_eq!(
        function.exception_table,
        [
            130, 3, 65, 56, 1, 133, 1, 65, 39, 3, 135, 10, 54, 2, 145, 1, 52, 6, 146, 4, 54, 2,
            150, 10, 65, 39, 3, 161, 19, 65, 56, 1, 180, 1, 54, 2, 182, 26, 65, 21, 5, 193, 16, 8,
            65, 24, 2, 193, 24, 12, 65, 36, 5, 193, 36, 3, 65, 39, 3, 193, 39, 10, 65, 53, 7, 193,
            49, 7, 65, 56, 1,
        ],
    );
}

#[test]
fn python313_with_while_raise_keeps_normal_pop_block() {
    let module = compile(
        "def f(manager, ready):\n    with manager:\n        while ready():\n            raise ValueError()\n    finish()\n",
        PythonVersion::Py313,
    );
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        [
            149, 0, 85, 0, 2, 0, 32, 0, 85, 1, 34, 0, 53, 0, 0, 0, 0, 0, 0, 0, 40, 0, 0, 0, 0, 0,
            0, 0, 97, 10, 0, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 101, 1, 30,
            0, 83, 0, 83, 0, 83, 0, 53, 2, 0, 0, 0, 0, 0, 0, 32, 0, 91, 3, 0, 0, 0, 0, 0, 0, 0, 0,
            53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 103, 0, 33, 0, 44, 0, 40, 0, 0, 0, 0, 0, 0, 0, 100, 1,
            0, 0, 102, 2, 32, 0, 31, 0, 32, 0, 32, 0, 78, 25, 61, 3, 31, 0, 102, 1,
        ],
    );
    assert_eq!(
        function.line_table,
        [
            128, 0, 218, 9, 16, 217, 14, 19, 143, 103, 137, 103, 220, 18, 28, 147, 44, 208, 12, 30,
            240, 3, 0, 15, 22, 247, 3, 0, 10, 17, 244, 6, 0, 5, 11, 133, 72, 247, 7, 0, 10, 17,
            141, 23, 250,
        ],
    );
    assert_eq!(function.exception_table, [131, 23, 46, 3, 174, 10, 60, 7,],);
}

#[test]
fn python_313_terminal_break_retains_native_try_pop_copy_cost() {
    let module = compile(
        "def f(manager):\n    while True:\n        try:\n            with manager:\n                work()\n            break\n        except Error:\n            retry()\n",
        PythonVersion::Py313,
    );
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        [
            149, 0, 30, 0, 30, 0, 85, 0, 2, 0, 32, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0,
            0, 0, 0, 0, 32, 0, 83, 0, 83, 0, 83, 0, 53, 2, 0, 0, 0, 0, 0, 0, 32, 0, 103, 0, 33, 0,
            44, 0, 40, 0, 0, 0, 0, 0, 0, 0, 100, 1, 0, 0, 102, 2, 32, 0, 31, 0, 32, 0, 32, 0, 78,
            15, 61, 3, 31, 0, 102, 1, 33, 0, 91, 2, 0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 97, 13, 0, 0, 32,
            0, 91, 5, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 31, 0, 79, 4, 102, 0,
            61, 3, 31, 0, 102, 1, 77, 68, 0, 0
        ]
    );
    assert_eq!(
        function.exception_table,
        [
            131, 2, 42, 0, 133, 11, 25, 3, 144, 8, 42, 0, 153, 10, 39, 7, 163, 7, 42, 0, 170, 20,
            65, 1, 3, 193, 0, 1, 65, 1, 3
        ]
    );
    assert_eq!(
        function.line_table,
        [
            128, 0, 216, 10, 14, 240, 2, 5, 9, 20, 218, 17, 24, 220, 16, 20, 148, 6, 247, 3, 0, 18,
            25, 224, 12, 17, 247, 5, 0, 18, 25, 149, 23, 251, 244, 6, 0, 16, 21, 243, 0, 1, 9, 20,
            220, 12, 17, 142, 71, 240, 3, 1, 9, 20, 250, 241, 11, 0, 11, 15
        ]
    );
}

#[test]
fn python_313_folded_nested_loop_does_not_locate_the_finally_pop() {
    // CPython 3.13.15 resolves the original constant-loop false edge before
    // folding it. The shared normal POP_BLOCK remains unlocated and disappears.
    let module = compile(
        "def f(flag):\n    try:\n        if flag:\n            while True:\n                work()\n    finally:\n        finish()\n",
        PythonVersion::Py313,
    );
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        [
            149, 0, 30, 0, 85, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 13, 0, 0, 30, 0, 91, 1, 0, 0, 0, 0,
            0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 77, 12, 0, 0, 91, 3, 0, 0, 0, 0, 0, 0, 0,
            0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 103, 0, 33, 0, 91, 3, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0,
            0, 0, 0, 0, 0, 0, 32, 0, 102, 0, 61, 3, 31, 0, 102, 1
        ]
    );
    assert_eq!(
        function.line_table,
        [
            128, 0, 240, 2, 5, 5, 17, 222, 11, 15, 216, 18, 22, 220, 16, 20, 148, 6, 241, 3, 0, 19,
            23, 244, 6, 0, 9, 15, 141, 8, 248, 140, 6, 141, 8, 250
        ]
    );
    assert_eq!(function.exception_table, [130, 20, 33, 0, 161, 12, 45, 3]);
}

#[test]
fn python312_retained_pop_block_has_no_exception_owner() {
    // Native 3.12.14 retains the successful assertion's POP_BLOCK as a NOP.
    // The enclosing coroutine cleanup does not own that original pseudo-op.
    let module = compile(
        "# Unexecuted public reducer: assertion success targets ownerless POP_BLOCK.\nasync def parse(step, valid):\n    initial = await step()\n    try:\n        result = transform(initial)\n        assert valid(result)\n    except ValueError as error:\n        record(error)\n        return None\n    return result\n",
        PythonVersion::Py312,
    );
    assert_eq!(
        child(&module, "parse").exception_table,
        [
            130, 11, 65, 14, 1, 141, 1, 43, 4, 142, 4, 65, 14, 1, 147, 21, 45, 0, 169, 3, 65, 14,
            1, 173, 9, 65, 11, 3, 182, 11, 65, 6, 3, 193, 1, 5, 65, 14, 1, 193, 6, 5, 65, 11, 3,
            193, 11, 3, 65, 14, 1
        ]
    );
}

#[test]
fn python313_exception_owners_follow_original_and_threaded_jumps() {
    // CPython 3.13.15 keeps manager exits under their surrounding try, while
    // threading a folded false loop condition appends a fresh ownerless jump.
    for (source, exception_table) in [
        (
            "async def f(values):\n    async for value in values:\n        if False:\n            yield value\n",
            &[
                130, 3, 20, 1, 133, 4, 18, 2, 137, 1, 16, 6, 138, 3, 18, 2, 141, 1, 20, 1, 144, 1,
                18, 2, 146, 2, 20, 1,
            ][..],
        ),
        (
            "def f(flag, manager, check, action):\n    try:\n        if flag:\n            with manager:\n                value = check()\n                if value:\n                    return 1\n                action()\n        else:\n            action()\n    except RuntimeError:\n        return 2\n    return 3\n",
            &[
                130, 9, 65, 13, 0, 139, 16, 60, 3, 155, 8, 65, 13, 0, 164, 7, 60, 3, 171, 16, 65,
                13, 0, 188, 10, 65, 10, 7, 193, 6, 7, 65, 13, 0, 193, 13, 10, 65, 26, 3, 193, 25,
                1, 65, 26, 3,
            ][..],
        ),
    ] {
        let module = compile(source, PythonVersion::Py313);
        assert_eq!(
            child(&module, "f").exception_table,
            exception_table,
            "{source}"
        );
    }
}

#[test]
fn legacy_break_threads_the_join_after_copied_finally_cleanup() {
    // The authored break NOP precedes the copied cleanup. Its trailing guard join
    // is a fresh native jump target, so the guard can thread directly to the exit.
    // Bytes captured from pinned CPython 3.12.14 and 3.13.15.
    let source = "def f(value, stop, cleanup, after):\n    while True:\n        try:\n            if stop:\n                break\n            stop = True\n        finally:\n            if value is not None:\n                cleanup()\n    after()\n";
    for (version, bytecode, line_table, exception_table) in [
        (
            PythonVersion::Py312,
            &[
                151, 0, 9, 0, 9, 0, 124, 1, 114, 11, 9, 0, 124, 0, 129, 21, 2, 0, 124, 2, 171, 0,
                0, 0, 0, 0, 0, 0, 1, 0, 110, 13, 100, 1, 125, 1, 124, 0, 129, 8, 2, 0, 124, 2, 171,
                0, 0, 0, 0, 0, 0, 0, 1, 0, 9, 0, 140, 27, 2, 0, 124, 3, 171, 0, 0, 0, 0, 0, 0, 0,
                1, 0, 121, 0, 35, 0, 124, 0, 129, 8, 2, 0, 124, 2, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0,
                119, 0, 119, 0, 120, 3, 89, 0, 119, 1,
            ][..],
            &[
                128, 0, 216, 10, 14, 240, 2, 6, 9, 26, 217, 15, 19, 216, 16, 21, 240, 6, 0, 16, 21,
                208, 15, 32, 217, 16, 23, 149, 9, 240, 7, 0, 20, 24, 136, 68, 224, 15, 20, 208, 15,
                32, 217, 16, 23, 149, 9, 240, 15, 0, 11, 15, 241, 16, 0, 5, 10, 133, 71, 248, 240,
                5, 0, 16, 21, 208, 15, 32, 217, 16, 23, 149, 9, 240, 3, 0, 16, 33, 250,
            ][..],
            &[131, 2, 37, 0, 144, 2, 37, 0, 165, 12, 49, 3][..],
        ),
        (
            PythonVersion::Py313,
            &[
                149, 0, 30, 0, 30, 0, 85, 1, 40, 0, 0, 0, 0, 0, 0, 0, 97, 12, 0, 0, 30, 0, 85, 0,
                98, 22, 0, 0, 85, 2, 34, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 79, 14, 83, 1, 110, 1,
                85, 0, 98, 7, 0, 0, 85, 2, 34, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 77, 34, 0, 0, 85,
                3, 34, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 103, 0, 33, 0, 85, 0, 98, 8, 0, 0, 85, 2,
                34, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 102, 0, 102, 0, 61, 3, 31, 0, 102, 1,
            ][..],
            &[
                128, 0, 216, 10, 14, 240, 2, 6, 9, 26, 222, 15, 19, 216, 16, 21, 240, 6, 0, 16, 21,
                209, 15, 32, 217, 16, 23, 149, 9, 240, 7, 0, 20, 24, 136, 68, 224, 15, 20, 209, 15,
                32, 217, 16, 23, 148, 9, 241, 15, 0, 11, 15, 241, 16, 0, 5, 10, 133, 71, 248, 240,
                5, 0, 16, 21, 209, 15, 32, 217, 16, 23, 149, 9, 240, 3, 0, 16, 33, 250,
            ][..],
            &[131, 7, 44, 0, 150, 2, 44, 0, 172, 13, 57, 3][..],
        ),
    ] {
        let module = compile(source, version);
        let function = child(&module, "f");
        assert_eq!(function.bytecode, bytecode, "{version:?}");
        assert_eq!(function.line_table, line_table, "{version:?}");
        assert_eq!(function.exception_table, exception_table, "{version:?}");
    }
}

#[test]
fn python312_manager_handler_pass_donates_cleanup_location() {
    let module = compile(
        "def f(manager, flag):\n    with manager:\n        try:\n            work()\n        except Failure as error:\n            if flag:\n                raise Other from error\n            pass\n    finish()\n",
        PythonVersion::Py312,
    );
    assert_eq!(
        child(&module, "f").line_table,
        [
            128, 0, 218, 9, 16, 240, 2, 5, 9, 17, 220, 12, 16, 140, 70, 247, 5, 0, 10, 17, 244, 14,
            0, 5, 11, 133, 72, 248, 244, 9, 0, 16, 23, 242, 0, 3, 9, 17, 217, 15, 19, 220, 22, 27,
            160, 21, 208, 16, 38, 220, 12, 16, 251, 240, 7, 3, 9, 17, 250, 247, 7, 0, 10, 17, 244,
            14, 0, 5, 11, 133, 72, 250,
        ],
    );
}

#[test]
fn python313_manager_handler_pass_donates_cleanup_location() {
    let module = compile(
        "def f(manager, flag):\n    with manager:\n        try:\n            work()\n        except Failure as error:\n            if flag:\n                raise Other from error\n            pass\n    finish()\n",
        PythonVersion::Py313,
    );
    assert_eq!(
        child(&module, "f").line_table,
        [
            128, 0, 218, 9, 16, 240, 2, 5, 9, 17, 220, 12, 16, 140, 70, 247, 5, 0, 10, 17, 244, 14,
            0, 5, 11, 133, 72, 248, 244, 9, 0, 16, 23, 243, 0, 3, 9, 17, 222, 15, 19, 220, 22, 27,
            160, 21, 208, 16, 38, 220, 12, 16, 251, 240, 7, 3, 9, 17, 250, 247, 7, 0, 10, 17, 141,
            23, 250,
        ],
    );
}

#[test]
fn python312_handler_manager_pass_donates_pop_block_location() {
    let module = compile(
        "def f(manager, flag):\n    try:\n        work()\n    except Failure as error:\n        with manager:\n            if flag:\n                raise Other from error\n            pass\n    finish()\n",
        PythonVersion::Py312,
    );
    assert_eq!(
        child(&module, "f").line_table,
        [
            128, 0, 240, 2, 6, 5, 17, 220, 8, 12, 140, 6, 244, 12, 0, 5, 11, 133, 72, 248, 244, 11,
            0, 12, 19, 242, 0, 4, 5, 17, 218, 13, 20, 217, 15, 19, 220, 22, 27, 160, 21, 208, 16,
            38, 216, 12, 16, 247, 7, 0, 14, 21, 143, 87, 137, 87, 254, 244, 8, 0, 5, 11, 133, 72,
            251, 240, 11, 4, 5, 17, 250,
        ],
    );
}

#[test]
fn python313_handler_manager_pass_donates_pop_block_location() {
    let module = compile(
        "def f(manager, flag):\n    try:\n        work()\n    except Failure as error:\n        with manager:\n            if flag:\n                raise Other from error\n            pass\n    finish()\n",
        PythonVersion::Py313,
    );
    assert_eq!(
        child(&module, "f").line_table,
        [
            128, 0, 240, 2, 6, 5, 17, 220, 8, 12, 140, 6, 244, 12, 0, 5, 11, 133, 72, 248, 244, 11,
            0, 12, 19, 243, 0, 4, 5, 17, 218, 13, 20, 222, 15, 19, 220, 22, 27, 160, 21, 208, 16,
            38, 216, 12, 16, 247, 7, 0, 14, 21, 143, 87, 143, 87, 143, 87, 254, 240, 3, 4, 5, 17,
            250,
        ],
    );
}

#[test]
fn python313_guarded_finally_loop_keeps_its_unlocated_shared_exit() {
    // Before folding, the while's constant-false edge still enters this shared
    // finalizer exit. The guard alone must not donate a source line to its jump.
    // Captured from pinned CPython 3.13.15.
    let module = compile(
        "def f(flag, action, after):\n    try:\n        action()\n    finally:\n        if flag:\n            while True:\n                action()\n                break\n    after()\n",
        PythonVersion::Py313,
    );
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        [
            149, 0, 30, 0, 85, 1, 34, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 85, 0, 40, 0, 0, 0, 0, 0,
            0, 0, 97, 9, 0, 0, 30, 0, 85, 1, 34, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 30, 0, 85, 2,
            34, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 103, 0, 33, 0, 85, 0, 40, 0, 0, 0, 0, 0, 0, 0,
            97, 9, 0, 0, 30, 0, 85, 1, 34, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 102, 0, 102, 0, 61,
            3, 31, 0, 102, 1,
        ],
    );
    assert_eq!(
        function.line_table,
        [
            128, 0, 240, 2, 6, 5, 22, 217, 8, 14, 140, 8, 230, 11, 15, 216, 18, 22, 217, 16, 22,
            148, 8, 216, 16, 21, 217, 4, 9, 133, 71, 248, 246, 9, 0, 12, 16, 216, 18, 22, 217, 16,
            22, 148, 8, 216, 16, 21, 240, 7, 0, 12, 16, 250,
        ],
    );
    assert_eq!(function.exception_table, [130, 7, 33, 0, 161, 18, 51, 3,],);
}

#[test]
fn legacy_async_comprehension_iterables_fold_before_constant_admission() {
    // CPython 3.12.14 / 3.13.15 fold async-comprehension iterables into tuples.
    // A discarded tuple does not admit its members as scalar constants.
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        let dead = compile(
            "async def outer():\n    if False:\n        dead = [x async for x in [41, 42]]\n    marker = 99\n    return 41\n",
            version,
        );
        assert!(matches!(
            child(&dead, "outer").constants.as_slice(),
            [Constant::None, Constant::Int(99), Constant::Int(41)]
        ));
        let live = compile(
            "async def outer():\n    return [x async for x in [41, 42]]\n",
            version,
        );
        assert!(
            matches!(child(&live, "outer").constants.as_slice(), [Constant::None, Constant::Tuple(values)] if matches!(values.as_slice(), [Constant::Int(41), Constant::Int(42)]))
        );
    }
}

#[test]
fn python_312_terminal_try_conditional_preserves_copy_rounds() {
    let module = compile(
        "def f(choice, flag, action, other):\n    try:\n        if choice:\n            action() if flag else other()\n        elif flag:\n            other()\n        else:\n            action()\n    except RuntimeError:\n        recover()\n",
        PythonVersion::Py312,
    );
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        [
            151, 0, 9, 0, 124, 0, 114, 19, 124, 1, 114, 7, 2, 0, 124, 2, 171, 0, 0, 0, 0, 0, 0, 0,
            110, 8, 2, 0, 124, 3, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0, 1, 0, 121, 0, 124, 1,
            114, 8, 2, 0, 124, 3, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0, 2, 0, 124, 2, 171, 0, 0,
            0, 0, 0, 0, 0, 1, 0, 121, 0, 35, 0, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 13, 1,
            0, 116, 3, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 89, 0, 121, 0, 119,
            0, 120, 3, 89, 0, 119, 1
        ]
    );
    assert_eq!(
        function.exception_table,
        [
            130, 18, 41, 0, 149, 1, 41, 0, 151, 9, 41, 0, 161, 7, 41, 0, 169, 19, 63, 3, 190, 1,
            63, 3
        ]
    );
    assert_eq!(
        function.line_table,
        [
            128, 0, 240, 2, 8, 5, 18, 217, 11, 17, 217, 24, 28, 137, 70, 140, 72, 161, 37, 165, 39,
            137, 72, 217, 13, 17, 217, 12, 17, 141, 71, 225, 12, 18, 141, 72, 248, 220, 11, 23,
            242, 0, 1, 5, 18, 220, 8, 15, 142, 9, 240, 3, 1, 5, 18, 250
        ]
    );
}

#[test]
fn python_313_folded_loop_false_path_crosses_nested_try_cleanup() {
    // Native 3.13.15 resolves lines before folding the loop's false exit,
    // including the original inner try exit copied onto the authored break.
    let module = compile(
        "def f():\n    try:\n        try:\n            while True:\n                if cancel():\n                    return None\n                if ready():\n                    result = receive()\n                    break\n        except Failure as error:\n            raise Other from error\n    finally:\n        finish()\n",
        PythonVersion::Py313,
    );
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        [
            149, 0, 30, 0, 30, 0, 30, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0,
            40, 0, 0, 0, 0, 0, 0, 0, 97, 12, 0, 0, 30, 0, 91, 3, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0,
            0, 0, 0, 0, 0, 32, 0, 103, 0, 91, 5, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0,
            40, 0, 0, 0, 0, 0, 0, 0, 97, 11, 0, 0, 91, 7, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0,
            0, 0, 0, 110, 0, 79, 2, 77, 55, 0, 0, 91, 3, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0,
            0, 0, 32, 0, 103, 0, 33, 0, 91, 8, 0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 97, 12, 0, 0, 110, 1,
            91, 10, 0, 0, 0, 0, 0, 0, 0, 0, 85, 1, 101, 2, 83, 0, 110, 1, 65, 1, 102, 1, 102, 0,
            61, 3, 31, 0, 102, 1, 33, 0, 91, 3, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0,
            32, 0, 102, 0, 61, 3, 31, 0, 102, 1
        ]
    );
    assert_eq!(
        function.line_table,
        [
            128, 0, 240, 2, 11, 5, 17, 240, 2, 8, 9, 35, 216, 18, 22, 220, 19, 25, 151, 56, 145,
            56, 216, 27, 31, 244, 14, 0, 9, 15, 141, 8, 244, 13, 0, 20, 25, 151, 55, 145, 55, 220,
            29, 36, 155, 89, 144, 70, 216, 20, 25, 241, 11, 0, 19, 23, 244, 18, 0, 9, 15, 141, 8,
            248, 244, 7, 0, 16, 23, 243, 0, 1, 9, 35, 220, 18, 23, 152, 85, 208, 12, 34, 251, 240,
            3, 1, 9, 35, 251, 244, 6, 0, 9, 15, 141, 8, 250
        ]
    );
    assert_eq!(
        function.exception_table,
        [
            131, 16, 65, 6, 0, 159, 25, 65, 6, 0, 184, 1, 65, 31, 0, 185, 2, 65, 6, 0, 193, 6, 10,
            65, 28, 3, 193, 16, 7, 65, 23, 3, 193, 23, 5, 65, 28, 3, 193, 28, 3, 65, 31, 0, 193,
            31, 12, 65, 43, 3
        ]
    );
}

#[test]
fn legacy_high_local_checks_keep_folded_condition_boundaries() {
    // Captured from the pinned CPython 3.12 and 3.13 compilers.
    let parameters = (0..64)
        .map(|index| format!("p{index}"))
        .chain(["condition", "action", "sink"].map(str::to_owned))
        .collect::<Vec<_>>()
        .join(", ");
    for (body, expected312, expected313) in [
        (
            "    value = action() if condition else None\n    assert False, f\"value: {value}\"\n",
            &[
                151, 0, 127, 64, 114, 7, 2, 0, 127, 65, 171, 0, 0, 0, 0, 0, 0, 0, 110, 1, 100, 0,
                125, 67, 74, 0, 100, 1, 127, 67, 155, 0, 157, 2, 171, 0, 0, 0, 0, 0, 0, 0, 130, 1,
            ][..],
            &[
                149, 0, 87, 64, 40, 0, 0, 0, 0, 0, 0, 0, 97, 7, 0, 0, 87, 65, 34, 0, 53, 0, 0, 0,
                0, 0, 0, 0, 79, 1, 83, 0, 110, 67, 23, 0, 83, 1, 87, 67, 14, 0, 51, 2, 53, 0, 0, 0,
                0, 0, 0, 0, 101, 1,
            ][..],
        ),
        (
            "    value = action() if condition else None\n    if \"ready\" and value and \"item\" in value:\n        sink(value)\n",
            &[
                151, 0, 127, 64, 114, 7, 2, 0, 127, 65, 171, 0, 0, 0, 0, 0, 0, 0, 110, 1, 100, 0,
                125, 67, 127, 67, 114, 14, 100, 1, 127, 67, 118, 0, 114, 9, 2, 0, 127, 66, 127, 67,
                171, 1, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0, 121, 0, 121, 0,
            ][..],
            &[
                149, 0, 87, 64, 40, 0, 0, 0, 0, 0, 0, 0, 97, 7, 0, 0, 87, 65, 34, 0, 53, 0, 0, 0,
                0, 0, 0, 0, 79, 1, 83, 0, 110, 67, 87, 67, 40, 0, 0, 0, 0, 0, 0, 0, 97, 16, 0, 0,
                83, 1, 87, 67, 59, 0, 0, 0, 97, 9, 0, 0, 87, 66, 34, 0, 87, 67, 53, 1, 0, 0, 0, 0,
                0, 0, 32, 0, 103, 0, 103, 0, 103, 0,
            ][..],
        ),
        (
            "    value: marker = 1\n    if True:\n        sink(value)\n",
            &[
                151, 0, 100, 1, 125, 67, 9, 0, 2, 0, 127, 66, 127, 67, 171, 1, 0, 0, 0, 0, 0, 0, 1,
                0, 121, 0,
            ][..],
            &[
                149, 0, 83, 1, 110, 67, 30, 0, 87, 66, 34, 0, 87, 67, 53, 1, 0, 0, 0, 0, 0, 0, 32,
                0, 103, 0,
            ][..],
        ),
    ] {
        let source = format!("def f({parameters}):\n{body}");
        for (version, expected) in [
            (PythonVersion::Py312, expected312),
            (PythonVersion::Py313, expected313),
        ] {
            let module = compile(&source, version);
            assert_eq!(
                child(&module, "f").bytecode,
                expected,
                "{version:?}: {body}"
            );
        }
    }
}

#[test]
fn legacy_static_swaps_keep_original_comparison_join() {
    let module = compile(
        "def f(value):\n    result = 0 < value < 10\n    return result\n",
        PythonVersion::Py312,
    );
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        [
            151, 0, 100, 1, 124, 0, 99, 2, 120, 2, 107, 2, 0, 0, 120, 1, 114, 7, 1, 0, 100, 2, 107,
            2, 0, 0, 125, 1, 124, 1, 83, 0, 99, 2, 1, 0, 125, 1, 124, 1, 83, 0
        ]
    );
    assert_eq!(
        function.line_table,
        [
            128, 0, 216, 13, 14, 144, 21, 142, 94, 152, 18, 137, 94, 128, 70, 216, 11, 17, 128, 77,
            241, 3, 0, 14, 28, 128, 70, 216, 11, 17, 128, 77
        ]
    );
    assert!(function.exception_table.is_empty());
    let module = compile(
        "def f(value):\n    result = 0 < value < 10\n    return result\n",
        PythonVersion::Py313,
    );
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        [
            149, 0, 83, 1, 85, 0, 115, 2, 61, 2, 58, 2, 0, 0, 61, 1, 40, 0, 0, 0, 0, 0, 0, 0, 97,
            7, 0, 0, 32, 0, 83, 2, 58, 2, 0, 0, 110, 1, 85, 1, 36, 0, 115, 2, 32, 0, 110, 1, 85, 1,
            36, 0
        ]
    );
    assert_eq!(
        function.line_table,
        [
            128, 0, 216, 13, 14, 144, 21, 143, 94, 139, 94, 152, 18, 137, 94, 128, 70, 216, 11, 17,
            128, 77, 241, 3, 0, 14, 28, 128, 70, 216, 11, 17, 128, 77
        ]
    );
    assert!(function.exception_table.is_empty());
}

#[test]
fn legacy_interleaved_manager_returns_use_target_stack_slots() {
    let module = compile(
        "def f(items, manager, flag):\n    try:\n        for outer in items:\n            with manager:\n                for inner in items:\n                    if flag:\n                        return\n    finally:\n        finish()\n",
        PythonVersion::Py312,
    );
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        [
            151, 0, 9, 0, 124, 0, 68, 0, 93, 44, 0, 0, 125, 3, 124, 1, 53, 0, 1, 0, 124, 0, 68, 0,
            93, 25, 0, 0, 125, 4, 124, 2, 115, 1, 140, 6, 1, 0, 100, 0, 100, 0, 100, 0, 171, 2, 0,
            0, 0, 0, 0, 0, 1, 0, 1, 0, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1,
            0, 121, 0, 4, 0, 9, 0, 100, 0, 100, 0, 100, 0, 171, 2, 0, 0, 0, 0, 0, 0, 1, 0, 140, 46,
            4, 0, 9, 0, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0, 35,
            0, 49, 0, 115, 1, 119, 2, 1, 0, 89, 0, 1, 0, 1, 0, 140, 68, 120, 3, 89, 0, 119, 1, 35,
            0, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 119, 0, 120, 3, 89,
            0, 119, 1
        ]
    );
    assert_eq!(
        function.exception_table,
        [
            130, 7, 65, 11, 0, 137, 8, 63, 5, 146, 1, 63, 5, 147, 9, 65, 11, 0, 167, 1, 63, 5, 169,
            10, 65, 11, 0, 191, 5, 65, 8, 9, 193, 4, 7, 65, 11, 0, 193, 11, 12, 65, 23, 3
        ]
    );
    assert_eq!(
        function.line_table,
        [
            128, 0, 240, 2, 7, 5, 17, 219, 21, 26, 136, 69, 218, 17, 24, 219, 29, 34, 144, 69, 218,
            23, 27, 216, 24, 30, 247, 7, 0, 18, 25, 144, 23, 244, 10, 0, 9, 15, 141, 8, 241, 9, 0,
            30, 35, 247, 3, 0, 18, 25, 144, 23, 241, 3, 0, 22, 27, 244, 12, 0, 9, 15, 141, 8, 247,
            11, 0, 18, 25, 144, 23, 251, 244, 10, 0, 9, 15, 141, 8, 250
        ]
    );
    let module = compile(
        "def f(items, manager, flag):\n    try:\n        for outer in items:\n            with manager:\n                for inner in items:\n                    if flag:\n                        return\n    finally:\n        finish()\n",
        PythonVersion::Py313,
    );
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        [
            149, 0, 30, 0, 85, 0, 19, 0, 72, 51, 0, 0, 110, 3, 85, 1, 2, 0, 32, 0, 85, 0, 19, 0,
            72, 31, 0, 0, 110, 4, 85, 2, 40, 0, 0, 0, 0, 0, 0, 0, 100, 2, 0, 0, 77, 12, 0, 0, 32,
            0, 83, 0, 83, 0, 83, 0, 53, 2, 0, 0, 0, 0, 0, 0, 32, 0, 32, 0, 91, 1, 0, 0, 0, 0, 0, 0,
            0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 103, 0, 11, 0, 32, 0, 83, 0, 83, 0, 83, 0, 53, 2,
            0, 0, 0, 0, 0, 0, 32, 0, 77, 53, 0, 0, 11, 0, 32, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0, 53,
            0, 0, 0, 0, 0, 0, 0, 32, 0, 103, 0, 33, 0, 44, 0, 40, 0, 0, 0, 0, 0, 0, 0, 100, 1, 0,
            0, 102, 2, 32, 0, 31, 0, 32, 0, 32, 0, 77, 81, 0, 0, 61, 3, 31, 0, 102, 1, 33, 0, 91,
            1, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 102, 0, 61, 3, 31, 0, 102,
            1
        ]
    );
    assert_eq!(
        function.exception_table,
        [
            130, 7, 65, 24, 0, 137, 11, 65, 6, 5, 152, 1, 65, 6, 5, 153, 9, 65, 24, 0, 173, 2, 65,
            6, 5, 175, 12, 65, 24, 0, 193, 6, 10, 65, 21, 9, 193, 16, 8, 65, 24, 0, 193, 24, 12,
            65, 36, 3
        ]
    );
    assert_eq!(
        function.line_table,
        [
            128, 0, 240, 2, 7, 5, 17, 219, 21, 26, 136, 69, 218, 17, 24, 219, 29, 34, 144, 69, 223,
            23, 27, 144, 116, 216, 24, 30, 247, 7, 0, 18, 25, 144, 23, 244, 10, 0, 9, 15, 141, 8,
            241, 9, 0, 30, 35, 247, 3, 0, 18, 25, 145, 23, 241, 3, 0, 22, 27, 244, 12, 0, 9, 15,
            141, 8, 247, 11, 0, 18, 25, 150, 23, 251, 244, 10, 0, 9, 15, 141, 8, 250
        ]
    );
}

#[test]
fn legacy_match_default_pass_keeps_original_return_array() {
    // Captured from the pinned CPython 3.12 and 3.13 compilers.
    for (version, source, expected_code, expected_lines) in [
        (
            PythonVersion::Py312,
            "def f(x, action):\n    match x:\n        case 1 | 2:\n            action()\n        case _:\n            pass\n",
            &[
                151, 0, 124, 0, 120, 1, 100, 1, 107, 40, 0, 0, 114, 1, 110, 8, 120, 1, 100, 2, 107,
                40, 0, 0, 114, 1, 110, 2, 1, 0, 110, 9, 1, 0, 2, 0, 124, 1, 171, 0, 0, 0, 0, 0, 0,
                0, 1, 0, 121, 0, 9, 0, 121, 0,
            ][..],
            &[
                128, 0, 216, 10, 11, 221, 13, 14, 149, 17, 138, 85, 217, 12, 18, 141, 72, 216, 13,
                14, 216, 12, 16,
            ][..],
        ),
        (
            PythonVersion::Py313,
            "def f(x, action):\n    match x:\n        case 1 | 2:\n            pass\n        case _:\n            pass\n",
            &[
                149, 0, 85, 0, 61, 1, 83, 1, 58, 88, 0, 0, 97, 1, 0, 0, 79, 9, 61, 1, 83, 2, 58,
                88, 0, 0, 97, 1, 0, 0, 79, 2, 32, 0, 79, 2, 32, 0, 103, 0, 30, 0, 103, 0,
            ][..],
            &[
                128, 0, 216, 10, 11, 222, 13, 14, 150, 17, 138, 85, 216, 12, 16, 216, 13, 14, 216,
                12, 16,
            ][..],
        ),
        (
            PythonVersion::Py313,
            "def f(x, action):\n    match x:\n        case 1 | 2:\n            action()\n        case _:\n            pass\n",
            &[
                149, 0, 85, 0, 61, 1, 83, 1, 58, 88, 0, 0, 97, 1, 0, 0, 79, 9, 61, 1, 83, 2, 58,
                88, 0, 0, 97, 1, 0, 0, 79, 2, 32, 0, 79, 9, 32, 0, 85, 1, 34, 0, 53, 0, 0, 0, 0, 0,
                0, 0, 32, 0, 103, 0, 30, 0, 103, 0,
            ][..],
            &[
                128, 0, 216, 10, 11, 222, 13, 14, 150, 17, 138, 85, 217, 12, 18, 141, 72, 216, 13,
                14, 216, 12, 16,
            ][..],
        ),
    ] {
        let module = compile(source, version);
        let function = child(&module, "f");
        assert_eq!(function.bytecode, expected_code, "{version:?}: {source}");
        assert_eq!(function.line_table, expected_lines, "{version:?}: {source}");
    }
}

#[test]
fn python_313_nested_manager_pop_uses_its_complete_original_array() {
    // CPython 3.13.15 resolves the outer POP_BLOCK while the original
    // inner-loop false exit still joins the manager suppression path.
    let module = compile(
        "def f(outer, inner):\n    with outer:\n        with inner:\n            while True:\n                work()\n",
        PythonVersion::Py313,
    );
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        [
            149, 0, 85, 0, 2, 0, 32, 0, 85, 1, 2, 0, 32, 0, 30, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0,
            53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 77, 12, 0, 0, 33, 0, 44, 0, 40, 0, 0, 0, 0, 0, 0, 0,
            100, 1, 0, 0, 102, 2, 32, 0, 31, 0, 32, 0, 32, 0, 79, 3, 61, 3, 31, 0, 102, 1, 83, 0,
            83, 0, 83, 0, 53, 2, 0, 0, 0, 0, 0, 0, 32, 0, 103, 0, 33, 0, 44, 0, 40, 0, 0, 0, 0, 0,
            0, 0, 100, 1, 0, 0, 102, 2, 32, 0, 31, 0, 32, 0, 32, 0, 103, 0, 61, 3, 31, 0, 102, 1
        ]
    );
    assert_eq!(
        function.line_table,
        [
            128, 0, 218, 9, 14, 218, 13, 18, 216, 18, 22, 220, 16, 20, 148, 6, 241, 3, 0, 19, 23,
            247, 3, 0, 14, 19, 141, 85, 250, 247, 3, 0, 10, 15, 143, 21, 142, 21, 250
        ]
    );
    assert_eq!(
        function.exception_table,
        [
            131, 3, 46, 3, 134, 14, 20, 5, 148, 10, 34, 9, 158, 7, 46, 3, 174, 10, 60, 7
        ]
    );
}

#[test]
fn legacy_continue_finally_preserves_pending_outer_try_exit() {
    // Captured from the pinned CPython 3.12 and 3.13 compilers.
    let source = "def f(events, action, flag):\n    for event in events:\n        try:\n            try:\n                action()\n            except ValueError:\n                continue\n            finally:\n                if flag:\n                    action()\n        except BaseException:\n            pass\n";
    for (version, expected_code, expected_lines, expected_exceptions) in [
        (
            PythonVersion::Py312,
            &[
                151, 0, 124, 0, 68, 0, 93, 22, 0, 0, 125, 3, 9, 0, 9, 0, 2, 0, 124, 1, 171, 0, 0,
                0, 0, 0, 0, 0, 1, 0, 9, 0, 124, 2, 114, 8, 2, 0, 124, 1, 171, 0, 0, 0, 0, 0, 0, 0,
                1, 0, 9, 0, 140, 24, 4, 0, 121, 0, 35, 0, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0,
                114, 12, 1, 0, 89, 0, 124, 2, 114, 7, 2, 0, 124, 1, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0,
                140, 46, 119, 0, 120, 3, 89, 0, 119, 1, 35, 0, 124, 2, 114, 8, 2, 0, 124, 1, 171,
                0, 0, 0, 0, 0, 0, 0, 1, 0, 119, 0, 119, 0, 120, 3, 89, 0, 119, 1, 35, 0, 116, 2, 0,
                0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 3, 1, 0, 89, 0, 140, 76, 119, 0, 120, 3, 89, 0,
                119, 1,
            ][..],
            &[
                128, 0, 219, 17, 23, 136, 5, 240, 2, 9, 9, 17, 240, 2, 6, 13, 29, 217, 16, 22, 149,
                8, 241, 8, 0, 20, 24, 217, 20, 26, 149, 72, 248, 241, 17, 0, 18, 24, 248, 244, 8,
                0, 20, 30, 242, 0, 1, 13, 25, 216, 16, 24, 225, 19, 23, 217, 20, 26, 148, 72, 248,
                240, 9, 1, 13, 25, 251, 241, 6, 0, 20, 24, 217, 20, 26, 149, 72, 240, 3, 0, 20, 24,
                251, 228, 15, 28, 242, 0, 1, 9, 17, 217, 12, 16, 240, 3, 1, 9, 17, 250,
            ][..],
            &[
                136, 7, 29, 2, 144, 10, 65, 4, 2, 157, 9, 50, 5, 166, 1, 53, 2, 167, 9, 65, 4, 2,
                177, 1, 50, 5, 178, 3, 53, 2, 181, 12, 65, 1, 5, 193, 1, 3, 65, 4, 2, 193, 4, 9,
                65, 16, 5, 193, 15, 1, 65, 16, 5,
            ][..],
        ),
        (
            PythonVersion::Py313,
            &[
                149, 0, 85, 0, 19, 0, 72, 29, 0, 0, 110, 3, 30, 0, 30, 0, 85, 1, 34, 0, 53, 0, 0,
                0, 0, 0, 0, 0, 32, 0, 30, 0, 85, 2, 40, 0, 0, 0, 0, 0, 0, 0, 97, 9, 0, 0, 85, 1,
                34, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 77, 29, 0, 0, 77, 31, 0, 0, 11, 0, 32, 0,
                103, 0, 33, 0, 91, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 97, 20, 0, 0, 32, 0, 31, 0, 85,
                2, 40, 0, 0, 0, 0, 0, 0, 0, 97, 9, 0, 0, 85, 1, 34, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32,
                0, 77, 61, 0, 0, 77, 63, 0, 0, 102, 0, 61, 3, 31, 0, 102, 1, 33, 0, 85, 2, 40, 0,
                0, 0, 0, 0, 0, 0, 97, 8, 0, 0, 85, 1, 34, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 102,
                0, 102, 0, 61, 3, 31, 0, 102, 1, 33, 0, 91, 2, 0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 97, 4,
                0, 0, 32, 0, 31, 0, 77, 100, 0, 0, 102, 0, 61, 3, 31, 0, 102, 1,
            ][..],
            &[
                128, 0, 219, 17, 23, 136, 5, 240, 2, 9, 9, 17, 240, 2, 6, 13, 29, 217, 16, 22, 149,
                8, 246, 8, 0, 20, 24, 217, 20, 26, 150, 72, 241, 3, 0, 20, 24, 242, 15, 0, 18, 24,
                248, 244, 8, 0, 20, 30, 243, 0, 1, 13, 25, 216, 16, 24, 230, 19, 23, 217, 20, 26,
                150, 72, 241, 3, 0, 20, 24, 240, 7, 1, 13, 25, 251, 246, 6, 0, 20, 24, 217, 20, 26,
                149, 72, 240, 3, 0, 20, 24, 251, 228, 15, 28, 243, 0, 1, 9, 17, 218, 12, 16, 240,
                3, 1, 9, 17, 250,
            ][..],
            &[
                136, 7, 37, 2, 144, 14, 65, 26, 2, 165, 10, 65, 3, 5, 175, 1, 65, 6, 2, 176, 14,
                65, 26, 2, 193, 2, 1, 65, 3, 5, 193, 3, 3, 65, 6, 2, 193, 6, 17, 65, 23, 5, 193,
                23, 3, 65, 26, 2, 193, 26, 10, 65, 40, 5, 193, 39, 1, 65, 40, 5,
            ][..],
        ),
    ] {
        let module = compile(source, version);
        let function = child(&module, "f");
        assert_eq!(function.bytecode, expected_code, "{version:?}");
        assert_eq!(function.line_table, expected_lines, "{version:?}");
        assert_eq!(function.exception_table, expected_exceptions, "{version:?}");
    }
}

#[test]
fn python_313_handler_backedges_duplicate_complete_cleanup_arrays() {
    // Exact code and tables from the pinned native target for generic public reducers.
    for (source, bytecode, line_table, exception_table) in [
        (
            "def f(items, flag):\n    for item in items:\n        try:\n            work(item)\n        except Error as outer:\n            try:\n                recover(item)\n            except OtherError:\n                if flag:\n                    notify(item)\n    finish()\n",
            &[
                149, 0, 85, 0, 19, 0, 72, 15, 0, 0, 110, 2, 30, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0,
                85, 2, 53, 1, 0, 0, 0, 0, 0, 0, 32, 0, 77, 17, 0, 0, 11, 0, 32, 0, 91, 11, 0, 0, 0,
                0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 103, 0, 33, 0, 91, 2, 0, 0, 0, 0, 0,
                0, 0, 0, 7, 0, 97, 69, 0, 0, 110, 3, 30, 0, 91, 5, 0, 0, 0, 0, 0, 0, 0, 0, 85, 2,
                53, 1, 0, 0, 0, 0, 0, 0, 32, 0, 31, 0, 83, 0, 110, 3, 65, 3, 77, 58, 0, 0, 33, 0,
                91, 6, 0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 97, 33, 0, 0, 32, 0, 85, 1, 40, 0, 0, 0, 0, 0,
                0, 0, 97, 18, 0, 0, 91, 9, 0, 0, 0, 0, 0, 0, 0, 0, 85, 2, 53, 1, 0, 0, 0, 0, 0, 0,
                32, 0, 31, 0, 31, 0, 83, 0, 110, 3, 65, 3, 77, 93, 0, 0, 31, 0, 31, 0, 83, 0, 110,
                3, 65, 3, 77, 100, 0, 0, 102, 0, 61, 3, 31, 0, 102, 1, 83, 0, 110, 3, 65, 3, 102,
                1, 102, 0, 61, 3, 31, 0, 102, 1,
            ][..],
            &[
                128, 0, 219, 16, 21, 136, 4, 240, 2, 7, 9, 33, 220, 12, 16, 144, 20, 142, 74, 241,
                5, 0, 17, 22, 244, 18, 0, 5, 11, 133, 72, 248, 244, 13, 0, 16, 21, 243, 0, 5, 9,
                33, 240, 2, 4, 13, 33, 220, 16, 23, 152, 4, 151, 13, 146, 13, 248, 220, 19, 29,
                243, 0, 2, 13, 33, 222, 19, 23, 220, 20, 26, 152, 52, 151, 76, 147, 76, 246, 3, 0,
                20, 24, 240, 3, 2, 13, 33, 254, 240, 7, 5, 9, 33, 250,
            ][..],
            &[
                135, 11, 33, 2, 161, 10, 65, 48, 5, 172, 11, 61, 4, 189, 28, 65, 40, 7, 193, 25, 1,
                65, 43, 5, 193, 32, 1, 65, 43, 5, 193, 39, 1, 65, 40, 7, 193, 40, 3, 65, 43, 5,
                193, 43, 5, 65, 48, 5,
            ][..],
        ),
        (
            "def f(items, flag):\n    for item in items:\n        try:\n            work(item)\n        except Error as outer:\n            try:\n                recover(item)\n            except OtherError as inner:\n                if flag:\n                    notify(item)\n    finish()\n",
            &[
                149, 0, 85, 0, 19, 0, 72, 15, 0, 0, 110, 2, 30, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0,
                85, 2, 53, 1, 0, 0, 0, 0, 0, 0, 32, 0, 77, 17, 0, 0, 11, 0, 32, 0, 91, 11, 0, 0, 0,
                0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 103, 0, 33, 0, 91, 2, 0, 0, 0, 0, 0,
                0, 0, 0, 7, 0, 97, 79, 0, 0, 110, 3, 30, 0, 91, 5, 0, 0, 0, 0, 0, 0, 0, 0, 85, 2,
                53, 1, 0, 0, 0, 0, 0, 0, 32, 0, 31, 0, 83, 0, 110, 3, 65, 3, 77, 58, 0, 0, 33, 0,
                91, 6, 0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 97, 43, 0, 0, 110, 4, 85, 1, 40, 0, 0, 0, 0,
                0, 0, 0, 97, 21, 0, 0, 91, 9, 0, 0, 0, 0, 0, 0, 0, 0, 85, 2, 53, 1, 0, 0, 0, 0, 0,
                0, 32, 0, 31, 0, 83, 0, 110, 4, 65, 4, 31, 0, 83, 0, 110, 3, 65, 3, 77, 96, 0, 0,
                31, 0, 83, 0, 110, 4, 65, 4, 31, 0, 83, 0, 110, 3, 65, 3, 77, 106, 0, 0, 83, 0,
                110, 4, 65, 4, 102, 1, 102, 0, 61, 3, 31, 0, 102, 1, 83, 0, 110, 3, 65, 3, 102, 1,
                102, 0, 61, 3, 31, 0, 102, 1,
            ][..],
            &[
                128, 0, 219, 16, 21, 136, 4, 240, 2, 7, 9, 33, 220, 12, 16, 144, 20, 142, 74, 241,
                5, 0, 17, 22, 244, 18, 0, 5, 11, 133, 72, 248, 244, 13, 0, 16, 21, 243, 0, 5, 9,
                33, 240, 2, 4, 13, 33, 220, 16, 23, 152, 4, 151, 13, 146, 13, 248, 220, 19, 29,
                243, 0, 2, 13, 33, 222, 19, 23, 220, 20, 26, 152, 52, 151, 76, 150, 76, 247, 3, 0,
                20, 24, 145, 52, 251, 240, 3, 2, 13, 33, 254, 240, 7, 5, 9, 33, 250,
            ][..],
            &[
                135, 11, 33, 2, 161, 10, 65, 58, 5, 172, 11, 61, 4, 189, 10, 65, 50, 7, 193, 7, 18,
                65, 45, 7, 193, 25, 4, 65, 53, 5, 193, 35, 4, 65, 53, 5, 193, 45, 5, 65, 50, 7,
                193, 50, 3, 65, 53, 5, 193, 53, 5, 65, 58, 5,
            ][..],
        ),
        (
            "def f(items, flag):\n    try:\n        prepare()\n    except OuterError:\n        for item in items:\n            try:\n                work(item)\n            except Error:\n                if flag:\n                    notify(item)\n    finish()\n",
            &[
                149, 0, 30, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 91,
                11, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 103, 0, 33, 0, 91, 2,
                0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 97, 62, 0, 0, 32, 0, 85, 0, 19, 0, 72, 53, 0, 0, 110,
                2, 30, 0, 91, 5, 0, 0, 0, 0, 0, 0, 0, 0, 85, 2, 53, 1, 0, 0, 0, 0, 0, 0, 32, 0, 77,
                17, 0, 0, 33, 0, 91, 6, 0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 97, 25, 0, 0, 32, 0, 85, 1,
                40, 0, 0, 0, 0, 0, 0, 0, 97, 14, 0, 0, 91, 9, 0, 0, 0, 0, 0, 0, 0, 0, 85, 2, 53, 1,
                0, 0, 0, 0, 0, 0, 32, 0, 31, 0, 77, 48, 0, 0, 31, 0, 77, 51, 0, 0, 102, 0, 61, 3,
                31, 0, 102, 1, 11, 0, 32, 0, 31, 0, 78, 82, 102, 0, 61, 3, 31, 0, 102, 1,
            ][..],
            &[
                128, 0, 240, 2, 8, 5, 33, 220, 8, 15, 140, 9, 244, 16, 0, 5, 11, 133, 72, 248, 244,
                15, 0, 12, 22, 243, 0, 6, 5, 33, 219, 20, 25, 136, 68, 240, 2, 4, 13, 33, 220, 16,
                20, 144, 84, 150, 10, 248, 220, 19, 24, 243, 0, 2, 13, 33, 222, 19, 23, 220, 20,
                26, 152, 52, 151, 76, 242, 3, 0, 20, 24, 240, 3, 2, 13, 33, 250, 243, 7, 0, 21, 26,
                240, 3, 6, 5, 33, 250,
            ][..],
            &[
                130, 10, 23, 0, 151, 15, 65, 31, 3, 167, 11, 52, 4, 178, 2, 65, 31, 3, 180, 28, 65,
                23, 7, 193, 16, 6, 65, 31, 3, 193, 22, 1, 65, 23, 7, 193, 23, 5, 65, 31, 3, 193,
                30, 1, 65, 31, 3,
            ][..],
        ),
    ] {
        let module = compile(source, PythonVersion::Py313);
        let code = child(&module, "f");
        assert_eq!(code.bytecode, bytecode, "{source}");
        assert_eq!(code.line_table, line_table, "{source}");
        assert_eq!(code.exception_table, exception_table, "{source}");
    }
}

#[test]
fn python312_tested_if_arm_copies_small_tails_on_second_sweep() {
    // Captured from the pinned CPython 3.12 compiler.
    for (source, expected_code, expected_lines) in [
        (
            "def f(obj, action, outer, cond, left, right):\n    if outer:\n        obj.value = left if cond else right\n    else:\n        action()\n",
            &[
                151, 0, 124, 2, 114, 18, 124, 3, 114, 8, 124, 4, 124, 0, 95, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 121, 0, 124, 5, 124, 0, 95, 0, 0, 0, 0, 0, 0, 0, 0, 0, 121, 0, 2, 0, 124, 1,
                171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0,
            ][..],
            &[
                128, 0, 217, 7, 12, 217, 28, 32, 144, 68, 136, 3, 141, 9, 160, 101, 136, 3, 141, 9,
                225, 8, 14, 141, 8,
            ][..],
        ),
        (
            "def f(obj, action, outer, cond, left, right):\n    if outer:\n        action(left if cond else right)\n    else:\n        action()\n",
            &[
                151, 0, 124, 2, 114, 18, 2, 0, 124, 1, 124, 3, 114, 7, 124, 4, 171, 1, 0, 0, 0, 0,
                0, 0, 1, 0, 121, 0, 124, 5, 171, 1, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0, 2, 0, 124, 1,
                171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0,
            ][..],
            &[
                128, 0, 217, 7, 12, 217, 8, 14, 145, 116, 136, 116, 213, 8, 39, 160, 21, 213, 8,
                39, 225, 8, 14, 141, 8,
            ][..],
        ),
        (
            "def f(obj, action, outer, cond, left, right):\n    if outer:\n        action(value=left if cond else right)\n    else:\n        action()\n",
            &[
                151, 0, 124, 2, 114, 20, 2, 0, 124, 1, 124, 3, 114, 8, 124, 4, 172, 1, 171, 1, 0,
                0, 0, 0, 0, 0, 1, 0, 121, 0, 124, 5, 172, 1, 171, 1, 0, 0, 0, 0, 0, 0, 1, 0, 121,
                0, 2, 0, 124, 1, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0,
            ][..],
            &[
                128, 0, 217, 7, 12, 217, 8, 14, 153, 84, 144, 84, 214, 8, 45, 160, 117, 214, 8, 45,
                225, 8, 14, 141, 8,
            ][..],
        ),
        (
            "def f(obj, action, outer, cond, left, right):\n    if outer:\n        obj.value = (left if cond else right)()\n    else:\n        action()\n",
            &[
                151, 0, 124, 2, 114, 27, 2, 0, 124, 3, 114, 12, 124, 4, 171, 0, 0, 0, 0, 0, 0, 0,
                124, 0, 95, 0, 0, 0, 0, 0, 0, 0, 0, 0, 121, 0, 124, 5, 171, 0, 0, 0, 0, 0, 0, 0,
                124, 0, 95, 0, 0, 0, 0, 0, 0, 0, 0, 0, 121, 0, 2, 0, 124, 1, 171, 0, 0, 0, 0, 0, 0,
                0, 1, 0, 121, 0,
            ][..],
            &[
                128, 0, 217, 7, 12, 216, 21, 44, 153, 84, 144, 84, 211, 20, 47, 136, 3, 141, 9,
                160, 117, 211, 20, 47, 136, 3, 141, 9, 225, 8, 14, 141, 8,
            ][..],
        ),
    ] {
        let module = compile(source, PythonVersion::Py312);
        let function = child(&module, "f");
        assert_eq!(function.bytecode, expected_code, "{source}");
        assert_eq!(function.line_table, expected_lines, "{source}");
    }
}

#[test]
fn python_312_match_tail_condition_does_not_locate_a_copied_exit_nop() {
    // Native 3.12.14 copies the common return into unlocated case-success
    // jumps, then removes their NOPs before propagating condition locations.
    let module = compile(
        "def f(value, flag):\n    match value:\n        case 1:\n            if flag:\n                raise ValueError\n        case 2:\n            if not flag:\n                raise TypeError\n    return value\n",
        PythonVersion::Py312,
    );
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        [
            151, 0, 124, 0, 120, 1, 100, 1, 107, 40, 0, 0, 114, 11, 1, 0, 124, 1, 114, 6, 116, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 130, 1, 124, 0, 83, 0, 100, 2, 107, 40, 0, 0, 114, 10, 124, 1,
            115, 6, 116, 2, 0, 0, 0, 0, 0, 0, 0, 0, 130, 1, 124, 0, 83, 0, 124, 0, 83, 0
        ]
    );
    assert_eq!(
        function.line_table,
        [
            128, 0, 216, 10, 15, 221, 13, 14, 217, 15, 19, 220, 22, 32, 208, 16, 32, 240, 8, 0, 12,
            17, 128, 76, 243, 7, 0, 14, 15, 217, 19, 23, 220, 22, 31, 144, 15, 216, 11, 16, 128,
            76, 136, 53, 128, 76
        ]
    );
    assert!(function.exception_table.is_empty());
}

#[test]
fn python_312_terminal_chained_comparison_has_no_located_success_nop() {
    // Native 3.12.14 removes the unlocated success jump after copying the return.
    let module = compile(
        "def f(value, lower, upper):\n    if lower <= value < upper:\n        return value\n    else:\n        return None\n",
        PythonVersion::Py312,
    );
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        [
            151, 0, 124, 1, 124, 0, 99, 2, 120, 2, 107, 26, 0, 0, 114, 6, 124, 2, 107, 2, 0, 0,
            114, 4, 124, 0, 83, 0, 1, 0, 121, 0, 121, 0
        ]
    );
    assert_eq!(
        function.line_table,
        [
            128, 0, 216, 7, 12, 144, 5, 212, 7, 29, 152, 5, 210, 7, 29, 216, 15, 20, 136, 12, 240,
            3, 0, 8, 30, 241, 6, 0, 16, 20
        ]
    );
    assert!(function.exception_table.is_empty());
}

#[test]
fn python313_nested_named_handler_join_keeps_its_original_unlocated_exit() {
    // Captured from the pinned CPython 3.13.15 compiler.
    let module = compile(
        "def run(action, first, second):\n    try:\n        action()\n    except Exception as error:\n        if first:\n            action()\n        else:\n            if second:\n                action()\n            else:\n                action()\n",
        PythonVersion::Py313,
    );
    let function = child(&module, "run");
    assert_eq!(
        function.bytecode,
        [
            149, 0, 30, 0, 85, 0, 34, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 103, 0, 33, 0, 91, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 7, 0, 97, 55, 0, 0, 110, 3, 85, 1, 40, 0, 0, 0, 0, 0, 0, 0, 97,
            12, 0, 0, 85, 0, 34, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 31, 0, 83, 0, 110, 3, 65, 3,
            103, 0, 85, 2, 40, 0, 0, 0, 0, 0, 0, 0, 97, 12, 0, 0, 85, 0, 34, 0, 53, 0, 0, 0, 0, 0,
            0, 0, 32, 0, 31, 0, 83, 0, 110, 3, 65, 3, 103, 0, 85, 0, 34, 0, 53, 0, 0, 0, 0, 0, 0,
            0, 32, 0, 31, 0, 83, 0, 110, 3, 65, 3, 103, 0, 83, 0, 110, 3, 65, 3, 102, 1, 102, 0,
            61, 3, 31, 0, 102, 1
        ]
    );
    assert_eq!(
        function.line_table,
        [
            128, 0, 240, 2, 9, 5, 25, 217, 8, 14, 141, 8, 248, 220, 11, 20, 243, 0, 7, 5, 25, 222,
            11, 16, 217, 12, 18, 143, 72, 137, 72, 230, 15, 21, 217, 16, 22, 151, 8, 145, 8, 225,
            16, 22, 151, 8, 145, 8, 251, 240, 15, 7, 5, 25, 250
        ]
    );
    assert_eq!(
        function.exception_table,
        [
            130, 7, 10, 0, 138, 10, 65, 11, 3, 148, 14, 65, 6, 3, 167, 14, 65, 6, 3, 186, 7, 65, 6,
            3, 193, 6, 5, 65, 11, 3
        ]
    );
}

#[test]
fn python_312_nested_terminal_if_retains_shared_footer_order() {
    let module = compile(
        "def f(a, b, c):\n    if a:\n        if b:\n            assert c\n        else:\n            other()\n    else:\n        work()\n",
        PythonVersion::Py312,
    );
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        [
            151, 0, 124, 0, 114, 18, 124, 1, 114, 4, 124, 2, 115, 13, 74, 0, 130, 1, 116, 1, 0, 0,
            0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0, 121, 0, 116, 3, 0, 0, 0, 0,
            0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0
        ]
    );
    assert_eq!(
        function.line_table,
        [
            128, 0, 217, 7, 8, 217, 11, 12, 217, 19, 20, 136, 72, 144, 49, 228, 12, 17, 141, 71,
            240, 5, 0, 20, 21, 244, 8, 0, 9, 13, 141, 6
        ]
    );
    assert!(function.exception_table.is_empty());
}

#[test]
fn python312_terminal_async_manager_keeps_original_footer_jump_seed() {
    // Captured from CPython 3.12.14. The initial native labels are small, but
    // this scope exceeds the conservative uniform-seed bound. Its cold await
    // bridge must converge to 255 without an unnecessary EXTENDED_ARG.
    let mut source = String::from("async def f(cm, flag, work):\n");
    source.push_str(&"    if flag:\n        value = 0\n".repeat(60));
    source.push_str("    await work()\n");
    source.push_str(&"    value = 0\n".repeat(111));
    source.push_str("    async with cm:\n        pass\n");
    let module = compile(&source, PythonVersion::Py312);
    let function = child(&module, "f");
    let mut bytecode = vec![75, 0, 1, 0, 151, 0];
    bytecode.extend([124, 1, 114, 2, 100, 1, 125, 3].repeat(60));
    bytecode.extend([
        2, 0, 124, 2, 171, 0, 0, 0, 0, 0, 0, 0, 131, 0, 100, 0, 123, 3, 0, 0, 150, 2, 151, 3, 134,
        5, 5, 0, 1, 0,
    ]);
    bytecode.extend([100, 1, 125, 3].repeat(111));
    bytecode.extend([
        124, 0, 52, 0, 131, 1, 100, 0, 123, 3, 0, 0, 150, 2, 151, 3, 134, 5, 5, 0, 1, 0, 9, 0, 100,
        0, 100, 0, 100, 0, 171, 2, 0, 0, 0, 0, 0, 0, 131, 2, 100, 0, 123, 3, 0, 0, 150, 2, 151, 3,
        134, 5, 5, 0, 1, 0, 121, 0, 55, 0, 140, 255, 55, 0, 140, 24, 55, 0, 140, 9, 35, 0, 49, 0,
        131, 2, 100, 0, 123, 4, 0, 0, 150, 3, 151, 3, 134, 5, 55, 0, 5, 0, 115, 1, 119, 2, 1, 0,
        89, 0, 1, 0, 1, 0, 121, 0, 120, 3, 89, 0, 119, 1, 173, 3, 119, 1,
    ]);
    assert_eq!(function.bytecode, bytecode);
    assert_eq!(
        function.exception_table,
        [
            130, 67, 59, 72, 24, 1, 195, 61, 1, 71, 61, 4, 195, 62, 67, 40, 72, 24, 1, 199, 38, 1,
            71, 63, 6, 199, 39, 3, 72, 24, 1, 199, 42, 2, 72, 3, 3, 199, 44, 11, 72, 24, 1, 199,
            55, 1, 72, 1, 4, 199, 56, 6, 72, 24, 1, 199, 63, 1, 72, 24, 1, 200, 1, 1, 72, 24, 1,
            200, 3, 6, 72, 21, 7, 200, 9, 1, 72, 12, 12, 200, 10, 7, 72, 21, 7, 200, 17, 7, 72, 24,
            1,
        ]
    );
}

#[test]
fn python312_finally_bound_preserves_backward_jump_at_one_byte_boundary() {
    // Captured from CPython 3.12.14. Finally source emission formerly declined
    // the uniform native seed certificate. Its small label budget must let the
    // loop converge to 255 instead of preserving a spurious EXTENDED_ARG at 256.
    let mut source =
        String::from("def f(flag):\n    try:\n        work()\n    finally:\n        cleanup()\n");
    source.push_str(&"    padding = 0\n".repeat(150));
    source.push_str("    while flag:\n");
    source.push_str(&"        padding = 0\n".repeat(126));
    let module = compile(&source, PythonVersion::Py312);
    let function = child(&module, "f");
    let mut bytecode = vec![
        151, 0, 9, 0, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 116, 3, 0, 0,
        0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0,
    ];
    bytecode.extend([100, 1, 125, 1].repeat(150));
    bytecode.extend([124, 0, 144, 1, 114, 0]);
    bytecode.extend([100, 1, 125, 1].repeat(126));
    bytecode.extend([
        124, 0, 114, 1, 140, 255, 121, 0, 121, 0, 35, 0, 116, 3, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0,
        0, 0, 0, 0, 0, 1, 0, 119, 0, 120, 3, 89, 0, 119, 1,
    ]);
    assert_eq!(function.bytecode, bytecode);
    assert_eq!(
        function.exception_table,
        [130, 10, 73, 6, 0, 201, 6, 12, 73, 18, 3,]
    );
}

#[test]
fn python_312_break_keeps_inner_try_pop_inside_an_enclosing_handler() {
    // Native 3.12.14 unwinds the inner TRY_EXCEPT before the loop break,
    // retaining the surrounding handler while POP_BLOCK itself stays ownerless.
    let module = compile(
        "async def f(flag):\n    try:\n        await work()\n    except Failure as error:\n        if flag:\n            while True:\n                try:\n                    await cleanup()\n                    break\n                except Cancelled:\n                    if ready():\n                        break\n        else:\n            await cleanup()\n        raise\n",
        PythonVersion::Py312,
    );
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        [
            75, 0, 1, 0, 151, 0, 9, 0, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0,
            131, 0, 100, 0, 123, 3, 0, 0, 150, 3, 151, 3, 134, 5, 5, 0, 1, 0, 121, 0, 55, 0, 140,
            5, 35, 0, 116, 2, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 78, 125, 1, 124, 0, 114, 51, 9,
            0, 9, 0, 116, 5, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 131, 0, 100, 0, 123,
            4, 0, 0, 150, 5, 151, 3, 134, 5, 55, 0, 5, 0, 1, 0, 9, 0, 130, 0, 35, 0, 116, 6, 0, 0,
            0, 0, 0, 0, 0, 0, 36, 0, 114, 15, 1, 0, 116, 9, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0,
            0, 0, 0, 0, 114, 2, 89, 0, 130, 0, 89, 0, 110, 4, 119, 0, 120, 3, 89, 0, 119, 1, 140,
            50, 116, 5, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 131, 0, 100, 0, 123, 4,
            0, 0, 150, 4, 151, 3, 134, 5, 55, 0, 5, 0, 1, 0, 130, 0, 100, 0, 125, 1, 126, 1, 119,
            1, 119, 0, 120, 3, 89, 0, 119, 1, 173, 3, 119, 1
        ]
    );
    assert_eq!(
        function.line_table,
        [
            232, 0, 248, 128, 0, 240, 2, 13, 5, 14, 220, 14, 18, 139, 102, 143, 12, 138, 12, 249,
            220, 11, 18, 242, 0, 11, 5, 14, 217, 11, 15, 216, 18, 22, 240, 2, 5, 17, 30, 220, 26,
            33, 155, 41, 151, 79, 145, 79, 216, 20, 25, 240, 12, 0, 9, 14, 248, 244, 11, 0, 24, 33,
            242, 0, 2, 17, 30, 220, 23, 28, 148, 119, 216, 24, 29, 240, 6, 0, 9, 14, 241, 9, 0, 24,
            31, 240, 3, 2, 17, 30, 250, 240, 9, 0, 19, 23, 244, 16, 0, 19, 26, 147, 41, 143, 79,
            137, 79, 216, 8, 13, 251, 240, 23, 11, 5, 14, 252
        ]
    );
    assert_eq!(
        function.exception_table,
        [
            130, 1, 65, 51, 1, 132, 13, 25, 0, 145, 1, 23, 4, 146, 4, 25, 0, 150, 1, 65, 51, 1,
            151, 1, 25, 0, 153, 9, 65, 48, 3, 162, 3, 65, 43, 3, 166, 13, 59, 2, 179, 1, 54, 6,
            180, 5, 59, 2, 186, 1, 65, 43, 3, 187, 19, 65, 19, 5, 193, 14, 4, 65, 43, 3, 193, 18,
            1, 65, 19, 5, 193, 19, 17, 65, 43, 3, 193, 36, 1, 65, 39, 6, 193, 37, 6, 65, 43, 3,
            193, 43, 5, 65, 48, 3, 193, 48, 3, 65, 51, 1
        ]
    );
}

#[test]
fn python312_compound_try_exits_keep_the_shared_footer() {
    // Captured from the pinned CPython 3.12 compiler.
    for (source, expected_code, expected_lines, expected_exceptions) in [
        (
            "def f(flag, guard, action):\n    try:\n        action(0)\n    finally:\n        try:\n            if flag:\n                action(1)\n            else:\n                action(2)\n        except ValueError:\n            action(3)\n",
            &[
                151, 0, 9, 0, 2, 0, 124, 2, 100, 1, 171, 1, 0, 0, 0, 0, 0, 0, 1, 0, 9, 0, 124, 0,
                114, 9, 2, 0, 124, 2, 100, 2, 171, 1, 0, 0, 0, 0, 0, 0, 1, 0, 110, 9, 2, 0, 124, 2,
                100, 3, 171, 1, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0, 121, 0, 35, 0, 116, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 36, 0, 114, 11, 1, 0, 2, 0, 124, 2, 100, 4, 171, 1, 0, 0, 0, 0, 0, 0,
                1, 0, 89, 0, 121, 0, 119, 0, 120, 3, 89, 0, 119, 1, 35, 0, 9, 0, 124, 0, 114, 9, 2,
                0, 124, 2, 100, 2, 171, 1, 0, 0, 0, 0, 0, 0, 1, 0, 119, 0, 2, 0, 124, 2, 100, 3,
                171, 1, 0, 0, 0, 0, 0, 0, 1, 0, 119, 0, 35, 0, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 36,
                0, 114, 11, 1, 0, 2, 0, 124, 2, 100, 4, 171, 1, 0, 0, 0, 0, 0, 0, 1, 0, 89, 0, 119,
                0, 119, 0, 120, 3, 89, 0, 119, 1, 120, 3, 89, 0, 119, 1,
            ][..],
            &[
                128, 0, 240, 2, 9, 5, 22, 217, 8, 14, 136, 113, 140, 9, 240, 4, 6, 9, 22, 217, 15,
                19, 217, 16, 22, 144, 113, 149, 9, 225, 16, 22, 144, 113, 149, 9, 240, 5, 0, 17,
                26, 248, 244, 6, 0, 16, 26, 242, 0, 1, 9, 22, 217, 12, 18, 144, 49, 142, 73, 240,
                3, 1, 9, 22, 251, 240, 11, 6, 9, 22, 217, 15, 19, 217, 16, 22, 144, 113, 149, 9,
                225, 16, 22, 144, 113, 149, 9, 248, 220, 15, 25, 242, 0, 1, 9, 22, 217, 12, 18,
                144, 49, 142, 73, 240, 3, 1, 9, 22, 253,
            ][..],
            &[
                130, 8, 55, 0, 139, 19, 32, 0, 160, 17, 52, 3, 179, 1, 52, 3, 183, 1, 65, 36, 3,
                185, 10, 65, 13, 4, 193, 3, 1, 65, 36, 3, 193, 4, 8, 65, 13, 4, 193, 12, 1, 65, 36,
                3, 193, 13, 17, 65, 33, 7, 193, 30, 2, 65, 36, 3, 193, 32, 1, 65, 33, 7, 193, 33,
                3, 65, 36, 3,
            ][..],
        ),
        (
            "def f(flag, guard, action):\n    try:\n        try:\n            if flag:\n                action(1)\n            else:\n                action(2)\n        except ValueError:\n            action(3)\n    except RuntimeError:\n        action(4)\n",
            &[
                151, 0, 9, 0, 9, 0, 124, 0, 114, 9, 2, 0, 124, 2, 100, 1, 171, 1, 0, 0, 0, 0, 0, 0,
                1, 0, 110, 9, 2, 0, 124, 2, 100, 2, 171, 1, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0, 121, 0,
                35, 0, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 11, 1, 0, 2, 0, 124, 2, 100, 3,
                171, 1, 0, 0, 0, 0, 0, 0, 1, 0, 89, 0, 121, 0, 119, 0, 120, 3, 89, 0, 119, 1, 35,
                0, 116, 2, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 11, 1, 0, 2, 0, 124, 2, 100, 4, 171,
                1, 0, 0, 0, 0, 0, 0, 1, 0, 89, 0, 121, 0, 119, 0, 120, 3, 89, 0, 119, 1,
            ][..],
            &[
                128, 0, 240, 2, 9, 5, 18, 240, 2, 6, 9, 22, 217, 15, 19, 217, 16, 22, 144, 113,
                149, 9, 225, 16, 22, 144, 113, 149, 9, 240, 5, 0, 17, 26, 248, 244, 6, 0, 16, 26,
                242, 0, 1, 9, 22, 217, 12, 18, 144, 49, 142, 73, 240, 3, 1, 9, 22, 251, 228, 11,
                23, 242, 0, 1, 5, 18, 217, 8, 14, 136, 113, 142, 9, 240, 3, 1, 5, 18, 250,
            ][..],
            &[
                131, 19, 24, 0, 152, 17, 44, 3, 169, 1, 47, 0, 171, 1, 44, 3, 172, 3, 47, 0, 175,
                17, 65, 3, 3, 193, 2, 1, 65, 3, 3,
            ][..],
        ),
    ] {
        let module = compile(source, PythonVersion::Py312);
        let function = child(&module, "f");
        assert_eq!(function.bytecode, expected_code, "{source}");
        assert_eq!(function.line_table, expected_lines, "{source}");
        assert_eq!(function.exception_table, expected_exceptions, "{source}");
    }
}

#[test]
fn python_312_nested_finally_condition_keeps_shared_footer_order() {
    let module = compile(
        "def f(a, b):\n    try:\n        work()\n    finally:\n        if a:\n            if b:\n                finish()\n        else:\n            other()\n",
        PythonVersion::Py312,
    );
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        [
            151, 0, 9, 0, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 124, 0,
            114, 13, 124, 1, 114, 22, 116, 3, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1,
            0, 121, 0, 116, 5, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0, 121,
            0, 35, 0, 124, 0, 114, 14, 124, 1, 114, 11, 116, 3, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0,
            0, 0, 0, 0, 0, 1, 0, 119, 0, 119, 0, 116, 5, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0,
            0, 0, 0, 1, 0, 119, 0, 120, 3, 89, 0, 119, 1
        ]
    );
    assert_eq!(
        function.exception_table,
        [130, 10, 39, 0, 167, 28, 65, 3, 3]
    );
    assert_eq!(
        function.line_table,
        [
            128, 0, 240, 2, 7, 5, 20, 220, 8, 12, 140, 6, 225, 11, 12, 217, 15, 16, 220, 16, 22,
            149, 8, 228, 12, 17, 141, 71, 240, 7, 0, 16, 17, 248, 241, 3, 0, 12, 13, 217, 15, 16,
            220, 16, 22, 149, 8, 240, 3, 0, 16, 17, 244, 6, 0, 13, 18, 141, 71, 250
        ]
    );
}

#[test]
fn python312_compound_match_tails_keep_the_original_shared_footer() {
    // Captured from the pinned CPython 3.12.14 compiler.
    let module = compile(
        "def choose(value, mode, ready, action):\n    match value:\n        case 1:\n            match mode:\n                case 3:\n                    if ready:\n                        action()\n                case 4:\n                    action()\n                case _:\n                    raise ValueError\n        case _:\n            action()\n",
        PythonVersion::Py312,
    );
    let function = child(&module, "choose");
    assert_eq!(
        function.bytecode.as_slice(),
        &[
            151, 0, 124, 0, 100, 1, 107, 40, 0, 0, 114, 37, 124, 1, 120, 1, 100, 2, 107, 40, 0, 0,
            114, 11, 1, 0, 124, 2, 114, 27, 2, 0, 124, 3, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0,
            100, 3, 107, 40, 0, 0, 114, 8, 2, 0, 124, 3, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0, 9,
            0, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 130, 1, 121, 0, 9, 0, 2, 0, 124, 3, 171, 0, 0, 0, 0,
            0, 0, 0, 1, 0, 121, 0,
        ]
    );
    assert_eq!(
        function.line_table.as_slice(),
        &[
            128, 0, 216, 10, 15, 219, 13, 14, 216, 18, 22, 221, 21, 22, 217, 23, 28, 217, 24, 30,
            157, 8, 219, 21, 22, 217, 20, 26, 149, 72, 216, 21, 22, 220, 26, 36, 208, 20, 36, 240,
            11, 0, 24, 29, 240, 12, 0, 14, 15, 217, 12, 18, 141, 72,
        ]
    );
    assert!(function.exception_table.is_empty());

    let module = compile(
        "def choose(value, mode, ready, action):\n    if mode:\n        match value:\n            case 1:\n                if ready:\n                    action()\n            case _:\n                action()\n    else:\n        match value:\n            case 2:\n                if ready:\n                    action()\n            case _:\n                action()\n",
        PythonVersion::Py312,
    );
    let function = child(&module, "choose");
    assert_eq!(
        function.bytecode.as_slice(),
        &[
            151, 0, 124, 1, 114, 25, 124, 0, 100, 1, 107, 40, 0, 0, 114, 10, 124, 2, 114, 17, 2, 0,
            124, 3, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0, 9, 0, 2, 0, 124, 3, 171, 0, 0, 0, 0, 0,
            0, 0, 1, 0, 121, 0, 121, 0, 124, 0, 100, 2, 107, 40, 0, 0, 114, 11, 124, 2, 114, 8, 2,
            0, 124, 3, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0, 121, 0, 9, 0, 2, 0, 124, 3, 171, 0,
            0, 0, 0, 0, 0, 0, 1, 0, 121, 0,
        ]
    );
    assert_eq!(
        function.line_table.as_slice(),
        &[
            128, 0, 217, 7, 11, 216, 14, 19, 219, 17, 18, 217, 19, 24, 217, 20, 26, 149, 72, 216,
            17, 18, 217, 16, 22, 149, 8, 240, 7, 0, 20, 25, 240, 10, 0, 15, 20, 219, 17, 18, 217,
            19, 24, 217, 20, 26, 149, 72, 240, 3, 0, 20, 25, 224, 17, 18, 217, 16, 22, 149, 8,
        ]
    );
    assert!(function.exception_table.is_empty());
}

#[test]
fn python_312_adjacent_finalizers_retire_their_owners_in_unwind_order() {
    // FINALLY_END keeps SWAP/POP_TOP/SWAP protected until its POP_BLOCK.
    // The outer FINALLY_TRY still owns POP_EXCEPT until its own POP_BLOCK.
    let module = compile(
        "async def f(flag, inner, outer):\n    try:\n        try:\n            if flag:\n                return payload\n        finally:\n            if inner:\n                return inner_result\n    finally:\n        if outer:\n            return outer_result\n    return None\n",
        PythonVersion::Py312,
    );
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        [
            75, 0, 1, 0, 151, 0, 9, 0, 9, 0, 124, 0, 114, 37, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 124,
            1, 114, 18, 116, 2, 0, 0, 0, 0, 0, 0, 0, 0, 99, 2, 1, 0, 124, 2, 114, 8, 116, 4, 0, 0,
            0, 0, 0, 0, 0, 0, 99, 2, 1, 0, 83, 0, 83, 0, 9, 0, 124, 2, 114, 8, 116, 4, 0, 0, 0, 0,
            0, 0, 0, 0, 99, 2, 1, 0, 83, 0, 83, 0, 9, 0, 124, 1, 114, 16, 116, 2, 0, 0, 0, 0, 0, 0,
            0, 0, 124, 2, 114, 8, 116, 4, 0, 0, 0, 0, 0, 0, 0, 0, 99, 2, 1, 0, 83, 0, 83, 0, 9, 0,
            124, 2, 114, 6, 116, 4, 0, 0, 0, 0, 0, 0, 0, 0, 83, 0, 121, 0, 35, 0, 124, 1, 114, 20,
            116, 2, 0, 0, 0, 0, 0, 0, 0, 0, 99, 2, 1, 0, 99, 2, 89, 0, 124, 2, 114, 8, 116, 4, 0,
            0, 0, 0, 0, 0, 0, 0, 99, 2, 1, 0, 83, 0, 83, 0, 119, 0, 120, 3, 89, 0, 119, 1, 35, 0,
            124, 2, 114, 10, 116, 4, 0, 0, 0, 0, 0, 0, 0, 0, 99, 2, 1, 0, 99, 2, 89, 0, 83, 0, 119,
            0, 120, 3, 89, 0, 119, 1, 173, 3, 119, 1
        ]
    );
    assert_eq!(
        function.line_table,
        [
            232, 0, 248, 128, 0, 240, 2, 9, 5, 32, 240, 2, 5, 9, 36, 217, 15, 19, 220, 23, 30, 225,
            15, 20, 220, 23, 35, 209, 16, 35, 225, 11, 16, 220, 19, 31, 210, 12, 31, 240, 3, 0, 12,
            17, 240, 7, 0, 16, 21, 241, 6, 0, 12, 17, 220, 19, 31, 210, 12, 31, 240, 3, 0, 12, 17,
            240, 13, 0, 16, 20, 241, 6, 0, 16, 21, 220, 23, 35, 225, 11, 16, 220, 19, 31, 210, 12,
            31, 240, 3, 0, 12, 17, 240, 7, 0, 16, 21, 241, 6, 0, 12, 17, 220, 19, 31, 208, 12, 31,
            216, 11, 15, 248, 241, 11, 0, 16, 21, 220, 23, 35, 211, 16, 35, 225, 11, 16, 220, 19,
            31, 210, 12, 31, 240, 3, 0, 12, 17, 240, 7, 0, 16, 21, 251, 241, 6, 0, 12, 17, 220, 19,
            31, 212, 12, 31, 240, 3, 0, 12, 17, 252
        ]
    );
    assert_eq!(
        function.exception_table,
        [
            130, 1, 65, 53, 1, 133, 7, 65, 9, 0, 140, 9, 65, 36, 0, 149, 11, 65, 53, 1, 161, 11,
            65, 53, 1, 173, 7, 65, 36, 0, 180, 11, 65, 53, 1, 193, 0, 9, 65, 53, 1, 193, 9, 11, 65,
            33, 3, 193, 20, 1, 65, 36, 0, 193, 21, 11, 65, 53, 1, 193, 32, 1, 65, 33, 3, 193, 33,
            3, 65, 36, 0, 193, 36, 11, 65, 50, 3, 193, 47, 2, 65, 53, 1, 193, 49, 1, 65, 50, 3,
            193, 50, 3, 65, 53, 1
        ]
    );
}

#[test]
fn python_313_adjacent_finalizers_retire_their_owners_in_unwind_order() {
    // FINALLY_END keeps SWAP/POP_TOP/SWAP protected until its POP_BLOCK.
    // The outer FINALLY_TRY still owns POP_EXCEPT until its own POP_BLOCK.
    let module = compile(
        "async def f(flag, inner, outer):\n    try:\n        try:\n            if flag:\n                return payload\n        finally:\n            if inner:\n                return inner_result\n    finally:\n        if outer:\n            return outer_result\n    return None\n",
        PythonVersion::Py313,
    );
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        [
            35, 0, 32, 0, 149, 0, 30, 0, 30, 0, 85, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 52, 0, 0, 91,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 85, 1, 40, 0, 0, 0, 0, 0, 0, 0, 97, 23, 0, 0, 91, 2, 0, 0,
            0, 0, 0, 0, 0, 0, 115, 2, 32, 0, 85, 2, 40, 0, 0, 0, 0, 0, 0, 0, 97, 8, 0, 0, 91, 4, 0,
            0, 0, 0, 0, 0, 0, 0, 115, 2, 32, 0, 36, 0, 36, 0, 30, 0, 85, 2, 40, 0, 0, 0, 0, 0, 0,
            0, 97, 8, 0, 0, 91, 4, 0, 0, 0, 0, 0, 0, 0, 0, 115, 2, 32, 0, 36, 0, 36, 0, 30, 0, 85,
            1, 40, 0, 0, 0, 0, 0, 0, 0, 97, 21, 0, 0, 91, 2, 0, 0, 0, 0, 0, 0, 0, 0, 85, 2, 40, 0,
            0, 0, 0, 0, 0, 0, 97, 8, 0, 0, 91, 4, 0, 0, 0, 0, 0, 0, 0, 0, 115, 2, 32, 0, 36, 0, 36,
            0, 30, 0, 85, 2, 40, 0, 0, 0, 0, 0, 0, 0, 97, 6, 0, 0, 91, 4, 0, 0, 0, 0, 0, 0, 0, 0,
            36, 0, 30, 0, 103, 0, 33, 0, 85, 1, 40, 0, 0, 0, 0, 0, 0, 0, 97, 25, 0, 0, 91, 2, 0, 0,
            0, 0, 0, 0, 0, 0, 115, 2, 32, 0, 115, 2, 31, 0, 85, 2, 40, 0, 0, 0, 0, 0, 0, 0, 97, 8,
            0, 0, 91, 4, 0, 0, 0, 0, 0, 0, 0, 0, 115, 2, 32, 0, 36, 0, 36, 0, 102, 0, 61, 3, 31, 0,
            102, 1, 33, 0, 85, 2, 40, 0, 0, 0, 0, 0, 0, 0, 97, 10, 0, 0, 91, 4, 0, 0, 0, 0, 0, 0,
            0, 0, 115, 2, 32, 0, 115, 2, 31, 0, 36, 0, 102, 0, 61, 3, 31, 0, 102, 1, 55, 3, 102, 1
        ]
    );
    assert_eq!(
        function.line_table,
        [
            233, 0, 128, 0, 240, 2, 9, 5, 32, 240, 2, 5, 9, 36, 222, 15, 19, 220, 23, 30, 230, 15,
            20, 220, 23, 35, 209, 16, 35, 230, 11, 16, 220, 19, 31, 210, 12, 31, 240, 3, 0, 12, 17,
            240, 7, 0, 16, 21, 246, 6, 0, 12, 17, 220, 19, 31, 210, 12, 31, 240, 3, 0, 12, 17, 240,
            13, 0, 16, 20, 246, 6, 0, 16, 21, 220, 23, 35, 230, 11, 16, 220, 19, 31, 210, 12, 31,
            240, 3, 0, 12, 17, 240, 7, 0, 16, 21, 246, 6, 0, 12, 17, 220, 19, 31, 208, 12, 31, 240,
            3, 0, 12, 17, 224, 11, 15, 248, 246, 11, 0, 16, 21, 220, 23, 35, 211, 16, 35, 230, 11,
            16, 220, 19, 31, 210, 12, 31, 240, 3, 0, 12, 17, 240, 7, 0, 16, 21, 251, 246, 6, 0, 12,
            17, 220, 19, 31, 212, 12, 31, 240, 3, 0, 12, 17, 252
        ]
    );
    assert_eq!(
        function.exception_table,
        [
            130, 1, 66, 40, 1, 133, 12, 65, 45, 0, 145, 14, 66, 18, 0, 159, 16, 66, 40, 1, 176, 16,
            66, 40, 1, 193, 1, 12, 66, 18, 0, 193, 13, 16, 66, 40, 1, 193, 30, 15, 66, 40, 1, 193,
            45, 16, 66, 15, 3, 193, 61, 1, 66, 18, 0, 193, 62, 16, 66, 40, 1, 194, 14, 1, 66, 15,
            3, 194, 15, 3, 66, 18, 0, 194, 18, 16, 66, 37, 3, 194, 34, 2, 66, 40, 1, 194, 36, 1,
            66, 37, 3, 194, 37, 3, 66, 40, 1
        ]
    );
}

#[test]
fn python312_inlined_comprehension_keeps_native_jump_seed_at_one_byte_boundary() {
    // Captured from CPython 3.12.14. This scope exceeds the uniform seed bound;
    // its comprehension must bind all five original labels so the cold await
    // bridge converges to 255 without an unnecessary EXTENDED_ARG at 256.
    let mut source = String::from("async def f(xs, flag, work):\n");
    source.push_str(&"    if flag:\n        value = 0\n".repeat(60));
    source.push_str("    values = [item for item in xs]\n    await work()\n");
    source.push_str(&"    value = 0\n".repeat(122));
    source.push_str("    pass\n    return None\n");
    let module = compile(&source, PythonVersion::Py312);
    let function = child(&module, "f");
    let mut bytecode = vec![75, 0, 1, 0, 151, 0];
    bytecode.extend([124, 1, 114, 2, 100, 1, 125, 3].repeat(60));
    bytecode.extend([
        124, 0, 68, 0, 143, 4, 99, 2, 103, 0, 99, 2, 93, 4, 0, 0, 125, 4, 124, 4, 145, 2, 140, 6,
        4, 0, 125, 5, 125, 4, 2, 0, 124, 2, 171, 0, 0, 0, 0, 0, 0, 0, 131, 0, 100, 0, 123, 3, 0, 0,
        150, 2, 151, 3, 134, 5, 5, 0, 1, 0,
    ]);
    bytecode.extend([100, 1, 125, 3].repeat(122));
    bytecode.extend([
        9, 0, 121, 0, 99, 2, 1, 0, 99, 2, 125, 4, 119, 0, 55, 0, 140, 255, 173, 3, 119, 1,
    ]);
    assert_eq!(function.bytecode, bytecode);
    assert_eq!(
        function.exception_table,
        [
            130, 67, 53, 72, 14, 1, 195, 55, 9, 72, 7, 4, 196, 0, 12, 72, 14, 1, 196, 12, 1, 72,
            12, 4, 196, 13, 68, 0, 72, 14, 1,
        ]
    );
}

#[test]
fn python312_handler_nested_if_preserves_original_cleanup_records() {
    // Captured from the pinned CPython 3.12.14 compiler.
    let module = compile(
        "def read(action, ready):\n    try:\n        action()\n    except ValueError:\n        try:\n            if ready:\n                action()\n            else:\n                action()\n        except TypeError:\n            action()\n",
        PythonVersion::Py312,
    );
    let function = child(&module, "read");
    assert_eq!(
        function.bytecode.as_slice(),
        &[
            151, 0, 9, 0, 2, 0, 124, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0, 35, 0, 116, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 46, 1, 0, 9, 0, 124, 1, 114, 8, 2, 0, 124, 0, 171, 0,
            0, 0, 0, 0, 0, 0, 1, 0, 110, 9, 2, 0, 124, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 89, 0,
            121, 0, 89, 0, 121, 0, 35, 0, 116, 2, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 11, 1, 0, 2,
            0, 124, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 89, 0, 89, 0, 121, 0, 119, 0, 120, 3, 89, 0,
            119, 1, 119, 0, 120, 3, 89, 0, 119, 1,
        ]
    );
    assert_eq!(
        function.line_table.as_slice(),
        &[
            128, 0, 240, 2, 9, 5, 21, 217, 8, 14, 141, 8, 248, 220, 11, 21, 242, 0, 7, 5, 21, 240,
            2, 6, 9, 21, 217, 15, 20, 217, 16, 22, 149, 8, 225, 16, 22, 150, 8, 241, 5, 0, 17, 25,
            248, 244, 6, 0, 16, 25, 242, 0, 1, 9, 21, 217, 12, 18, 143, 72, 240, 3, 1, 9, 21, 250,
            240, 13, 7, 5, 21, 250,
        ]
    );
    assert_eq!(
        function.exception_table.as_slice(),
        &[
            130, 7, 10, 0, 138, 9, 65, 1, 3, 148, 17, 41, 2, 169, 16, 61, 5, 185, 1, 65, 1, 3, 188,
            1, 61, 5, 189, 4, 65, 1, 3,
        ]
    );
}

#[test]
fn python312_nested_handler_attribute_preserves_original_cleanup_records() {
    // Captured from the pinned CPython 3.12.14 compiler.
    let module = compile(
        "def read(value, decode):\n    try:\n        result = decode(value)\n    except ValueError:\n        try:\n            result = decode(value.upper())\n        except KeyError:\n            return None\n    return result.attribute\n",
        PythonVersion::Py312,
    );
    let function = child(&module, "read");
    assert_eq!(
        function.bytecode.as_slice(),
        &[
            151, 0, 9, 0, 2, 0, 124, 1, 124, 0, 171, 1, 0, 0, 0, 0, 0, 0, 125, 2, 124, 2, 106, 6,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 83, 0, 35, 0, 116, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 36, 0, 114, 54, 1, 0, 9, 0, 2, 0, 124, 1, 124, 0, 106, 3, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 171, 1, 0, 0, 0, 0, 0, 0,
            125, 2, 110, 16, 35, 0, 116, 4, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 4, 1, 0, 89, 0, 89,
            0, 121, 0, 119, 0, 120, 3, 89, 0, 119, 1, 89, 0, 124, 2, 106, 6, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 83, 0, 119, 0, 120, 3, 89, 0, 119, 1,
        ]
    );
    assert_eq!(
        function.line_table.as_slice(),
        &[
            128, 0, 240, 2, 6, 5, 24, 217, 17, 23, 152, 5, 147, 29, 136, 6, 240, 12, 0, 12, 18,
            215, 11, 27, 209, 11, 27, 208, 4, 27, 248, 244, 11, 0, 12, 22, 242, 0, 4, 5, 24, 240,
            2, 3, 9, 24, 217, 21, 27, 152, 69, 159, 75, 153, 75, 155, 77, 211, 21, 42, 137, 70,
            248, 220, 15, 23, 242, 0, 1, 9, 24, 218, 19, 23, 240, 3, 1, 9, 24, 250, 240, 3, 0, 13,
            19, 240, 6, 0, 12, 18, 215, 11, 27, 209, 11, 27, 208, 4, 27, 240, 11, 4, 5, 24, 250,
        ]
    );
    assert_eq!(
        function.exception_table.as_slice(),
        &[
            130, 8, 22, 0, 150, 9, 65, 21, 3, 160, 22, 55, 2, 182, 1, 65, 21, 3, 183, 9, 65, 4, 5,
            193, 0, 1, 65, 21, 3, 193, 3, 1, 65, 4, 5, 193, 4, 3, 65, 21, 3, 193, 20, 1, 65, 21, 3,
        ]
    );
}

#[test]
fn python312_handler_conditional_call_preserves_original_cleanup_records() {
    // Captured from the pinned CPython 3.12.14 compiler.
    let module = compile(
        "def read(action, ready, value, fallback):\n    try:\n        action()\n    except ValueError:\n        action(value if ready else fallback.attribute)\n",
        PythonVersion::Py312,
    );
    let function = child(&module, "read");
    assert_eq!(
        function.bytecode.as_slice(),
        &[
            151, 0, 9, 0, 2, 0, 124, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0, 35, 0, 116, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 25, 1, 0, 2, 0, 124, 0, 124, 1, 114, 2, 124, 2, 110,
            11, 124, 3, 106, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 171, 1, 0, 0,
            0, 0, 0, 0, 1, 0, 89, 0, 121, 0, 119, 0, 120, 3, 89, 0, 119, 1,
        ]
    );
    assert_eq!(
        function.line_table.as_slice(),
        &[
            128, 0, 240, 2, 3, 5, 55, 217, 8, 14, 141, 8, 248, 220, 11, 21, 242, 0, 1, 5, 55, 217,
            8, 14, 153, 5, 137, 117, 160, 56, 215, 35, 53, 209, 35, 53, 214, 8, 54, 240, 3, 1, 5,
            55, 250,
        ]
    );
    assert_eq!(
        function.exception_table.as_slice(),
        &[130, 7, 10, 0, 138, 31, 44, 3, 171, 1, 44, 3,]
    );
}

#[test]
fn python312_outer_try_return_controls_preserves_original_cleanup_records() {
    // Captured from the pinned CPython 3.12.14 compiler.
    let module = compile(
        "# Invented public candidates. AST parsed only; bytecode regressions unverified.\n\n\ndef conditional_handler_return(flag, value, alternate, fail):\n    try:\n        fail()\n    except Exception:\n        return (\n            value\n            if flag\n            else alternate()\n        )\n\n\ndef conditional_handler_return_nested(flag, value, alternate, fail):\n    try:\n        try:\n            fail()\n        except Exception:\n            return (\n                value\n                if flag\n                else alternate()\n            )\n    except Exception:\n        return value\n\n\ndef finally_with_suppressed_cleanup_error(fail, produce, cleanup):\n    try:\n        fail()\n    except Exception:\n        try:\n            return produce()\n        finally:\n            try:\n                cleanup()\n            except OSError:\n                pass\n\n\ndef finally_with_outer_named_handler(fail, produce, cleanup):\n    try:\n        try:\n            fail()\n        except Exception:\n            try:\n                return produce()\n            finally:\n                try:\n                    cleanup()\n                except OSError:\n                    pass\n    except Exception as error:\n        raise RuntimeError(\"failed\") from error\n",
        PythonVersion::Py312,
    );
    let function = child(&module, "conditional_handler_return_nested");
    assert_eq!(
        function.bytecode.as_slice(),
        &[
            151, 0, 9, 0, 9, 0, 2, 0, 124, 3, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0, 35, 0, 116,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 14, 1, 0, 124, 0, 114, 2, 124, 1, 110, 6, 2, 0,
            124, 2, 171, 0, 0, 0, 0, 0, 0, 0, 99, 2, 89, 0, 83, 0, 119, 0, 120, 3, 89, 0, 119, 1,
            35, 0, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 5, 1, 0, 124, 1, 99, 2, 89, 0, 83,
            0, 119, 0, 120, 3, 89, 0, 119, 1,
        ]
    );
    assert_eq!(
        function.line_table.as_slice(),
        &[
            128, 0, 240, 2, 10, 5, 21, 240, 2, 7, 9, 14, 217, 12, 16, 141, 70, 248, 220, 15, 24,
            242, 0, 5, 9, 14, 241, 6, 0, 20, 24, 241, 3, 0, 17, 22, 225, 21, 30, 147, 91, 242, 7,
            4, 13, 14, 240, 3, 5, 9, 14, 251, 244, 12, 0, 12, 21, 242, 0, 1, 5, 21, 216, 15, 20,
            138, 12, 240, 3, 1, 5, 21, 250,
        ]
    );
    assert_eq!(
        function.exception_table.as_slice(),
        &[
            131, 7, 11, 0, 139, 20, 34, 3, 159, 1, 37, 0, 161, 1, 34, 3, 162, 3, 37, 0, 165, 11,
            51, 3, 178, 1, 51, 3,
        ]
    );
}

#[test]
fn python312_finally_loop_break_keeps_the_shared_footer() {
    // Captured from the pinned CPython 3.12 compiler.
    let source = "def f(action, values, manager):\n    try:\n        action()\n    finally:\n        for value in values:\n            with manager:\n                action()\n            break\n";
    let module = compile(source, PythonVersion::Py312);
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        &[
            151, 0, 9, 0, 2, 0, 124, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 124, 1, 68, 0, 93, 21, 0,
            0, 125, 3, 124, 2, 53, 0, 1, 0, 2, 0, 124, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 100, 0,
            100, 0, 100, 0, 171, 2, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 121, 0, 4, 0, 121, 0, 35, 0, 49,
            0, 115, 1, 119, 2, 1, 0, 89, 0, 1, 0, 1, 0, 140, 13, 120, 3, 89, 0, 119, 1, 35, 0, 124,
            1, 68, 0, 93, 34, 0, 0, 125, 3, 124, 2, 53, 0, 1, 0, 2, 0, 124, 0, 171, 0, 0, 0, 0, 0,
            0, 0, 1, 0, 100, 0, 100, 0, 100, 0, 171, 2, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 119, 0, 35,
            0, 49, 0, 115, 1, 119, 2, 1, 0, 89, 0, 1, 0, 1, 0, 1, 0, 119, 0, 120, 3, 89, 0, 119, 1,
            4, 0, 119, 0, 120, 3, 89, 0, 119, 1
        ]
    );
    assert_eq!(
        function.line_table,
        &[
            128, 0, 240, 2, 6, 5, 18, 217, 8, 14, 140, 8, 227, 21, 27, 136, 69, 218, 17, 24, 217,
            16, 22, 148, 8, 247, 3, 0, 18, 25, 225, 12, 17, 241, 7, 0, 22, 28, 223, 17, 24, 144,
            23, 251, 243, 3, 0, 22, 28, 136, 69, 218, 17, 24, 217, 16, 22, 148, 8, 247, 3, 0, 18,
            25, 225, 12, 17, 247, 5, 0, 18, 25, 225, 12, 17, 250, 241, 7, 0, 22, 28, 250
        ]
    );
    assert_eq!(
        function.exception_table,
        &[
            130, 7, 48, 0, 144, 8, 36, 5, 164, 5, 45, 9, 176, 8, 65, 25, 3, 184, 8, 65, 10, 9, 193,
            0, 10, 65, 25, 3, 193, 10, 5, 65, 20, 13, 193, 15, 10, 65, 25, 3
        ]
    );
}

#[test]
fn python312_terminal_if_keeps_warm_footer_after_cold_continuation() {
    // Captured from the pinned CPython 3.12 compiler.
    for (source, expected_code, expected_lines, expected_exceptions) in [
        (
            "async def f(flag, values, action):\n    if flag:\n        async for value in values:\n            action(value)\n        action()\n",
            &[
                75, 0, 1, 0, 151, 0, 124, 0, 114, 21, 124, 1, 50, 0, 51, 0, 100, 0, 123, 3, 0, 0,
                150, 3, 151, 3, 134, 5, 5, 0, 125, 3, 2, 0, 124, 2, 124, 3, 171, 1, 0, 0, 0, 0, 0,
                0, 1, 0, 140, 18, 121, 0, 121, 0, 55, 0, 140, 15, 54, 0, 2, 0, 124, 2, 171, 0, 0,
                0, 0, 0, 0, 0, 1, 0, 140, 13, 173, 3, 119, 1,
            ][..],
            &[
                232, 0, 248, 128, 0, 217, 7, 11, 217, 27, 33, 247, 0, 1, 9, 26, 144, 37, 217, 12,
                18, 144, 53, 141, 77, 248, 240, 5, 0, 8, 12, 240, 2, 1, 9, 26, 248, 152, 54, 225,
                8, 14, 140, 8, 250,
            ][..],
            &[
                130, 5, 38, 1, 135, 4, 29, 2, 139, 1, 27, 6, 140, 3, 29, 2, 143, 12, 38, 1, 155, 1,
                29, 2, 157, 8, 38, 1,
            ][..],
        ),
        (
            "async def f(flag, values):\n    if flag:\n        async for value in values:\n            yield value\n        yield None\n",
            &[
                75, 0, 1, 0, 151, 0, 124, 0, 114, 18, 124, 1, 50, 0, 51, 0, 100, 0, 123, 3, 0, 0,
                150, 3, 151, 3, 134, 5, 5, 0, 125, 2, 124, 2, 173, 4, 150, 1, 151, 1, 1, 0, 140,
                15, 121, 0, 121, 0, 55, 0, 140, 12, 54, 0, 100, 0, 173, 4, 150, 1, 151, 1, 1, 0,
                140, 11, 173, 3, 119, 1,
            ][..],
            &[
                232, 0, 248, 128, 0, 217, 7, 11, 217, 27, 33, 247, 0, 1, 9, 24, 144, 37, 216, 18,
                23, 140, 75, 248, 240, 5, 0, 8, 12, 240, 2, 1, 9, 24, 248, 152, 54, 224, 14, 18,
                139, 10, 250,
            ][..],
            &[
                130, 5, 33, 1, 135, 4, 26, 2, 139, 1, 24, 6, 140, 3, 26, 2, 143, 9, 33, 1, 152, 1,
                26, 2, 154, 6, 33, 1,
            ][..],
        ),
        (
            "def f(flag, action, valid):\n    if flag:\n        try:\n            action()\n            assert False\n        except ValueError:\n            assert valid\n        assert valid\n",
            &[
                151, 0, 124, 0, 114, 11, 9, 0, 2, 0, 124, 1, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 74, 0,
                130, 1, 121, 0, 121, 0, 35, 0, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 7, 1, 0,
                124, 2, 115, 2, 74, 0, 130, 1, 89, 0, 110, 4, 119, 0, 120, 3, 89, 0, 119, 1, 124,
                2, 114, 1, 140, 24, 74, 0, 130, 1,
            ][..],
            &[
                128, 0, 217, 7, 11, 240, 2, 4, 9, 25, 217, 12, 18, 140, 72, 216, 12, 24, 144, 53,
                240, 6, 0, 16, 21, 240, 13, 0, 8, 12, 248, 244, 8, 0, 16, 26, 242, 0, 1, 9, 25,
                217, 19, 24, 136, 76, 146, 53, 240, 3, 1, 9, 25, 250, 226, 15, 20, 136, 12, 136,
                117,
            ][..],
            &[132, 9, 15, 0, 143, 13, 31, 3, 158, 1, 31, 3][..],
        ),
        (
            "def f(flag, action):\n    if flag:\n        try:\n            while True:\n                action()\n        except ValueError:\n            action()\n        action()\n",
            &[
                151, 0, 124, 0, 114, 11, 9, 0, 9, 0, 2, 0, 124, 1, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0,
                140, 8, 121, 0, 121, 0, 35, 0, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 10, 1,
                0, 2, 0, 124, 1, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 89, 0, 110, 4, 119, 0, 120, 3, 89,
                0, 119, 1, 2, 0, 124, 1, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 140, 32,
            ][..],
            &[
                128, 0, 217, 7, 11, 240, 2, 4, 9, 21, 216, 18, 22, 217, 16, 22, 148, 8, 240, 3, 0,
                19, 23, 248, 240, 5, 0, 8, 12, 248, 244, 8, 0, 16, 26, 242, 0, 1, 9, 21, 217, 12,
                18, 142, 72, 240, 3, 1, 9, 21, 250, 225, 8, 14, 140, 8, 248,
            ][..],
            &[132, 9, 15, 0, 143, 16, 34, 3, 161, 1, 34, 3][..],
        ),
    ] {
        let module = compile(source, PythonVersion::Py312);
        let function = child(&module, "f");
        assert_eq!(function.bytecode, expected_code, "{source}");
        assert_eq!(function.line_table, expected_lines, "{source}");
        assert_eq!(function.exception_table, expected_exceptions, "{source}");
    }
}

#[test]
fn python312_handler_break_keeps_terminal_if_footer_warm() {
    // Captured from the pinned CPython 3.12 compiler.
    let source = "def f(flag, manager, action):\n    if flag:\n        with manager:\n            while True:\n                try:\n                    action()\n                except ValueError:\n                    break\n        action()\n";
    let module = compile(source, PythonVersion::Py312);
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        &[
            151, 0, 124, 0, 114, 14, 124, 1, 53, 0, 1, 0, 9, 0, 9, 0, 2, 0, 124, 2, 171, 0, 0, 0,
            0, 0, 0, 0, 1, 0, 140, 9, 121, 0, 121, 0, 35, 0, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0,
            114, 3, 1, 0, 89, 0, 110, 4, 119, 0, 120, 3, 89, 0, 119, 1, 9, 0, 100, 0, 100, 0, 100,
            0, 171, 2, 0, 0, 0, 0, 0, 0, 1, 0, 110, 12, 35, 0, 49, 0, 115, 1, 119, 2, 1, 0, 89, 0,
            1, 0, 1, 0, 110, 3, 120, 3, 89, 0, 119, 1, 2, 0, 124, 2, 171, 0, 0, 0, 0, 0, 0, 0, 1,
            0, 140, 47
        ]
    );
    assert_eq!(
        function.line_table,
        &[
            128, 0, 217, 7, 11, 218, 13, 20, 216, 18, 22, 240, 2, 3, 17, 26, 217, 20, 26, 148, 72,
            240, 5, 0, 19, 23, 248, 240, 5, 0, 8, 12, 248, 244, 10, 0, 24, 34, 242, 0, 1, 17, 26,
            217, 20, 25, 240, 3, 1, 17, 26, 250, 216, 20, 25, 247, 11, 0, 14, 21, 143, 87, 137, 87,
            250, 241, 12, 0, 9, 15, 140, 8, 248
        ]
    );
    assert_eq!(
        function.exception_table,
        &[
            133, 2, 43, 3, 136, 7, 18, 2, 143, 1, 43, 3, 146, 9, 30, 5, 155, 2, 43, 3, 157, 1, 30,
            5, 158, 3, 43, 3, 171, 5, 52, 7
        ]
    );
}

#[test]
fn python_312_try_star_normal_exit_keeps_original_pop_block_barrier() {
    // Native 3.12.14 keeps POP_BLOCK ahead of the normal except* exit during
    // jump threading. The with-suppression edge still visits that original array.
    let module = compile(
        r"async def f(manager):
    try:
        with manager():
            while True:
                await work()
    except* Failure as errors:
        result = errors
    finally:
        finish()
    return result
",
        PythonVersion::Py312,
    );
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        [
            75, 0, 1, 0, 151, 0, 9, 0, 2, 0, 124, 0, 171, 0, 0, 0, 0, 0, 0, 0, 53, 0, 1, 0, 9, 0,
            116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 131, 0, 100, 0, 123, 3, 0, 0,
            150, 5, 151, 3, 134, 5, 5, 0, 1, 0, 140, 19, 55, 0, 140, 5, 35, 0, 49, 0, 115, 1, 119,
            2, 1, 0, 89, 0, 1, 0, 1, 0, 110, 3, 120, 3, 89, 0, 119, 1, 110, 39, 35, 0, 103, 0, 120,
            2, 116, 2, 0, 0, 0, 0, 0, 0, 0, 0, 37, 0, 120, 1, 129, 14, 125, 1, 124, 1, 125, 2, 100,
            0, 125, 1, 126, 1, 110, 6, 100, 0, 125, 1, 126, 1, 145, 3, 1, 0, 110, 2, 110, 1, 1, 0,
            145, 1, 174, 1, 120, 1, 128, 3, 1, 0, 89, 0, 110, 6, 99, 2, 89, 0, 119, 0, 120, 3, 89,
            0, 119, 1, 116, 5, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 127, 2, 83,
            0, 35, 0, 116, 5, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 119, 0, 120,
            3, 89, 0, 119, 1, 173, 3, 119, 1,
        ]
    );
    assert_eq!(
        function.line_table,
        [
            232, 0, 248, 128, 0, 240, 2, 7, 5, 17, 217, 13, 20, 141, 89, 216, 18, 22, 220, 22, 26,
            147, 102, 151, 12, 144, 12, 240, 3, 0, 19, 23, 216, 16, 28, 248, 247, 5, 0, 14, 23,
            136, 89, 250, 136, 89, 248, 241, 6, 1, 5, 24, 140, 71, 243, 0, 1, 5, 24, 216, 17, 23,
            140, 6, 253, 136, 6, 240, 3, 1, 5, 24, 255, 252, 244, 6, 0, 9, 15, 140, 8, 216, 11, 17,
            128, 77, 248, 244, 3, 0, 9, 15, 141, 8, 252,
        ]
    );
    assert_eq!(
        function.exception_table,
        [
            130, 1, 65, 49, 1, 132, 7, 47, 0, 139, 15, 34, 3, 154, 1, 32, 6, 155, 6, 34, 3, 162, 5,
            43, 7, 167, 7, 47, 0, 174, 1, 65, 34, 0, 175, 12, 65, 19, 3, 187, 2, 65, 1, 9, 189, 17,
            65, 19, 3, 193, 14, 8, 65, 34, 0, 193, 22, 12, 65, 49, 1, 193, 34, 12, 65, 46, 3, 193,
            46, 3, 65, 49, 1,
        ]
    );
}

#[test]
fn python_312_try_star_pop_counts_in_second_exit_copy_sweep() {
    // Four return instructions plus the retained POP_BLOCK exceed MAX_COPY_SIZE.
    // Native keeps the with-suppression jump and the original normal-exit array.
    let module = compile(
        r"async def f(manager):
    try:
        with manager():
            while True:
                await work()
    except* Failure as errors:
        result = errors
    return (result, manager)
",
        PythonVersion::Py312,
    );
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        [
            75, 0, 1, 0, 151, 0, 9, 0, 2, 0, 124, 0, 171, 0, 0, 0, 0, 0, 0, 0, 53, 0, 1, 0, 9, 0,
            116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 131, 0, 100, 0, 123, 3, 0, 0,
            150, 4, 151, 3, 134, 5, 5, 0, 1, 0, 140, 19, 55, 0, 140, 5, 35, 0, 49, 0, 115, 1, 119,
            2, 1, 0, 89, 0, 1, 0, 1, 0, 110, 3, 120, 3, 89, 0, 119, 1, 9, 0, 127, 2, 124, 0, 102,
            2, 83, 0, 35, 0, 103, 0, 120, 2, 116, 2, 0, 0, 0, 0, 0, 0, 0, 0, 37, 0, 120, 1, 129,
            14, 125, 1, 124, 1, 125, 2, 100, 0, 125, 1, 126, 1, 110, 6, 100, 0, 125, 1, 126, 1,
            145, 3, 1, 0, 110, 2, 110, 1, 1, 0, 145, 1, 174, 1, 120, 1, 128, 6, 1, 0, 89, 0, 127,
            2, 124, 0, 102, 2, 83, 0, 99, 2, 89, 0, 119, 0, 120, 3, 89, 0, 119, 1, 173, 3, 119, 1,
        ]
    );
    assert_eq!(
        function.line_table,
        [
            232, 0, 248, 128, 0, 240, 2, 5, 5, 24, 217, 13, 20, 141, 89, 216, 18, 22, 220, 22, 26,
            147, 102, 151, 12, 144, 12, 240, 3, 0, 19, 23, 216, 16, 28, 248, 247, 5, 0, 14, 23,
            136, 89, 250, 136, 89, 240, 10, 0, 13, 19, 144, 71, 208, 11, 28, 208, 4, 28, 248, 241,
            5, 1, 5, 24, 140, 71, 243, 0, 1, 5, 24, 216, 17, 23, 140, 6, 253, 136, 6, 240, 3, 1, 5,
            24, 253, 224, 12, 18, 144, 71, 208, 11, 28, 208, 4, 28, 255,
        ]
    );
    assert_eq!(
        function.exception_table,
        [
            130, 1, 65, 29, 1, 132, 7, 51, 0, 139, 15, 34, 3, 154, 1, 32, 6, 155, 6, 34, 3, 162, 5,
            43, 7, 167, 7, 51, 0, 175, 4, 65, 29, 1, 179, 12, 65, 26, 3, 191, 2, 65, 5, 9, 193, 1,
            17, 65, 26, 3, 193, 18, 11, 65, 29, 1,
        ]
    );
}

#[test]
fn python312_terminal_expressions_keep_native_copy_and_location_phases() {
    for (source, expected_code, expected_lines, expected_exceptions) in [
        (
            "def f(self, axis):\n    self.value = axis if 0 <= axis < len(self.items) else None\n",
            &[
                151, 0, 100, 1, 124, 1, 99, 2, 120, 2, 107, 26, 0, 0, 114, 31, 116, 1, 0, 0, 0, 0,
                0, 0, 0, 0, 124, 0, 106, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                171, 1, 0, 0, 0, 0, 0, 0, 107, 2, 0, 0, 114, 9, 124, 1, 124, 0, 95, 2, 0, 0, 0, 0,
                0, 0, 0, 0, 121, 0, 1, 0, 100, 0, 124, 0, 95, 2, 0, 0, 0, 0, 0, 0, 0, 0, 121, 0,
            ][..],
            &[
                128, 0, 216, 25, 26, 152, 100, 212, 25, 52, 164, 83, 168, 20, 175, 26, 169, 26,
                163, 95, 210, 25, 52, 144, 20, 128, 68, 133, 74, 208, 25, 52, 184, 36, 128, 68,
                133, 74,
            ][..],
            &[][..],
        ),
        (
            "def f(items, flag):\n    {use(x) for x in items} if flag else None\n",
            &[
                151, 0, 124, 1, 114, 26, 124, 0, 68, 0, 143, 2, 99, 2, 104, 0, 99, 2, 93, 13, 0, 0,
                125, 2, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 124, 2, 171, 1, 0, 0, 0, 0, 0, 0, 146, 2,
                140, 15, 4, 0, 99, 2, 125, 2, 1, 0, 121, 0, 100, 0, 1, 0, 121, 0, 99, 2, 1, 0, 99,
                2, 125, 2, 119, 0,
            ][..],
            &[
                128, 0, 217, 31, 35, 145, 85, 211, 4, 27, 145, 85, 144, 1, 132, 83, 136, 17, 133,
                86, 144, 85, 211, 4, 27, 170, 20, 249, 210, 4, 27,
            ][..],
            &[135, 18, 32, 4][..],
        ),
    ] {
        let module = compile(source, PythonVersion::Py312);
        let function = child(&module, "f");
        assert_eq!(function.bytecode, expected_code, "{source}");
        assert_eq!(function.line_table, expected_lines, "{source}");
        assert_eq!(function.exception_table, expected_exceptions, "{source}");
    }
}

#[test]
fn python312_folded_false_if_preserves_shared_literal_footer() {
    let module = compile(
        "if flag:\n    if True:\n        use()\n    if False:\n        other()\n\"unused\"\n",
        PythonVersion::Py312,
    );
    assert_eq!(
        module.bytecode,
        [
            151, 0, 101, 0, 114, 10, 9, 0, 2, 0, 101, 1, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 9, 0, 121,
            1, 121, 1
        ]
    );
    assert_eq!(
        module.line_table,
        [
            240, 3, 1, 1, 1, 217, 3, 7, 216, 7, 11, 217, 8, 11, 140, 5, 216, 7, 12, 225, 0, 8
        ]
    );
    assert!(module.exception_table.is_empty());
}

#[test]
fn python312_try_match_pass_keeps_its_separate_unlocated_pop() {
    // Captured from the pinned CPython 3.12.14 compiler.
    let module = compile(
        "# Original public reducer; staged without compilation or execution.\ndef update(kind, value, enabled, target):\n    try:\n        match kind:\n            case \"first\":\n                if value is not None and enabled:\n                    target.append(value)\n            case \"second\":\n                target.append(value if enabled else None)\n            case _:\n                pass\n    except ValueError:\n        pass\n",
        PythonVersion::Py312,
    );
    let function = child(&module, "update");
    assert_eq!(
        function.bytecode.as_slice(),
        &[
            151, 0, 9, 0, 124, 0, 120, 1, 100, 1, 107, 40, 0, 0, 114, 23, 1, 0, 124, 1, 129, 49,
            124, 2, 114, 46, 124, 3, 106, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            124, 1, 171, 1, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0, 100, 2, 107, 40, 0, 0, 114, 22, 124, 3,
            106, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 124, 2, 114, 2, 124, 1,
            110, 1, 100, 0, 171, 1, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0, 9, 0, 121, 0, 121, 0, 121, 0,
            35, 0, 116, 2, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 3, 1, 0, 89, 0, 121, 0, 119, 0, 120,
            3, 89, 0, 119, 1,
        ]
    );
    assert_eq!(
        function.line_table.as_slice(),
        &[
            128, 0, 240, 2, 10, 5, 13, 216, 14, 18, 221, 17, 24, 216, 19, 24, 208, 19, 36, 169, 23,
            216, 20, 26, 151, 77, 145, 77, 160, 37, 213, 20, 40, 219, 17, 25, 216, 16, 22, 151, 13,
            145, 13, 161, 119, 153, 101, 176, 68, 213, 16, 57, 216, 17, 18, 216, 16, 20, 240, 11,
            0, 42, 49, 208, 19, 36, 248, 244, 12, 0, 12, 22, 242, 0, 1, 5, 13, 217, 8, 12, 240, 3,
            1, 5, 13, 250,
        ]
    );
    assert_eq!(
        function.exception_table.as_slice(),
        &[
            130, 28, 61, 0, 159, 25, 61, 0, 185, 1, 61, 0, 189, 9, 65, 9, 3, 193, 8, 1, 65, 9, 3,
        ]
    );
}

#[test]
fn python_312_folded_false_try_retains_the_normal_exit_owner() {
    let module = compile(
        "async def f(wait):\n    try:\n        if False:\n            await wait()\n    except RuntimeError:\n        pass\n    await wait()\n",
        PythonVersion::Py312,
    );
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        [
            75, 0, 1, 0, 151, 0, 9, 0, 9, 0, 2, 0, 124, 0, 171, 0, 0, 0, 0, 0, 0, 0, 131, 0, 100,
            0, 123, 3, 0, 0, 150, 2, 151, 3, 134, 5, 5, 0, 1, 0, 121, 0, 35, 0, 116, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 36, 0, 114, 3, 1, 0, 89, 0, 140, 27, 119, 0, 120, 3, 89, 0, 119, 1, 55, 0,
            140, 20, 173, 3, 119, 1
        ]
    );
    assert_eq!(
        function.line_table,
        [
            232, 0, 248, 128, 0, 240, 2, 4, 5, 13, 216, 11, 16, 241, 8, 0, 11, 15, 139, 38, 135,
            76, 129, 76, 248, 244, 5, 0, 12, 24, 242, 0, 1, 5, 13, 217, 8, 12, 240, 3, 1, 5, 13,
            250, 224, 4, 16, 250
        ]
    );
    assert_eq!(
        function.exception_table,
        [
            130, 1, 38, 1, 132, 11, 38, 1, 143, 1, 36, 4, 144, 5, 38, 1, 149, 9, 33, 3, 158, 2, 38,
            1, 160, 1, 33, 3, 161, 4, 38, 1
        ]
    );
}

#[test]
fn python312_match_failure_cleanup_keeps_original_zero_pop_boundary() {
    // Captured from the pinned CPython 3.12 compiler.
    for (source, expected_code, expected_lines, expected_exceptions) in [
        (
            "def f(value, action):\n    try:\n        match value:\n            case bytes() | bytearray() as raw:\n                action(raw)\n            case _:\n                return None\n        action()\n    except ValueError:\n        return None\n",
            &[
                151, 0, 9, 0, 124, 0, 120, 1, 120, 1, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 100, 1, 152,
                0, 120, 1, 129, 3, 92, 0, 0, 0, 110, 17, 1, 0, 120, 1, 116, 2, 0, 0, 0, 0, 0, 0, 0,
                0, 100, 1, 152, 0, 120, 1, 129, 3, 92, 0, 0, 0, 110, 3, 1, 0, 1, 0, 110, 11, 1, 0,
                125, 2, 2, 0, 124, 1, 124, 2, 171, 1, 0, 0, 0, 0, 0, 0, 1, 0, 110, 3, 1, 0, 9, 0,
                121, 0, 2, 0, 124, 1, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0, 35, 0, 116, 4, 0, 0,
                0, 0, 0, 0, 0, 0, 36, 0, 114, 3, 1, 0, 89, 0, 121, 0, 119, 0, 120, 3, 89, 0, 119,
                1,
            ][..],
            &[
                128, 0, 240, 2, 8, 5, 20, 216, 14, 19, 216, 17, 45, 144, 23, 148, 21, 151, 23, 152,
                59, 156, 57, 159, 59, 210, 17, 38, 208, 17, 45, 217, 16, 22, 144, 115, 149, 11,
                240, 3, 0, 18, 46, 224, 17, 18, 216, 23, 27, 217, 8, 14, 141, 8, 248, 220, 11, 21,
                242, 0, 1, 5, 20, 217, 15, 19, 240, 3, 1, 5, 20, 250,
            ][..],
            &[
                130, 45, 56, 0, 176, 7, 56, 0, 184, 9, 65, 4, 3, 193, 3, 1, 65, 4, 3,
            ][..],
        ),
        (
            "def f(value, action):\n    match value:\n        case bytes() | bytearray() as raw:\n            action(raw)\n        case _:\n            return None\n    action()\n",
            &[
                151, 0, 124, 0, 120, 1, 120, 1, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 100, 1, 152, 0,
                120, 1, 129, 3, 92, 0, 0, 0, 110, 17, 1, 0, 120, 1, 116, 2, 0, 0, 0, 0, 0, 0, 0, 0,
                100, 1, 152, 0, 120, 1, 129, 3, 92, 0, 0, 0, 110, 3, 1, 0, 1, 0, 110, 11, 1, 0,
                125, 2, 2, 0, 124, 1, 124, 2, 171, 1, 0, 0, 0, 0, 0, 0, 1, 0, 110, 3, 1, 0, 9, 0,
                121, 0, 2, 0, 124, 1, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0,
            ][..],
            &[
                128, 0, 216, 10, 15, 216, 13, 41, 136, 87, 140, 85, 143, 87, 144, 123, 148, 121,
                151, 123, 210, 13, 34, 208, 13, 41, 217, 12, 18, 144, 51, 141, 75, 240, 3, 0, 14,
                42, 224, 13, 14, 216, 19, 23, 217, 4, 10, 133, 72,
            ][..],
            &[][..],
        ),
    ] {
        let module = compile(source, PythonVersion::Py312);
        let function = child(&module, "f");
        assert_eq!(function.bytecode, expected_code, "{source}");
        assert_eq!(function.line_table, expected_lines, "{source}");
        assert_eq!(function.exception_table, expected_exceptions, "{source}");
    }
}

#[test]
fn python312_cold_handler_does_not_pre_locate_copied_finalizer_return() {
    let module = compile(
        "def f(flag, other, preserve, value):\n    try:\n        return value\n    finally:\n        try:\n            try:\n                try:\n                    work()\n                except Error:\n                    failed()\n                else:\n                    if flag:\n                        done()\n            except Error:\n                branch()\n        except Error:\n            if preserve:\n                return value\n            raise\n",
        PythonVersion::Py312,
    );
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        [
            151, 0, 9, 0, 124, 3, 9, 0, 9, 0, 9, 0, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0,
            0, 0, 0, 0, 1, 0, 124, 0, 114, 12, 116, 3, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0,
            0, 0, 1, 0, 83, 0, 83, 0, 83, 0, 35, 0, 116, 4, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 13,
            1, 0, 116, 7, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 89, 0, 140, 23,
            119, 0, 120, 3, 89, 0, 119, 1, 35, 0, 116, 4, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 13,
            1, 0, 116, 9, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 89, 0, 83, 0,
            119, 0, 120, 3, 89, 0, 119, 1, 35, 0, 116, 4, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 10,
            1, 0, 124, 2, 114, 6, 124, 3, 99, 2, 89, 0, 99, 2, 1, 0, 83, 0, 130, 0, 119, 0, 120, 3,
            89, 0, 119, 1, 35, 0, 9, 0, 9, 0, 9, 0, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0,
            0, 0, 0, 0, 1, 0, 124, 0, 114, 38, 116, 3, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0,
            0, 0, 1, 0, 110, 26, 35, 0, 116, 4, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 13, 1, 0, 116,
            7, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 89, 0, 110, 4, 119, 0, 120,
            3, 89, 0, 119, 1, 119, 0, 119, 0, 119, 0, 35, 0, 116, 4, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0,
            114, 13, 1, 0, 116, 9, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 89, 0,
            119, 0, 119, 0, 120, 3, 89, 0, 119, 1, 35, 0, 116, 4, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0,
            114, 12, 1, 0, 124, 2, 114, 8, 124, 3, 99, 2, 89, 0, 99, 2, 1, 0, 99, 2, 89, 0, 83, 0,
            130, 0, 119, 0, 120, 3, 89, 0, 119, 1, 120, 3, 89, 0, 119, 1
        ]
    );
    assert_eq!(
        function.line_table,
        [
            128, 0, 240, 2, 17, 5, 18, 216, 15, 20, 240, 4, 14, 9, 18, 240, 2, 9, 13, 25, 240, 2,
            6, 17, 31, 220, 20, 24, 148, 70, 241, 8, 0, 24, 28, 220, 24, 28, 157, 6, 232, 0, 240,
            3, 0, 24, 28, 248, 244, 7, 0, 24, 29, 242, 0, 1, 17, 29, 220, 20, 26, 150, 72, 240, 3,
            1, 17, 29, 251, 244, 10, 0, 20, 25, 242, 0, 1, 13, 25, 220, 16, 22, 150, 8, 240, 3, 1,
            13, 25, 251, 228, 15, 20, 242, 0, 3, 9, 18, 217, 15, 23, 216, 23, 28, 148, 12, 216, 12,
            17, 240, 7, 3, 9, 18, 251, 240, 23, 14, 9, 18, 240, 2, 9, 13, 25, 240, 2, 6, 17, 31,
            220, 20, 24, 148, 70, 241, 8, 0, 24, 28, 220, 24, 28, 157, 6, 248, 244, 9, 0, 24, 29,
            242, 0, 1, 17, 29, 220, 20, 26, 150, 72, 240, 3, 1, 17, 29, 250, 216, 20, 28, 240, 6,
            0, 25, 31, 240, 3, 0, 24, 28, 248, 228, 19, 24, 242, 0, 1, 13, 25, 220, 16, 22, 150, 8,
            240, 3, 1, 13, 25, 251, 228, 15, 20, 242, 0, 3, 9, 18, 217, 15, 23, 216, 23, 28, 150,
            12, 216, 12, 17, 240, 7, 3, 9, 18, 253
        ]
    );
    assert_eq!(
        function.exception_table,
        [
            130, 1, 65, 39, 0, 134, 10, 31, 2, 144, 12, 56, 2, 159, 19, 53, 5, 178, 2, 56, 2, 180,
            1, 53, 5, 181, 3, 56, 2, 184, 19, 65, 14, 5, 193, 11, 1, 65, 17, 2, 193, 13, 1, 65, 14,
            5, 193, 14, 3, 65, 17, 2, 193, 17, 13, 65, 36, 5, 193, 34, 2, 65, 36, 5, 193, 39, 1,
            67, 15, 3, 193, 43, 10, 66, 2, 4, 193, 53, 13, 66, 30, 4, 194, 2, 19, 66, 24, 7, 194,
            21, 2, 66, 30, 4, 194, 23, 1, 66, 24, 7, 194, 24, 3, 66, 30, 4, 194, 27, 3, 67, 15, 3,
            194, 30, 19, 66, 52, 7, 194, 49, 1, 66, 55, 4, 194, 50, 1, 67, 15, 3, 194, 51, 1, 66,
            52, 7, 194, 52, 3, 66, 55, 4, 194, 55, 13, 67, 12, 7, 195, 4, 4, 67, 15, 3, 195, 10, 2,
            67, 12, 7, 195, 12, 3, 67, 15, 3
        ]
    );
}

#[test]
fn python312_unpacked_for_continue_keeps_original_native_jump_seed() {
    // Captured from CPython 3.12.14. The source bound is 273, so this exercises
    // exact source labels: the cold await bridge needs 255, rather than 256.
    let mut source = String::from("async def f(items, flag, work):\n");
    source.push_str(&"    if flag:\n        value = 0\n".repeat(60));
    source.push_str("    for first, second in items:\n        if flag:\n            continue\n        use(first, second)\n    await work()\n");
    source.push_str(&"    value = 0\n".repeat(125));
    source.push_str("    return None\n");
    let module = compile(&source, PythonVersion::Py312);
    let function = child(&module, "f");
    let mut bytecode = vec![75, 0, 1, 0, 151, 0];
    bytecode.extend([124, 1, 114, 2, 100, 1, 125, 3].repeat(60));
    bytecode.extend([
        124, 0, 68, 0, 93, 20, 0, 0, 92, 2, 0, 0, 125, 4, 125, 5, 124, 1, 114, 1, 140, 9, 116, 1,
        0, 0, 0, 0, 0, 0, 0, 0, 124, 4, 124, 5, 171, 2, 0, 0, 0, 0, 0, 0, 1, 0, 140, 22, 4, 0, 2,
        0, 124, 2, 171, 0, 0, 0, 0, 0, 0, 0, 131, 0, 100, 0, 123, 3, 0, 0, 150, 2, 151, 3, 134, 5,
        5, 0, 1, 0,
    ]);
    bytecode.extend([100, 1, 125, 3].repeat(125));
    bytecode.extend([121, 0, 55, 0, 140, 255, 173, 3, 119, 1]);
    assert_eq!(function.bytecode, bytecode);
    assert_eq!(
        function.exception_table,
        [
            130, 68, 20, 72, 24, 1, 196, 22, 1, 72, 22, 4, 196, 23, 68, 0, 72, 24, 1,
        ]
    );
}

#[test]
fn legacy_terminal_elif_declaration_else_uses_the_last_test_span() {
    // Native 3.12.14 and 3.13.15 from the unchanged pinned corpus-oracle
    // protocol. A declaration-only else emits no instruction; the implicit
    // shared footer inherits the final elif test, not the Global span.
    let source = "def f(a, b):\n    if a:\n        return\n    elif b:\n        return\n    else:\n        global shared\n";
    for (version, bytecode, lines) in [
        (
            PythonVersion::Py312,
            &[
                151, 0, 124, 0, 114, 1, 121, 0, 124, 1, 114, 1, 121, 0, 121, 0,
            ][..],
            &[
                128, 0, 217, 7, 8, 216, 8, 14, 217, 9, 10, 216, 8, 14, 240, 3, 0, 10, 11,
            ][..],
        ),
        (
            PythonVersion::Py313,
            &[
                149, 0, 85, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 1, 0, 0, 103, 0, 85, 1, 40, 0, 0, 0, 0,
                0, 0, 0, 97, 1, 0, 0, 103, 0, 103, 0,
            ][..],
            &[
                128, 0, 222, 7, 8, 216, 8, 14, 222, 9, 10, 216, 8, 14, 240, 3, 0, 10, 11,
            ][..],
        ),
    ] {
        let module = compile(source, version);
        let function = child(&module, "f");
        assert_eq!(function.bytecode, bytecode, "{version:?}");
        assert_eq!(function.line_table, lines, "{version:?}");
        assert!(function.exception_table.is_empty(), "{version:?}");
    }
}

#[test]
fn python312_labeled_successor_bound_preserves_one_byte_loop_backedge() {
    // Captured from CPython 3.12.14. Try/While decline the exact-label plan.
    // Counting labeled helper successors gives 254 instead of 282, allowing the
    // native seed certificate and the loop's final backedge of 255 code units.
    let mut source = String::from("async def f(flag):\n");
    source.push_str(&"    padding = 0\n".repeat(150));
    source.push_str("    try:\n        work()\n    except:\n        pass\n");
    source.push_str(&"    await action()\n".repeat(28));
    source.push_str("    while flag:\n");
    source.push_str(&"        padding = 0\n".repeat(126));
    let module = compile(&source, PythonVersion::Py312);
    let function = child(&module, "f");
    let mut bytecode = vec![75, 0, 1, 0, 151, 0];
    bytecode.extend([100, 1, 125, 1].repeat(150));
    bytecode.extend([
        9, 0, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0,
    ]);
    bytecode.extend(
        [
            116, 3, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 131, 0, 100, 0, 123, 3, 0, 0,
            150, 2, 151, 3, 134, 5, 5, 0, 1, 0,
        ]
        .repeat(28),
    );
    bytecode.extend([124, 0, 144, 1, 114, 0]);
    bytecode.extend([100, 1, 125, 1].repeat(126));
    bytecode.extend([
        124, 0, 114, 1, 140, 255, 121, 0, 121, 0, 35, 0, 1, 0, 89, 0, 144, 3, 140, 1, 120, 3, 89,
        0, 119, 1, 55, 0, 144, 2, 140, 247, 55, 0, 144, 2, 140, 232, 55, 0, 144, 2, 140, 217, 55,
        0, 144, 2, 140, 202, 55, 0, 144, 2, 140, 187, 55, 0, 144, 2, 140, 172, 55, 0, 144, 2, 140,
        157, 55, 0, 144, 2, 140, 142, 55, 0, 144, 2, 140, 127, 55, 0, 144, 2, 140, 112, 55, 0, 144,
        2, 140, 97, 55, 0, 144, 2, 140, 82, 55, 0, 144, 2, 140, 67, 55, 0, 144, 2, 140, 52, 55, 0,
        144, 2, 140, 37, 55, 0, 144, 2, 140, 22, 55, 0, 144, 2, 140, 7, 55, 0, 144, 1, 140, 248,
        55, 0, 144, 1, 140, 233, 55, 0, 144, 1, 140, 218, 55, 0, 144, 1, 140, 203, 55, 0, 144, 1,
        140, 188, 55, 0, 144, 1, 140, 173, 55, 0, 144, 1, 140, 158, 55, 0, 144, 1, 140, 143, 55, 0,
        144, 1, 140, 128, 55, 0, 144, 1, 140, 113, 55, 0, 144, 1, 140, 98, 173, 3, 119, 1,
    ]);
    assert_eq!(function.bytecode, bytecode);
    assert_eq!(
        function.exception_table,
        [
            130, 68, 45, 82, 18, 1, 196, 48, 10, 80, 54, 0, 196, 58, 13, 82, 18, 1, 197, 7, 1, 80,
            62, 4, 197, 8, 17, 82, 18, 1, 197, 25, 1, 81, 1, 4, 197, 26, 17, 82, 18, 1, 197, 43, 1,
            81, 4, 4, 197, 44, 17, 82, 18, 1, 197, 61, 1, 81, 7, 4, 197, 62, 17, 82, 18, 1, 198,
            15, 1, 81, 10, 4, 198, 16, 17, 82, 18, 1, 198, 33, 1, 81, 13, 4, 198, 34, 17, 82, 18,
            1, 198, 51, 1, 81, 16, 4, 198, 52, 17, 82, 18, 1, 199, 5, 1, 81, 19, 4, 199, 6, 17, 82,
            18, 1, 199, 23, 1, 81, 22, 4, 199, 24, 17, 82, 18, 1, 199, 41, 1, 81, 25, 4, 199, 42,
            17, 82, 18, 1, 199, 59, 1, 81, 28, 4, 199, 60, 17, 82, 18, 1, 200, 13, 1, 81, 31, 4,
            200, 14, 17, 82, 18, 1, 200, 31, 1, 81, 34, 4, 200, 32, 17, 82, 18, 1, 200, 49, 1, 81,
            37, 4, 200, 50, 17, 82, 18, 1, 201, 3, 1, 81, 40, 4, 201, 4, 17, 82, 18, 1, 201, 21, 1,
            81, 43, 4, 201, 22, 17, 82, 18, 1, 201, 39, 1, 81, 46, 4, 201, 40, 17, 82, 18, 1, 201,
            57, 1, 81, 49, 4, 201, 58, 17, 82, 18, 1, 202, 11, 1, 81, 52, 4, 202, 12, 17, 82, 18,
            1, 202, 29, 1, 81, 55, 4, 202, 30, 17, 82, 18, 1, 202, 47, 1, 81, 58, 4, 202, 48, 17,
            82, 18, 1, 203, 1, 1, 81, 61, 4, 203, 2, 17, 82, 18, 1, 203, 19, 1, 82, 0, 4, 203, 20,
            17, 82, 18, 1, 203, 37, 1, 82, 3, 4, 203, 38, 17, 82, 18, 1, 203, 55, 1, 82, 6, 4, 203,
            56, 17, 82, 18, 1, 204, 9, 1, 82, 9, 4, 204, 10, 17, 82, 18, 1, 204, 27, 1, 82, 12, 4,
            204, 28, 17, 82, 18, 1, 204, 45, 1, 82, 15, 4, 204, 46, 68, 5, 82, 18, 1, 208, 52, 2,
            82, 18, 1, 208, 54, 2, 80, 59, 3, 208, 56, 7, 82, 18, 1, 209, 1, 1, 82, 18, 1, 209, 4,
            1, 82, 18, 1, 209, 7, 1, 82, 18, 1, 209, 10, 1, 82, 18, 1, 209, 13, 1, 82, 18, 1, 209,
            16, 1, 82, 18, 1, 209, 19, 1, 82, 18, 1, 209, 22, 1, 82, 18, 1, 209, 25, 1, 82, 18, 1,
            209, 28, 1, 82, 18, 1, 209, 31, 1, 82, 18, 1, 209, 34, 1, 82, 18, 1, 209, 37, 1, 82,
            18, 1, 209, 40, 1, 82, 18, 1, 209, 43, 1, 82, 18, 1, 209, 46, 1, 82, 18, 1, 209, 49, 1,
            82, 18, 1, 209, 52, 1, 82, 18, 1, 209, 55, 1, 82, 18, 1, 209, 58, 1, 82, 18, 1, 209,
            61, 1, 82, 18, 1, 210, 0, 1, 82, 18, 1, 210, 3, 1, 82, 18, 1, 210, 6, 1, 82, 18, 1,
            210, 9, 1, 82, 18, 1, 210, 12, 1, 82, 18, 1, 210, 15, 1, 82, 18, 1,
        ]
    );
}

#[test]
fn legacy_nested_terminating_finalizers_preserve_pending_returns() {
    // Captured from pinned CPython 3.12.14 / 3.13.15 with the unchanged
    // source-bytes and initial-intern-state corpus oracle protocol.
    let source = "def f():\n    try:\n        try:\n            return payload\n        finally:\n            return inner_result\n    finally:\n        return outer_result\n";
    for (version, bytecode, lines, exceptions) in [
        (
            PythonVersion::Py312,
            &[
                151, 0, 9, 0, 9, 0, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 116, 2, 0, 0, 0, 0, 0, 0, 0, 0,
                99, 2, 1, 0, 116, 4, 0, 0, 0, 0, 0, 0, 0, 0, 99, 2, 1, 0, 83, 0, 35, 0, 116, 2, 0,
                0, 0, 0, 0, 0, 0, 0, 99, 2, 1, 0, 99, 2, 89, 0, 116, 4, 0, 0, 0, 0, 0, 0, 0, 0, 99,
                2, 1, 0, 83, 0, 120, 3, 89, 0, 119, 1, 35, 0, 116, 4, 0, 0, 0, 0, 0, 0, 0, 0, 99,
                2, 1, 0, 99, 2, 89, 0, 83, 0, 120, 3, 89, 0, 119, 1,
            ][..],
            &[
                128, 0, 240, 2, 6, 5, 28, 240, 2, 3, 9, 32, 220, 19, 26, 228, 19, 31, 209, 12, 31,
                228, 15, 27, 210, 8, 27, 248, 244, 5, 0, 20, 32, 211, 12, 31, 228, 15, 27, 210, 8,
                27, 251, 140, 124, 212, 8, 27, 250,
            ][..],
            &[
                131, 5, 23, 0, 136, 7, 44, 0, 151, 9, 41, 3, 160, 1, 44, 0, 169, 3, 44, 0, 172, 9,
                55, 3,
            ][..],
        ),
        (
            PythonVersion::Py313,
            &[
                149, 0, 30, 0, 30, 0, 91, 0, 0, 0, 0, 0, 0, 0, 0, 0, 91, 2, 0, 0, 0, 0, 0, 0, 0, 0,
                115, 2, 32, 0, 91, 4, 0, 0, 0, 0, 0, 0, 0, 0, 115, 2, 32, 0, 36, 0, 33, 0, 91, 2,
                0, 0, 0, 0, 0, 0, 0, 0, 115, 2, 32, 0, 115, 2, 31, 0, 91, 4, 0, 0, 0, 0, 0, 0, 0,
                0, 115, 2, 32, 0, 36, 0, 61, 3, 31, 0, 102, 1, 33, 0, 91, 4, 0, 0, 0, 0, 0, 0, 0,
                0, 115, 2, 32, 0, 115, 2, 31, 0, 36, 0, 61, 3, 31, 0, 102, 1,
            ][..],
            &[
                128, 0, 240, 2, 6, 5, 28, 240, 2, 3, 9, 32, 220, 19, 26, 228, 19, 31, 209, 12, 31,
                228, 15, 27, 210, 8, 27, 248, 244, 5, 0, 20, 32, 211, 12, 31, 228, 15, 27, 210, 8,
                27, 251, 140, 124, 212, 8, 27, 250,
            ][..],
            &[
                131, 5, 23, 0, 136, 7, 44, 0, 151, 9, 41, 3, 160, 1, 44, 0, 169, 3, 44, 0, 172, 9,
                55, 3,
            ][..],
        ),
    ] {
        let module = compile(source, version);
        let function = child(&module, "f");
        assert_eq!(function.bytecode, bytecode, "{version:?}");
        assert_eq!(function.line_table, lines, "{version:?}");
        assert_eq!(function.exception_table, exceptions, "{version:?}");
        assert_eq!(function.stack_size, 4, "{version:?}");
    }
}

#[test]
fn legacy_exceptional_inner_return_runs_the_outer_finalizer() {
    // Captured from pinned CPython 3.12.14 / 3.13.15 with the unchanged
    // source-bytes and initial-intern-state corpus oracle protocol.
    let source = "def f():\n    try:\n        try:\n            return payload\n        finally:\n            return inner_result\n    finally:\n        cleanup()\n        return outer_result\n";
    for (version, bytecode, lines, exceptions) in [
        (
            PythonVersion::Py312,
            &[
                151, 0, 9, 0, 9, 0, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 116, 2, 0, 0, 0, 0, 0, 0, 0, 0,
                99, 2, 1, 0, 116, 5, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 116,
                6, 0, 0, 0, 0, 0, 0, 0, 0, 99, 2, 1, 0, 83, 0, 35, 0, 116, 2, 0, 0, 0, 0, 0, 0, 0,
                0, 99, 2, 1, 0, 99, 2, 89, 0, 116, 5, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0,
                0, 0, 1, 0, 116, 6, 0, 0, 0, 0, 0, 0, 0, 0, 99, 2, 1, 0, 83, 0, 120, 3, 89, 0, 119,
                1, 35, 0, 116, 5, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 116, 6,
                0, 0, 0, 0, 0, 0, 0, 0, 99, 2, 1, 0, 99, 2, 89, 0, 83, 0, 120, 3, 89, 0, 119, 1,
            ][..],
            &[
                128, 0, 240, 2, 7, 5, 28, 240, 2, 3, 9, 32, 220, 19, 26, 228, 19, 31, 209, 12, 31,
                228, 8, 15, 140, 9, 220, 15, 27, 210, 8, 27, 248, 244, 7, 0, 20, 32, 211, 12, 31,
                228, 8, 15, 140, 9, 220, 15, 27, 210, 8, 27, 251, 244, 3, 0, 9, 16, 140, 9, 220,
                15, 27, 212, 8, 27, 250,
            ][..],
            &[
                131, 5, 33, 0, 136, 7, 65, 0, 0, 161, 9, 61, 3, 170, 1, 65, 0, 0, 189, 3, 65, 0, 0,
                193, 0, 19, 65, 21, 3,
            ][..],
        ),
        (
            PythonVersion::Py313,
            &[
                149, 0, 30, 0, 30, 0, 91, 0, 0, 0, 0, 0, 0, 0, 0, 0, 91, 2, 0, 0, 0, 0, 0, 0, 0, 0,
                115, 2, 32, 0, 91, 5, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 91,
                6, 0, 0, 0, 0, 0, 0, 0, 0, 115, 2, 32, 0, 36, 0, 33, 0, 91, 2, 0, 0, 0, 0, 0, 0, 0,
                0, 115, 2, 32, 0, 115, 2, 31, 0, 91, 5, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0,
                0, 0, 32, 0, 91, 6, 0, 0, 0, 0, 0, 0, 0, 0, 115, 2, 32, 0, 36, 0, 61, 3, 31, 0,
                102, 1, 33, 0, 91, 5, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 91,
                6, 0, 0, 0, 0, 0, 0, 0, 0, 115, 2, 32, 0, 115, 2, 31, 0, 36, 0, 61, 3, 31, 0, 102,
                1,
            ][..],
            &[
                128, 0, 240, 2, 7, 5, 28, 240, 2, 3, 9, 32, 220, 19, 26, 228, 19, 31, 209, 12, 31,
                228, 8, 15, 140, 9, 220, 15, 27, 210, 8, 27, 248, 244, 7, 0, 20, 32, 211, 12, 31,
                228, 8, 15, 140, 9, 220, 15, 27, 210, 8, 27, 251, 244, 3, 0, 9, 16, 140, 9, 220,
                15, 27, 212, 8, 27, 250,
            ][..],
            &[
                131, 5, 33, 0, 136, 7, 65, 0, 0, 161, 9, 61, 3, 170, 1, 65, 0, 0, 189, 3, 65, 0, 0,
                193, 0, 19, 65, 21, 3,
            ][..],
        ),
    ] {
        let module = compile(source, version);
        let function = child(&module, "f");
        assert_eq!(function.bytecode, bytecode, "{version:?}");
        assert_eq!(function.line_table, lines, "{version:?}");
        assert_eq!(function.exception_table, exceptions, "{version:?}");
        assert_eq!(function.stack_size, 4, "{version:?}");
    }
}

#[test]
fn python312_for_break_keeps_original_native_jump_seed() {
    // Captured from CPython 3.12.14. The source bound is 271, so this exercises
    // exact source labels at the cold await bridge's one-byte width boundary.
    let mut source = String::from("async def f(items, flag, work):\n");
    source.push_str(&"    if flag:\n        value = 0\n".repeat(60));
    source.push_str("    for item in items:\n        if flag:\n            break\n        use(item)\n    await work()\n");
    source.push_str(&"    value = 0\n".repeat(125));
    source.push_str("    return None\n");
    let module = compile(&source, PythonVersion::Py312);
    let function = child(&module, "f");
    let mut bytecode = vec![75, 0, 1, 0, 151, 0];
    bytecode.extend([124, 1, 114, 2, 100, 1, 125, 3].repeat(60));
    bytecode.extend([
        124, 0, 68, 0, 93, 17, 0, 0, 125, 4, 124, 1, 114, 2, 1, 0, 110, 13, 116, 1, 0, 0, 0, 0, 0,
        0, 0, 0, 124, 4, 171, 1, 0, 0, 0, 0, 0, 0, 1, 0, 140, 19, 4, 0, 2, 0, 124, 2, 171, 0, 0, 0,
        0, 0, 0, 0, 131, 0, 100, 0, 123, 3, 0, 0, 150, 2, 151, 3, 134, 5, 5, 0, 1, 0,
    ]);
    bytecode.extend([100, 1, 125, 3].repeat(125));
    bytecode.extend([121, 0, 55, 0, 140, 255, 173, 3, 119, 1]);
    assert_eq!(function.bytecode, bytecode);
    assert_eq!(
        function.exception_table,
        [
            130, 68, 17, 72, 21, 1, 196, 19, 1, 72, 19, 4, 196, 20, 68, 0, 72, 21, 1,
        ]
    );
}

#[test]
fn python313_manager_pop_stays_unlocated_until_infinite_loop_folding() {
    // Captured from pinned CPython 3.13.15 using the source-bytes oracle.
    // The loop's original false edge keeps this manager POP_BLOCK shared
    // during line propagation, before constant folding removes that edge.
    let source = "def f(manager, flag):\n    with manager:\n        if flag:\n            while True:\n                work()\n        else:\n            finish()\n";
    let module = compile(source, PythonVersion::Py313);
    let function = child(&module, "f");
    assert_eq!(
        function.bytecode,
        [
            149, 0, 85, 0, 2, 0, 32, 0, 85, 1, 40, 0, 0, 0, 0, 0, 0, 0, 97, 13, 0, 0, 30, 0, 91, 1,
            0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 77, 12, 0, 0, 91, 3, 0, 0, 0,
            0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 83, 0, 83, 0, 83, 0, 53, 2, 0, 0, 0, 0,
            0, 0, 32, 0, 103, 0, 33, 0, 44, 0, 40, 0, 0, 0, 0, 0, 0, 0, 100, 1, 0, 0, 102, 2, 32,
            0, 31, 0, 32, 0, 32, 0, 103, 0, 61, 3, 31, 0, 102, 1,
        ]
    );
    assert_eq!(
        function.line_table,
        [
            128, 0, 218, 9, 16, 222, 11, 15, 216, 18, 22, 220, 16, 20, 148, 6, 241, 3, 0, 19, 23,
            244, 6, 0, 13, 19, 140, 72, 247, 11, 0, 10, 17, 143, 23, 142, 23, 250,
        ]
    );
    assert_eq!(function.exception_table, [131, 31, 43, 3, 171, 10, 57, 7,]);
    assert_eq!(function.arg_count, 2);
    assert_eq!(function.positional_only_arg_count, 0);
    assert_eq!(function.keyword_only_arg_count, 0);
    assert_eq!(function.stack_size, 6);
    assert_eq!(function.flags, 3);
    assert_eq!(function.names, ["work", "finish"]);
    assert_eq!(function.locals, ["manager", "flag"]);
    assert_eq!(
        function.local_kinds,
        [crate::target::local_kinds::CO_FAST_LOCAL; 2]
    );
    assert_eq!(function.first_line_number, 1);
    assert_eq!(function.name, "f");
    assert_eq!(function.qualified_name, "f");
    assert!(matches!(function.constants.as_slice(), [Constant::None]));
}

#[test]
fn annotation_class_cells_follow_target_evaluation_rules() {
    use crate::target::local_kinds::CO_FAST_CELL;

    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for future in ["", "from __future__ import annotations\n"] {
            for definition in [
                "field: __class__",
                "def method(arg: __class__): pass",
                "type Alias = __class__",
                "type Alias = [__class__ for __class__ in ()]",
            ] {
                let source = format!("{future}class C:\n    {definition}\n");
                if version == PythonVersion::Py312
                    && definition == "type Alias = [__class__ for __class__ in ()]"
                {
                    let error = compile_with_options(
                        &source,
                        "legacy_annotations.py",
                        &CompileOptions {
                            python_version: version,
                            ..CompileOptions::default()
                        },
                    )
                    .unwrap_err();
                    assert!(
                        error.to_string().contains(
                            "Cannot use comprehension in annotation scope within class scope"
                        ),
                        "{error}"
                    );
                    continue;
                }
                let module = compile(&source, version);
                let class = child(&module, "C");
                let has_class_cell = class
                    .locals
                    .iter()
                    .zip(&class.local_kinds)
                    .any(|(name, kind)| name == "__class__" && kind & CO_FAST_CELL != 0);
                let expected = definition == "type Alias = __class__"
                    || !definition.starts_with("type ")
                        && version >= PythonVersion::Py314
                        && future.is_empty();
                assert_eq!(
                    has_class_cell, expected,
                    "{version:?}: {future}{definition}"
                );
            }
        }
    }
}
