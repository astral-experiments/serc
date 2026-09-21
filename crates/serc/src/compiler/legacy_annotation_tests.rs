use crate::code_object::{CodeObject, Constant};
use crate::target::code_flags::{CO_HAS_DOCSTRING, CO_METHOD};
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
