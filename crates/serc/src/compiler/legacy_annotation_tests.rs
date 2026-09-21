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
