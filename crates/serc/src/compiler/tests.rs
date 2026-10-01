use ruff_python_ast::token::TokenKind;
use ruff_python_ast::{Expr, ModModule, Stmt, StringLike};
use ruff_python_parser::{Parsed, parse_module};
use ruff_text_size::{Ranged, TextRange, TextSize};

use crate::assembler::SourceLocation;
use crate::code_object::{CodeObject, Constant};
use crate::target::code_flags::{CO_FUTURE_ANNOTATIONS, CO_FUTURE_BARRY_AS_BDFL};
use crate::target::opcodes::{
    BUILD_TUPLE, GET_LEN, LOAD_CONST, LOAD_SMALL_INT, NOP, NOT_TAKEN, RAISE_VARARGS, RERAISE,
    RETURN_VALUE,
};

use super::definition::clean_doc;
use super::scope::{FunctionPlan, NameId, NameInterner, ScopeBodyFacts, ScopeId};
use super::source::{
    debug_text_range, interpolated_string_content_range, strip_expression_comments,
    unparse_annotation,
};
use super::statement::{
    NestedIfExitLocation, first_coroutine_iterator_assignment_condition, nested_if_exit_location,
    try_propagates_protected_loop_tail_mode,
};
use super::symbol::{
    CompilerStackEffect, ReraiseStackState, compiler_stack_effect, validate_compiler_stack_effect,
};
use super::{Compiler, SHADOWED_SUPER_SENTINEL};

#[test]
fn optional_compiler_ids_remain_four_bytes() {
    assert_eq!(std::mem::size_of::<Option<NameId>>(), 4);
    assert_eq!(std::mem::size_of::<Option<ScopeId>>(), 4);
}

#[test]
fn compiled_modules_share_their_filename_and_remain_send_sync() {
    use crate::code_object::{CodeInterningSummary, CompilationInterningEvent};

    fn assert_send_sync<T: Send + Sync>() {}

    fn child<'a>(code: &'a CodeObject, name: &str) -> &'a CodeObject {
        code.constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(child) if child.name == name => Some(child.as_ref()),
                _ => None,
            })
            .expect("nested code should exist")
    }

    assert_send_sync::<crate::CompiledModule>();
    let source = "class Holder:\n    def method(self):\n        self.__hidden = 1\n        def nested():\n            return 1\n        return nested\n";
    let filename = "directory/nested/filename_é.py";
    for python_version in [
        crate::PythonVersion::Py312,
        crate::PythonVersion::Py313,
        crate::PythonVersion::Py314,
        crate::PythonVersion::Py315,
    ] {
        let module = crate::compile_with_options(
            source,
            filename,
            &crate::CompileOptions {
                python_version,
                ..crate::CompileOptions::default()
            },
        )
        .unwrap();
        let root = &module.code;
        let class = child(root, "Holder");
        let method = child(class, "method");
        let nested = child(method, "nested");
        assert_eq!(root.filename.as_utf8(), Some(filename));
        for code in [class, method, nested] {
            assert!(root.filename.same_allocation(&code.filename));
        }
        let clone = module.clone();
        assert!(root.filename.same_allocation(&clone.code.filename));
        let summary = CodeInterningSummary::from_code(nested);
        assert!(root.filename.same_allocation(&summary.filename));

        let mut summaries = 0;
        for event in &root.marshal_provenance.interning_events {
            if let CompilationInterningEvent::CodeAssembly(summary) = event {
                assert!(root.filename.same_allocation(&summary.filename));
                summaries += 1;
            }
        }
        assert!(summaries >= 4, "{python_version:?}: {summaries}");
    }
}

#[test]
fn generator_nested_comprehension_captures_keep_scope_roles_for_all_targets() {
    use crate::target::local_kinds::{CO_FAST_CELL, CO_FAST_FREE, CO_FAST_LOCAL};

    let source = concat!(
        include_str!("../../tests/fixtures/generator_nested_comprehension_captures.py"),
        "\n",
        include_str!("../../tests/fixtures/generator_nested_comprehension_cell_controls.py"),
    );
    for python_version in [
        crate::PythonVersion::Py312,
        crate::PythonVersion::Py313,
        crate::PythonVersion::Py314,
        crate::PythonVersion::Py315,
    ] {
        let module = crate::compile_with_options(
            source,
            "generator_nested_comprehension_captures.py",
            &crate::CompileOptions {
                python_version,
                ..crate::CompileOptions::default()
            },
        )
        .unwrap();
        for constant in &module.code.constants {
            let Constant::Code(function) = constant else {
                continue;
            };
            let generator = function
                .constants
                .iter()
                .find_map(|constant| match constant {
                    Constant::Code(code) if code.name == "<genexpr>" => Some(code),
                    _ => None,
                })
                .unwrap();
            let has_kind = |code: &CodeObject, kind| {
                code.locals
                    .iter()
                    .zip(&code.local_kinds)
                    .any(|(name, kinds)| name == "x" && kinds & kind != 0)
            };
            let captures_outer = !matches!(
                function.name.as_str(),
                "cell_control" | "local_control" | "outer_iterable_control" | "outside_lambda"
            );
            assert_eq!(
                has_kind(function, CO_FAST_CELL),
                captures_outer,
                "{python_version:?}: {} outer cell",
                function.name,
            );
            assert_eq!(
                has_kind(generator, CO_FAST_FREE),
                captures_outer,
                "{python_version:?}: {} generator capture",
                function.name,
            );
            assert_eq!(
                has_kind(generator, CO_FAST_CELL),
                matches!(
                    function.name.as_str(),
                    "cell_control" | "inlined_cell" | "outside_lambda"
                ),
                "{python_version:?}: {} generator cell",
                function.name,
            );
            if captures_outer && function.name != "renamed_control" {
                assert!(
                    has_kind(generator, CO_FAST_LOCAL),
                    "{python_version:?}: {} comprehension local",
                    function.name
                );
            }
        }
    }
}

#[test]
fn compiler_stack_effects_distinguish_values_from_scope_exit_bookkeeping() {
    assert_eq!(
        compiler_stack_effect(LOAD_CONST, 0),
        CompilerStackEffect::Fixed(1)
    );
    assert_eq!(
        compiler_stack_effect(BUILD_TUPLE, 3),
        CompilerStackEffect::Fixed(-2)
    );
    assert_eq!(
        compiler_stack_effect(RETURN_VALUE, 0),
        CompilerStackEffect::Fixed(-1)
    );
    assert_eq!(
        compiler_stack_effect(RAISE_VARARGS, 1),
        CompilerStackEffect::Fixed(-1)
    );
    assert_eq!(
        compiler_stack_effect(RERAISE, 1),
        CompilerStackEffect::ContextDependent
    );
}

#[test]
fn protected_loop_tail_mode_preserves_supported_handlers() {
    let cases = [
        (
            "def f():\n    try:\n        action()\n    except FirstError:\n        raise\n    except SecondError as error:\n        message = 'details'\n        raise WrappedError(message) from error\n",
            true,
        ),
        (
            "def f():\n    try:\n        action()\n    except Error:\n        log()\n        raise\n",
            false,
        ),
        (
            "def f():\n    try:\n        action()\n    except Error:\n        return\n",
            false,
        ),
        (
            "def f():\n    try:\n        action()\n    except Error:\n        recover()\n",
            true,
        ),
        (
            "def f():\n    try:\n        action()\n    except Error:\n        if retry():\n            recover()\n",
            false,
        ),
    ];

    for (source, expected) in cases {
        let parsed = parse_module(source).unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!("expected one function definition");
        };
        let [Stmt::Try(statement)] = function.body.as_slice() else {
            panic!("expected one try statement");
        };
        assert_eq!(
            try_propagates_protected_loop_tail_mode(statement),
            expected,
            "unexpected protected loop-tail handler classification for:\n{source}"
        );
    }
}

#[test]
fn nested_final_else_propagates_an_unlocated_exit() {
    let source = r"if mask is None:
    if not keep:
        mask = False
    elif isinstance(data, tuple):
        mask = data
    else:
        if copy:
            data.mask = mask
            if getmask(data):
                if data.mask.shape != data.shape:
                    data.mask.shape = data.shape
else:
    mask = True
";
    let parsed = parse_module(source).unwrap();
    let [Stmt::If(outer)] = parsed.suite().as_slice() else {
        panic!("expected an outer if statement");
    };
    let Some(Stmt::If(nested)) = outer.body.last() else {
        panic!("expected a nested if statement");
    };
    assert!(matches!(
        nested_if_exit_location(nested, true),
        Some(NestedIfExitLocation::Unlocated)
    ));
}

#[test]
fn reraise_stack_states_cover_every_supported_logical_shape() {
    assert_eq!(
        [
            ReraiseStackState::ExceptionOnly.argument_and_effect(),
            ReraiseStackState::ActiveExceptionHandler.argument_and_effect(),
            ReraiseStackState::StopIterationHandler.argument_and_effect(),
            ReraiseStackState::HandlerCleanup.argument_and_effect(),
            ReraiseStackState::WithHandler.argument_and_effect(),
        ],
        [(0, -1), (0, -2), (1, -1), (1, -3), (2, -5)]
    );
}

#[test]
fn generic_stack_effect_validation_rejects_context_dependent_reraise() {
    for (argument, effect) in [(0, -1), (1, -3), (2, -5), (0, -3), (3, -1)] {
        let error = validate_compiler_stack_effect(RERAISE, argument, effect).unwrap_err();
        assert!(
            matches!(error, crate::CompileError::Internal(message) if message.contains("context-dependent opcode") && message.contains("typed emitter"))
        );
    }
}

#[test]
fn compiler_stack_effect_validation_rejects_stale_lowering_metadata() {
    let error = validate_compiler_stack_effect(LOAD_CONST, 0, 0).unwrap_err();
    assert!(
        matches!(error, crate::CompileError::Internal(message) if message.contains("declared stack effect 0") && message.contains("expected 1"))
    );
}

fn module_compiler<'source>(
    source: &'source str,
    parsed: &'source Parsed<ModModule>,
) -> Box<Compiler<'source>> {
    Compiler::module("example.py", source, parsed.tokens(), parsed.suite())
}

#[test]
fn opcode_locations_preserve_multiline_attribute_and_call_ranges() {
    let source =
        "value.method(arg)\n(value\n .method)(arg)\n(imported\n .method)(arg)\n(value\n .field)\n";
    let parsed = parse_module(source).unwrap();
    let mut compiler = module_compiler(source, &parsed);
    let imported = compiler.required_name_id("imported");
    compiler.symbols.imported_scope_names.insert(imported);

    let [
        Stmt::Expr(same_line),
        Stmt::Expr(split_method),
        Stmt::Expr(split_import),
        Stmt::Expr(split_attribute),
    ] = parsed.suite().as_slice()
    else {
        panic!("expected four expression statements");
    };
    let Expr::Call(same_line) = same_line.value.as_ref() else {
        panic!("expected a same-line call");
    };
    let Expr::Call(split_method) = split_method.value.as_ref() else {
        panic!("expected a split method call");
    };
    let Expr::Call(split_import) = split_import.value.as_ref() else {
        panic!("expected a split imported call");
    };
    let Expr::Attribute(split_attribute) = split_attribute.value.as_ref() else {
        panic!("expected a split attribute");
    };
    let Expr::Attribute(split_method_attribute) = split_method.func.as_ref() else {
        panic!("expected a method attribute");
    };

    assert_eq!(
        compiler.call_opcode_location(same_line),
        compiler.source_location(same_line.range())
    );
    assert_eq!(
        compiler.call_opcode_location(split_method),
        compiler.source_location(TextRange::new(
            split_method_attribute.attr.range().start(),
            split_method.range().end(),
        ))
    );
    assert_eq!(
        compiler.call_opcode_location(split_import),
        compiler.source_location(split_import.range())
    );
    assert_eq!(
        compiler.attribute_opcode_location(split_attribute),
        compiler.source_location(split_attribute.attr.range())
    );
}

fn read_exception_varint(table: &[u8], index: &mut usize) -> u32 {
    let mut byte = table[*index];
    *index += 1;
    let mut value = u32::from(byte & 0x3f);
    while byte & 0x40 != 0 {
        byte = table[*index];
        *index += 1;
        value = (value << 6) | u32::from(byte & 0x3f);
    }
    value
}

#[test]
fn generator_call_condition_owners_preserve_original_while_blocks() {
    let source = r#"def debug_callable_control():
    while (__debug__)():
        yield 1


def optimized_generator_controls(items):
    while all(item for item in items):
        yield items
    while any(item for item in items):
        yield items
    while tuple(item for item in items):
        yield items


def unary_not_control(item):
    while not should_continue(item):
        yield item


def nested_non_tail_control(items, observe):
    for item in items:
        while should_continue(item):
            yield item
        observe(item)


def protected_try_control(item):
    try:
        while should_continue(item):
            yield item
    except ValueError:
        yield None


def handler_control(item):
    try:
        raise ValueError(item)
    except ValueError:
        while should_continue(item):
            yield item


def exception_group_handler_control(item):
    try:
        raise ExceptionGroup("group", [ValueError(item)])
    except* ValueError:
        while should_continue(item):
            yield item


def protected_with_control(manager, item):
    with manager:
        while should_continue(item):
            yield item


def finally_control(item, cleanup):
    try:
        yield item
    finally:
        while should_continue(item):
            cleanup(item)


def nested_finally_control(item, cleanup):
    try:
        try:
            yield item
        finally:
            while should_continue(item):
                cleanup(item)
    finally:
        cleanup(None)


def inline_controls(first, second, items):
    while should_continue(first and second):
        yield first
    while should_continue([item for item in items if item]):
        yield items


async def coroutine_control(item, consume):
    while should_continue(item):
        item = await consume(item)


async def async_generator_control(item, consume):
    while should_continue(item):
        yield item
        item = await consume(item)
"#;
    let parsed = parse_module(source).unwrap();
    let module = module_compiler(source, &parsed)
        .compile_module(parsed.suite())
        .unwrap();

    for (name, expected) in [
        ("debug_callable_control", &[false][..]),
        (
            "optimized_generator_controls",
            &[false, false, false, false, false, false, false, false][..],
        ),
        ("unary_not_control", &[true][..]),
        ("nested_non_tail_control", &[true][..]),
        ("protected_try_control", &[true, true][..]),
        ("handler_control", &[true, true][..]),
        ("exception_group_handler_control", &[true, true, false][..]),
        ("protected_with_control", &[true, true][..]),
        ("finally_control", &[true, true][..]),
        ("nested_finally_control", &[true, true][..]),
        ("inline_controls", &[false, false, false, false][..]),
        ("coroutine_control", &[true][..]),
        ("async_generator_control", &[true][..]),
    ] {
        let function = module
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) if code.name == name => Some(code),
                _ => None,
            })
            .unwrap_or_else(|| panic!("module should contain {name}"));
        let mut ranges = Vec::new();
        let mut index = 0;
        while index < function.exception_table.len() {
            let start = read_exception_varint(&function.exception_table, &mut index);
            let length = read_exception_varint(&function.exception_table, &mut index);
            read_exception_varint(&function.exception_table, &mut index);
            read_exception_varint(&function.exception_table, &mut index);
            ranges.push(start..start + length);
        }
        let marker_owners = function
            .bytecode
            .as_chunks::<2>()
            .0
            .iter()
            .enumerate()
            .filter(|(_, instruction)| instruction[0] == NOT_TAKEN.code())
            .map(|(offset, _)| {
                let offset = u32::try_from(offset).unwrap();
                ranges.iter().any(|range| range.contains(&offset))
            })
            .collect::<Vec<_>>();
        assert_eq!(marker_owners, expected, "unexpected owner for {name}");
    }
}

#[test]
fn interns_adversarial_identifier_families_densely() {
    let mut interner = NameInterner::default();
    let names = (0..4096)
        .map(|index| format!("collision_prefix_{index:04}_collision_suffix"))
        .collect::<Vec<_>>();

    for (index, name) in names.iter().enumerate() {
        let id = interner.intern(name);
        assert_eq!(id, NameId::from_u32(u32::try_from(index).unwrap()));
        assert_eq!(interner.resolve(id), name);
    }
    for (index, name) in names.iter().enumerate().rev() {
        assert_eq!(
            interner.intern(name),
            NameId::from_u32(u32::try_from(index).unwrap())
        );
        assert_eq!(
            interner.get(name),
            Some(NameId::from_u32(u32::try_from(index).unwrap()))
        );
    }
    assert_eq!(
        interner.get("collision_prefix_missing_collision_suffix"),
        None
    );
}

#[test]
fn name_ids_do_not_define_observable_name_order() {
    let mut interner = NameInterner::default();
    let source_order =
        ["zeta", "alpha", "beta", "_hidden", "éclair"].map(|name| interner.intern(name));
    let mut lexical_order = Vec::new();
    for name in source_order {
        interner.insert_lexical(&mut lexical_order, name);
    }

    assert_eq!(
        lexical_order
            .iter()
            .map(|name| interner.resolve(*name))
            .collect::<Vec<_>>(),
        ["_hidden", "alpha", "beta", "zeta", "éclair"]
    );
    assert_eq!(source_order[0], NameId::from_u32(0));
    assert_eq!(source_order[1], NameId::from_u32(1));

    let plan = FunctionPlan {
        locals: source_order.to_vec(),
        freevars: lexical_order,
        ..FunctionPlan::default()
    }
    .into_lowering_plan();
    assert_eq!(
        plan.locals
            .iter()
            .map(|name| interner.resolve(*name))
            .collect::<Vec<_>>(),
        ["zeta", "alpha", "beta", "_hidden", "éclair"]
    );
    assert_eq!(
        plan.freevars
            .into_iter()
            .map(|name| interner.resolve(name))
            .collect::<Vec<_>>(),
        ["_hidden", "alpha", "beta", "zeta", "éclair"]
    );
}

#[test]
fn name_ids_deduplicate_private_name_mangling_collisions() {
    let source = r"__value = 0

class Container:
    __value = 1
    _Container__value = 2

    def method(self, __value):
        return self.__value + __value
";
    let parsed = parse_module(source).unwrap();
    let module = module_compiler(source, &parsed)
        .compile_module(parsed.suite())
        .unwrap();
    let class = module
        .constants
        .iter()
        .find_map(|constant| match constant {
            Constant::Code(code) if code.name == "Container" => Some(code),
            _ => None,
        })
        .expect("module should contain the class body");

    assert!(module.names.iter().any(|name| name.as_str() == "__value"));
    assert_eq!(
        class
            .names
            .iter()
            .filter(|name| name.as_str() == "_Container__value")
            .count(),
        1
    );

    let method = class
        .constants
        .iter()
        .find_map(|constant| match constant {
            Constant::Code(code) if code.name == "method" => Some(code),
            _ => None,
        })
        .expect("class should contain the method body");
    let metadata_name_id = |code: &CodeObject, spelling: &str| {
        let index = code
            .names
            .iter()
            .position(|name| name == spelling)
            .expect("metadata name should be present");
        code.marshal_provenance.metadata_ids.names[index]
    };
    let module_container_id = metadata_name_id(&module, "Container");
    let class_mangled_id = metadata_name_id(class, "_Container__value");
    let method_mangled_id = metadata_name_id(method, "_Container__value");
    let method_metadata = &method.marshal_provenance.metadata_ids;
    let method_local = method
        .locals
        .iter()
        .position(|name| name == "_Container__value")
        .expect("private parameter should be mangled");
    assert_eq!(class_mangled_id, method_mangled_id);
    assert_eq!(method_metadata.locals[method_local], class_mangled_id);

    let module_metadata = &module.marshal_provenance.metadata_ids;
    let class_metadata = &class.marshal_provenance.metadata_ids;
    assert_eq!(class_metadata.name, Some(module_container_id));
    assert_eq!(class_metadata.qualified_name, Some(module_container_id));
    assert_eq!(module_metadata.filename, class_metadata.filename);
    assert_eq!(class_metadata.filename, method_metadata.filename);
    for code in [&module, class, method] {
        let metadata = &code.marshal_provenance.metadata_ids;
        assert_eq!(metadata.names.len(), code.names.len());
        assert_eq!(metadata.locals.len(), code.locals.len());
        assert!(metadata.filename.as_u32() < metadata.domain_len);
        assert!(
            metadata
                .names
                .iter()
                .chain(&metadata.locals)
                .all(|id| id.as_u32() < metadata.domain_len)
        );
    }
}

#[test]
fn marshal_frozenset_presence_hint_is_conservative_across_nested_code() {
    fn nested_code<'a>(code: &'a CodeObject, name: &str) -> Option<&'a CodeObject> {
        for constant in &code.constants {
            if let Constant::Code(child) = constant {
                if child.name == name {
                    return Some(child);
                }
                if let Some(found) = nested_code(child, name) {
                    return Some(found);
                }
            }
        }
        None
    }

    let source = r"def outer(value):
    def inner(item):
        return (value, item, [1, 2, 3])
    return inner
";
    let parsed = parse_module(source).unwrap();
    let module = module_compiler(source, &parsed)
        .compile_module(parsed.suite())
        .unwrap();
    let outer = nested_code(&module, "outer").unwrap();
    let inner = nested_code(&module, "inner").unwrap();
    for code in [&module, outer, inner] {
        assert!(!code.marshal_provenance.subtree_has_frozenset_constants);
    }

    for body in [
        "return {1, 2, 3}",
        "return {1, 2, 3, *items}",
        "return value in {'alpha', 'beta'}",
    ] {
        let source = format!(
            "def outer(value, items):\n    def inner(value, items):\n        {body}\n    return inner\n"
        );
        let parsed = parse_module(&source).unwrap();
        let module = module_compiler(&source, &parsed)
            .compile_module(parsed.suite())
            .unwrap();
        let outer = nested_code(&module, "outer").unwrap();
        let inner = nested_code(&module, "inner").unwrap();
        for code in [&module, outer, inner] {
            assert!(
                code.marshal_provenance.subtree_has_frozenset_constants,
                "missing frozenset hint for `{body}` in `{}`",
                code.name
            );
        }
    }
}

#[test]
fn single_pass_local_collection_preserves_source_order() {
    let source = r"def ordering():
    target = external + target
    print(later)
    earlier = 1
    later = 2
    annotation_only
    annotation_only: int
";
    let parsed = parse_module(source).unwrap();
    let module = module_compiler(source, &parsed)
        .compile_module(parsed.suite())
        .unwrap();
    let function = module
        .constants
        .iter()
        .find_map(|constant| match constant {
            Constant::Code(code) if code.name == "ordering" => Some(code),
            _ => None,
        })
        .expect("module should contain the function body");

    // A self-reference is claimed at its load in the assignment value. A local loaded before its
    // eventual store keeps the position of that first load, and a referenced annotation-only
    // local follows the same rule.
    assert_eq!(
        function.locals,
        ["target", "later", "earlier", "annotation_only"]
    );
}

#[test]
fn flat_function_collection_preserves_local_and_header_order() {
    let source = r"@decorate(header_value)
def ordering(values, *, fallback=default_value):
    import package.part as imported
    from elsewhere import value as alias
    forward = later
    container[index] = forward
    for item in values:
        with manager(item) as resource:
            result = resource
    later = result
    return imported, alias, forward, item, later, fallback
";
    let parsed = parse_module(source).unwrap();
    let module = module_compiler(source, &parsed)
        .compile_module(parsed.suite())
        .unwrap();
    let function = module
        .constants
        .iter()
        .find_map(|constant| match constant {
            Constant::Code(code) if code.name == "ordering" => Some(code),
            _ => None,
        })
        .expect("module should contain the function body");

    assert_eq!(
        function.locals,
        [
            "values", "fallback", "imported", "alias", "later", "forward", "item", "resource",
            "result",
        ]
    );
}

#[test]
fn complex_function_collection_preserves_late_local_and_sibling_order() {
    let source = r"def complex_order(values):
    early = values
    early += later
    print(final)
    later = early
    final = later
    return early, later, final


def flat_order(values):
    print(later)
    first = values
    later = first
    return first, later
";
    let parsed = parse_module(source).unwrap();
    let module = module_compiler(source, &parsed)
        .compile_module(parsed.suite())
        .unwrap();
    let locals = |name: &str| {
        module
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) if code.name == name => Some(code.locals.as_slice()),
                _ => None,
            })
            .expect("module should contain the function body")
    };

    assert_eq!(
        locals("complex_order"),
        ["values", "early", "later", "final"]
    );
    assert_eq!(locals("flat_order"), ["values", "later", "first"]);
}

#[test]
fn annotation_only_locals_claim_slots_at_first_runtime_use() {
    let source = r"def ordering(values):
    assigned_late: object
    tuple_late: object
    read_late: object
    callback_late: object
    never_used: object

    first = 1
    first, tuple_late = values
    captured = read_late
    assigned_late = 2

    def callback_late():
        pass

    return first, tuple_late, captured, assigned_late, callback_late


def comprehension_order(values):
    item: object
    first = 1
    result = [item for item in values]
    return first, result


def complex_comprehension_order():
    source: object
    container: object
    first = 1
    result = [0 for container.value in source]
    return first, result


def mixed_comprehension_order(values):
    container: object
    index: object
    named: object
    first = 1
    result = [named for container[index], named in values]
    return first, result
";
    let parsed = parse_module(source).unwrap();
    let module = module_compiler(source, &parsed)
        .compile_module(parsed.suite())
        .unwrap();
    let function = module
        .constants
        .iter()
        .find_map(|constant| match constant {
            Constant::Code(code) if code.name == "ordering" => Some(code),
            _ => None,
        })
        .expect("module should contain the function body");

    assert_eq!(
        function.locals,
        [
            "values",
            "first",
            "tuple_late",
            "read_late",
            "captured",
            "assigned_late",
            "callback_late",
        ]
    );

    let comprehension = module
        .constants
        .iter()
        .find_map(|constant| match constant {
            Constant::Code(code) if code.name == "comprehension_order" => Some(code),
            _ => None,
        })
        .expect("module should contain the comprehension function");

    assert_eq!(comprehension.locals, ["values", "first", "item", "result"]);

    let complex = module
        .constants
        .iter()
        .find_map(|constant| match constant {
            Constant::Code(code) if code.name == "complex_comprehension_order" => Some(code),
            _ => None,
        })
        .expect("module should contain the complex-comprehension function");

    assert_eq!(complex.locals, ["first", "source", "container", "result"]);

    let mixed = module
        .constants
        .iter()
        .find_map(|constant| match constant {
            Constant::Code(code) if code.name == "mixed_comprehension_order" => Some(code),
            _ => None,
        })
        .expect("module should contain the mixed-comprehension function");

    assert_eq!(
        mixed.locals,
        ["values", "first", "named", "container", "index", "result"]
    );
}

#[test]
fn local_collection_follows_runtime_evaluation_order() {
    let source = r"def assignment_order():
    target: object
    rhs: object
    first = 1
    target = rhs + target


def augmented_order():
    target: object
    rhs: object
    first = 1
    target += rhs


def stored_subscript_order():
    container: object
    index: object
    rhs: object
    first = 1
    container[index] = rhs


def augmented_subscript_order():
    container: object
    index: object
    rhs: object
    first = 1
    container[index] += rhs


def deferred_expression_order():
    defaulted: object
    iterable: object
    exception: object
    exception_type: object
    first = 1
    callback = lambda value=defaulted: value
    generated = (value for value in iterable)
    try:
        pass
    except exception_type as exception:
        pass


def comprehension_reuses_assignment_target():
    tasks = [task for task in tasks]
    return tasks


def comprehension_body_uses_assignment_target(values):
    tasks = [tasks for task in values]
    return tasks


def comprehension_outer_iterable_order():
    earlier: object
    target: object
    first = 1
    target = [item for item in (earlier, target)]
    return target


def nested_comprehension_order(outer):
    result = [[nested for nested in inner] for item in outer]
    inner = ()
    return result


def generator_first_iterable_comprehension_order(outer):
    result = [(nested for nested in [inner_item for inner_item in inner]) for item in outer]
    inner = ()
    return result
";
    let parsed = parse_module(source).unwrap();
    let module = module_compiler(source, &parsed)
        .compile_module(parsed.suite())
        .unwrap();
    let locals = |name: &str| {
        module
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) if code.name == name => Some(code.locals.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("module should contain {name}"))
    };

    assert_eq!(locals("assignment_order"), ["first", "rhs", "target"]);
    assert_eq!(locals("augmented_order"), ["first", "target", "rhs"]);
    assert_eq!(
        locals("stored_subscript_order"),
        ["first", "rhs", "container", "index"]
    );
    assert_eq!(
        locals("augmented_subscript_order"),
        ["first", "container", "index", "rhs"]
    );
    assert_eq!(
        locals("deferred_expression_order"),
        [
            "first",
            "defaulted",
            "callback",
            "iterable",
            "generated",
            "exception_type",
            "exception",
        ]
    );
    assert_eq!(
        locals("comprehension_reuses_assignment_target"),
        ["tasks", "task"]
    );
    assert_eq!(
        locals("comprehension_body_uses_assignment_target"),
        ["values", "task", "tasks"]
    );
    assert_eq!(
        locals("comprehension_outer_iterable_order"),
        ["first", "earlier", "target", "item"]
    );
    assert_eq!(
        locals("nested_comprehension_order"),
        ["outer", "item", "nested", "inner", "result"]
    );
    assert_eq!(
        locals("generator_first_iterable_comprehension_order"),
        ["outer", "item", "inner_item", "inner", "result"]
    );
}

#[test]
fn local_collection_includes_decorator_bindings_and_delete_targets() {
    let source = r"def bindings():
    @decorator
    def function():
        pass

    @(assigned := decorator)
    class Class:
        pass

    del deleted
    decorator = identity
";
    let parsed = parse_module(source).unwrap();
    let module = module_compiler(source, &parsed)
        .compile_module(parsed.suite())
        .unwrap();
    let function = module
        .constants
        .iter()
        .find_map(|constant| match constant {
            Constant::Code(code) if code.name == "bindings" => Some(code),
            _ => None,
        })
        .expect("module should contain the function body");

    assert_eq!(
        function.locals,
        ["decorator", "function", "assigned", "Class", "deleted"]
    );
}

#[test]
fn local_collection_includes_direct_definition_header_bindings() {
    let source = r"def bindings():
    @decorator
    def function(positional=(function_target := function_source), *, keyword=keyword_source):
        pass

    @class_decorator
    class Class((base_target := base_source), metaclass=metaclass_source):
        pass

    function_source = keyword_source = None
    base_source = object
    metaclass_source = type
    decorator = class_decorator = identity
";
    let parsed = parse_module(source).unwrap();
    let module = module_compiler(source, &parsed)
        .compile_module(parsed.suite())
        .unwrap();
    let function = module
        .constants
        .iter()
        .find_map(|constant| match constant {
            Constant::Code(code) if code.name == "bindings" => Some(code),
            _ => None,
        })
        .expect("module should contain the function body");

    assert_eq!(
        function.locals,
        [
            "decorator",
            "function_source",
            "function_target",
            "keyword_source",
            "function",
            "class_decorator",
            "base_source",
            "base_target",
            "metaclass_source",
            "Class",
        ]
    );
}

#[test]
fn symbol_name_ids_preserve_mangled_cell_and_freevar_order() {
    let source = r"class ____Container:
    def outer(self):
        zeta = 1
        __private = 2
        éclair = 3
        alpha = 4

        def inner():
            return zeta, __private, éclair, alpha

        return inner
";
    let parsed = parse_module(source).unwrap();
    let module = module_compiler(source, &parsed)
        .compile_module(parsed.suite())
        .unwrap();
    let class = module
        .constants
        .iter()
        .find_map(|constant| match constant {
            Constant::Code(code) if code.name == "____Container" => Some(code),
            _ => None,
        })
        .expect("module should contain the class body");
    let outer = class
        .constants
        .iter()
        .find_map(|constant| match constant {
            Constant::Code(code) if code.name == "outer" => Some(code),
            _ => None,
        })
        .expect("class should contain the outer function");
    let inner = outer
        .constants
        .iter()
        .find_map(|constant| match constant {
            Constant::Code(code) if code.name == "inner" => Some(code),
            _ => None,
        })
        .expect("outer should contain the inner function");

    assert_eq!(
        outer.locals,
        [
            "self",
            "inner",
            "_Container__private",
            "alpha",
            "zeta",
            "éclair"
        ]
    );
    assert_eq!(
        inner.locals,
        ["_Container__private", "alpha", "zeta", "éclair"]
    );
}

#[test]
fn module_plan_preserves_ordered_bindings_and_scope_facts() {
    let source = r"from __future__ import annotations, barry_as_FLUFL
import package.module as imported
from package import member as alias

def child():
    global nested_global

super = object()
result = [
    (captured := item, lambda: item, nested)
    for item in values
    for nested in others
]
if condition:
    annotated: int
";
    let parsed = parse_module(source).unwrap();
    let compiler = module_compiler(source, &parsed);
    let plan = compiler.context.module_plan();
    let names = &compiler.context.scopes.names;
    let name_id = |name| {
        names
            .get(name)
            .unwrap_or_else(|| panic!("module analysis did not intern `{name}`"))
    };

    assert_eq!(
        plan.future_flags,
        CO_FUTURE_ANNOTATIONS | CO_FUTURE_BARRY_AS_BDFL
    );
    assert_eq!(
        plan.comprehension_targets
            .iter()
            .map(|name| names.resolve(*name))
            .collect::<Vec<_>>(),
        ["item", "nested", "captured"]
    );
    assert!(plan.module_globals.contains(name_id("nested_global")));
    assert!(plan.module_globals.contains(name_id("captured")));
    assert!(plan.imported_scope_names.contains(name_id("imported")));
    assert!(plan.imported_scope_names.contains(name_id("alias")));
    assert!(
        plan.imported_scope_names
            .contains(name_id(SHADOWED_SUPER_SENTINEL))
    );
    assert!(plan.imported_scope_names.contains(name_id("annotations")));
    assert!(
        plan.imported_scope_names
            .contains(name_id("barry_as_FLUFL"))
    );
    assert!(plan.comprehension_cell_names.contains(name_id("item")));
    assert!(plan.has_annotations);
    assert!(plan.has_simple_annotations);
}

#[test]
fn scope_body_facts_preserve_named_scope_and_annotation_boundaries() {
    let source = r"def outer():
    global module_name
    nonlocal captured
    item: annotation = value
    closure = lambda: captured
    generated = ((captured := element) for element in values)
    inlined = [lambda: element for element in values]

    def child[U: child_bound]():
        return child_body

    class Child[T: class_bound](base):
        body = class_body

    type Alias[V: alias_bound] = tuple[V, alias_value]
";
    let parsed = parse_module(source).unwrap();
    let Stmt::FunctionDef(definition) = &parsed.suite()[0] else {
        panic!("expected a function definition");
    };
    let facts = ScopeBodyFacts::for_function(&definition.body);

    assert!(facts.globals.contains("module_name"));
    assert!(facts.nonlocals.contains("captured"));
    assert!(facts.references.contains("value"));
    assert!(!facts.references.contains("annotation"));
    assert!(!facts.references.contains("child_body"));
    assert!(!facts.references.contains("class_body"));
    assert!(facts.nested_lambda_requirements.contains("captured"));
    assert!(facts.nested_generator_requirements.contains("captured"));
    assert!(facts.generator_named_targets.contains("captured"));
    assert!(facts.inlined_comprehension_cellvars.contains("element"));
    for name in [
        "child_bound",
        "class_bound",
        "base",
        "alias_bound",
        "alias_value",
    ] {
        assert!(
            facts.nested_annotation_scope_requirements.contains(name),
            "missing annotation-scope requirement {name}"
        );
    }
    assert!(facts.has_function_definition);
    assert!(facts.has_generic_definition);
    assert!(facts.has_type_alias);
    assert!(facts.has_annotations);

    let source = "class Container:\n    item: annotation = value\n";
    let parsed = parse_module(source).unwrap();
    let Stmt::ClassDef(definition) = &parsed.suite()[0] else {
        panic!("expected a class definition");
    };
    let deferred = ScopeBodyFacts::for_class(&definition.body, true);
    assert!(deferred.references.contains("value"));
    assert!(!deferred.references.contains("annotation"));
    let eager = ScopeBodyFacts::for_class(&definition.body, false);
    assert!(eager.references.contains("value"));
    assert!(eager.references.contains("annotation"));
}

#[test]
fn comprehension_shadowing_distinguishes_explicit_and_synthetic_dunder_class_references() {
    let source = r"def synthetic(values):
    super()
    return [__class__ for __class__ in values]

def explicit_before(values):
    __class__
    return [__class__ for __class__ in values]

def explicit_after(values):
    result = [__class__ for __class__ in values]
    __class__
    return result
";
    let parsed = parse_module(source).unwrap();
    let compiler = module_compiler(source, &parsed);
    let arena = compiler.scope_arena();
    let dunder_class = arena.names.get("__class__").unwrap();
    let shadowed = |name: &str| {
        let definition = parsed
            .suite()
            .iter()
            .find_map(|statement| match statement {
                Stmt::FunctionDef(definition) if definition.name.as_str() == name => {
                    Some(definition)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("expected function {name}"));
        let id = compiler.function_scope_id(definition).unwrap();
        arena
            .function(id)
            .unwrap()
            .comprehension_shadowed_targets
            .contains(&dunder_class)
    };

    assert!(!shadowed("synthetic"));
    assert!(shadowed("explicit_before"));
    assert!(shadowed("explicit_after"));
}

#[test]
fn compiles_a_simple_module() {
    let source = "answer = 6 * 7\nprint(answer)\n";
    let parsed = parse_module(source).unwrap();
    let code = module_compiler(source, &parsed)
        .compile_module(parsed.suite())
        .unwrap();
    assert_eq!(code.name, "<module>");
    assert!(code.stack_size >= 2);
    assert!(code.names.iter().any(|name| name == "print"));
}

#[test]
fn compiles_type_alias() {
    let source = "type Alias = int\n";
    let parsed = parse_module(source).unwrap();
    module_compiler(source, &parsed)
        .compile_module(parsed.suite())
        .unwrap();
}

#[test]
fn retains_parser_tokens_in_the_shared_compilation_context() {
    let source = "value = (\n    1  # comment\n)\n";
    let parsed = parse_module(source).unwrap();
    let compiler = module_compiler(source, &parsed);

    assert_eq!(compiler.context.tokens, parsed.tokens());
}

#[test]
fn definition_locations_start_at_keyword_tokens() {
    let source = r#"@decorate("class def async")
async \
    def function():
    pass
@decorate("def class")
class Container:
    pass
"#;
    let parsed = parse_module(source).unwrap();
    let compiler = module_compiler(source, &parsed);
    let [Stmt::FunctionDef(function), Stmt::ClassDef(class)] = parsed.suite().as_slice() else {
        panic!("expected a function and a class");
    };

    assert_eq!(
        compiler.definition_location(
            function.range,
            function.name.range(),
            TokenKind::Def,
            function.is_async,
        ),
        SourceLocation::new(2, 4, 0, 8),
    );
    assert_eq!(
        compiler.definition_location(class.range, class.name.range(), TokenKind::Class, false),
        SourceLocation::new(6, 7, 0, 8),
    );
}

#[test]
fn definition_locations_preserve_crlf_comments_and_continuations() {
    let source = concat!(
        "@decorate(\"class def async\")\r\n",
        "async def fast():\r\n",
        "    pass; # comment\r\n",
        "class Fast:\r\n",
        "    pass  # ; not a boundary\r\n",
        "def  continued():\r\n",
        "    pass \\\r\n",
        "        ;\r\n",
        "def at_eof(): pass",
    );
    let parsed = parse_module(source).unwrap();
    let compiler = module_compiler(source, &parsed);
    let [
        Stmt::FunctionDef(asynchronous),
        Stmt::ClassDef(class),
        Stmt::FunctionDef(continued),
        Stmt::FunctionDef(at_eof),
    ] = parsed.suite().as_slice()
    else {
        panic!("expected three functions and a class");
    };

    let function_location = |function: &ruff_python_ast::StmtFunctionDef| {
        compiler.definition_location(
            function.range,
            function.name.range(),
            TokenKind::Def,
            function.is_async,
        )
    };
    assert_eq!(
        function_location(asynchronous),
        SourceLocation::new(2, 3, 0, 9)
    );
    assert_eq!(
        compiler.definition_location(class.range, class.name.range(), TokenKind::Class, false),
        SourceLocation::new(4, 5, 0, 8),
    );
    assert_eq!(
        function_location(continued),
        SourceLocation::new(6, 8, 0, 9)
    );
    assert_eq!(function_location(at_eof), SourceLocation::new(9, 9, 0, 18));
}

#[test]
fn trailing_semicolon_locations_follow_token_boundaries() {
    let source = r#"first = "not; boundary"; second = 2
continued = 1 \
    ; third = 3
commented = 4  # ; not a boundary
"#;
    let parsed = parse_module(source).unwrap();
    let compiler = module_compiler(source, &parsed);
    let locations = parsed
        .suite()
        .iter()
        .map(|statement| compiler.source_location_including_trailing_semicolon(statement.range()))
        .collect::<Vec<_>>();

    assert_eq!(
        locations,
        [
            SourceLocation::new(1, 1, 0, 24),
            SourceLocation::new(1, 1, 25, 35),
            SourceLocation::new(2, 3, 0, 5),
            SourceLocation::new(3, 3, 6, 15),
            SourceLocation::new(4, 4, 0, 13),
        ],
    );
}

#[test]
fn generator_expression_ranges_follow_parser_tokens() {
    for (source, expected) in [
        (
            "result = consume(\n    # before\n    value for value in values\n    # after\n)\n",
            "(\n    # before\n    value for value in values\n    # after\n)",
        ),
        (
            "result = consume(\\\n    value for value in values\\\n)\n",
            "(\\\n    value for value in values\\\n)",
        ),
        (
            r#"result = consume((value, nested[")"], {"(": value}) for value in values if predicate(")#"))
"#,
            r#"((value, nested[")"], {"(": value}) for value in values if predicate(")#"))"#,
        ),
        (
            r#"result = (((")", {"(": value}) for value in values))
"#,
            r#"((")", {"(": value}) for value in values)"#,
        ),
    ] {
        let parsed = parse_module(source).unwrap();
        let Stmt::Assign(assignment) = &parsed.suite()[0] else {
            panic!("expected an assignment");
        };
        let generator = match assignment.value.as_ref() {
            Expr::Generator(generator) => generator,
            Expr::Call(call) => {
                let [Expr::Generator(generator)] = call.arguments.args.as_ref() else {
                    panic!("expected a generator call argument");
                };
                generator
            }
            expression => panic!("expected a generator or call, got {expression:?}"),
        };
        let compiler = module_compiler(source, &parsed);
        let range = compiler.generator_expression_range(generator);
        let expected_start = source.find(expected).unwrap();
        let actual_start = usize::from(range.start());
        let actual_end = usize::from(range.end());

        assert_eq!(actual_start, expected_start);
        assert_eq!(actual_end, expected_start + expected.len());
        assert_eq!(&source[actual_start..actual_end], expected);
    }
}

#[test]
fn interpolated_string_content_ranges_follow_ast_parts() {
    for (source, expected) in [
        (
            r"value = rf'raw\path {item}'
",
            r"raw\path {item}",
        ),
        (
            "value = f'''first\n{item!r}\n# literal\nlast'''\n",
            "first\n{item!r}\n# literal\nlast",
        ),
        (
            r#"value = r'left' f"middle {item}" '''right'''
"#,
            r#"left' f"middle {item}" '''right"#,
        ),
        (
            "value = t'left' tr\"\"\"right {item}\n# literal\"\"\"\n",
            "left' tr\"\"\"right {item}\n# literal",
        ),
        (
            "value = f\"\"\"start {(\n    {'}': nested['{']}  # expression comment\n)} end\"\"\"\n",
            "start {(\n    {'}': nested['{']}  # expression comment\n)} end",
        ),
    ] {
        let parsed = parse_module(source).unwrap();
        let Stmt::Assign(assignment) = &parsed.suite()[0] else {
            panic!("expected an assignment");
        };
        let string = match assignment.value.as_ref() {
            Expr::FString(fstring) => StringLike::FString(fstring),
            Expr::TString(tstring) => StringLike::TString(tstring),
            expression => panic!("expected an interpolated string, got {expression:?}"),
        };
        let range = interpolated_string_content_range(string);
        let expected_start = source.find(expected).unwrap();
        let actual_start = usize::from(range.start());
        let actual_end = usize::from(range.end());

        assert_eq!(actual_start, expected_start);
        assert_eq!(actual_end, expected_start + expected.len());
        assert_eq!(&source[actual_start..actual_end], expected);
    }
}

#[test]
fn debug_text_ranges_follow_ast_text_and_marker_tokens() {
    fn first_interpolation(expression: &Expr) -> &ruff_python_ast::InterpolatedElement {
        match expression {
            Expr::FString(fstring) => fstring
                .value
                .elements()
                .find_map(|element| element.as_interpolation()),
            Expr::TString(tstring) => tstring
                .value
                .elements()
                .find_map(|element| element.as_interpolation()),
            expression => panic!("expected an interpolated string, got {expression:?}"),
        }
        .expect("expected a debug interpolation")
    }

    for (source, expected_debug, expected_expression) in [
        (
            "value = f\"{  item   =   !r}\"\n",
            "  item   =   ",
            "  item   ",
        ),
        (
            "value = t\"\"\"{(\n    mapping[\"=\"]  # = inside comment\n)   =   :>{width}}\"\"\"\n",
            "(\n    mapping[\"=\"]  # = inside comment\n)   =   ",
            "(\n    mapping[\"=\"]  # = inside comment\n)   ",
        ),
        (
            "value = t\"\"\"{f\"{item=} = literal\" = !s:^10}\"\"\"\n",
            "f\"{item=} = literal\" = ",
            "f\"{item=} = literal\" ",
        ),
        (
            "value = f\"\"\"{item=  # after marker\n}\"\"\"\n",
            "item=  # after marker\n",
            "item",
        ),
    ] {
        let parsed = parse_module(source).unwrap();
        let Stmt::Assign(assignment) = &parsed.suite()[0] else {
            panic!("expected an assignment");
        };
        let interpolation = first_interpolation(&assignment.value);
        let compiler = module_compiler(source, &parsed);
        let debug_range = debug_text_range(interpolation);
        let expression_range = compiler.debug_expression_source_range(interpolation);

        assert_eq!(
            &source[usize::from(debug_range.start())..usize::from(debug_range.end())],
            expected_debug,
        );
        assert_eq!(
            interpolation.debug_text.as_ref().unwrap().as_str(),
            expected_debug,
        );
        assert_eq!(
            &source[usize::from(expression_range.start())..usize::from(expression_range.end())],
            expected_expression,
        );
    }
}

#[test]
fn expression_comment_stripping_uses_ruff_comment_tokens() {
    for (source, expected) in [
        (
            "(r'# raw', R\"# raw double\", b'# bytes', rb'# raw bytes', u'# unicode', '# single', \"# double\", '''# triple''', \"\"\"# triple double\"\"\")  # remove\r\nnext",
            "(r'# raw', R\"# raw double\", b'# bytes', rb'# raw bytes', u'# unicode', '# single', \"# double\", '''# triple''', \"\"\"# triple double\"\"\")  \r\nnext",
        ),
        (
            "\"escaped \\\" # string\" + '\\\\ # string'  # remove\nvalue",
            "\"escaped \\\" # string\" + '\\\\ # string'  \nvalue",
        ),
        (
            "f\"\"\"outer # {\n    f'inner # {value}'  # nested comment\n} tail #\"\"\"  # remove\n",
            "f\"\"\"outer # {\n    f'inner # {value}'  # nested comment\n} tail #\"\"\"  \n",
        ),
        (
            "rf\"literal # {value}\" + tr'''template # {value}'''  # remove\n",
            "rf\"literal # {value}\" + tr'''template # {value}'''  \n",
        ),
        (
            "  call('#')  # generated comment\r\n =   # trailing debug comment\r\n",
            "  call('#')  \r\n =   \r\n",
        ),
        (
            "value  # first\n    # own line\nother  # last",
            "value  \n    \nother  ",
        ),
    ] {
        assert_eq!(strip_expression_comments(source), expected, "{source:?}");
    }
}

#[test]
fn annotation_unparsing_preserves_name_union_grouping_and_complex_members() {
    for (annotation, expected) in [
        ("((alpha.Type))", "alpha.Type"),
        ("None", "None"),
        ("((A | B) | C)", "A | B | C"),
        ("A | (B | (C | None))", "A | (B | (C | None))"),
        ("A | (B | \"Forward\")", "A | (B | 'Forward')"),
        ("('one' ' two') | None", "'one two' | None"),
        ("u'κ' | (None | A.B)", "u'κ' | (None | A.B)"),
        ("U'κ' | (A | B)", "'κ' | (A | B)"),
        ("('value').Type | (A | B)", "'value'.Type | (A | B)"),
        ("A | (T[0] | B)", "A | (T[0] | B)"),
        (
            "A.real | bool[True] | type[...]",
            "A.real | bool[True] | type[...]",
        ),
        ("(A | B)[C | None]", "(A | B)[C | None]"),
        ("A | (T() | (A | B)())", "A | (T() | (A | B)())"),
        ("(A | B)(X | Y, None, \"s\")", "(A | B)(X | Y, None, 's')"),
        ("1 - (2 - 3) * -(x ** y)", "1 - (2 - 3) * -x ** y"),
        ("(-x) ** (y ** -z)", "(-x) ** y ** (-z)"),
        ("A | (not B)", "A | (not B)"),
        (r#""\ud800" | T"#, r"'\ud800' | T"),
        ("0x1_0000_0000_0000_0000 | T", "18446744073709551616 | T"),
        ("(1).real | T", "1 .real | T"),
        (
            "(0x1_0000_0000_0000_0000).real | T",
            "18446744073709551616 .real | T",
        ),
        (r#"("\ud800").Type | T"#, r"'\ud800'.Type | T"),
        ("((A | B)).Nested | None", "(A | B).Nested | None"),
        ("(A | B).Nested.More", "(A | B).Nested.More"),
        ("\u{212A}.\u{FB03} | \u{1D4AF}", "K.ffi | T"),
        ("α.β | None", "α.β | None"),
        ("A | tuple[(B, C)]", "A | tuple[B, C]"),
        ("((A, B)) | None", "(A, B) | None"),
        (r#"A | "Forward""#, "A | 'Forward'"),
        ("A | (lambda *args: B)", "A | (lambda*args: B)"),
        ("A | factory(**(B | C))", "A | factory(**B | C)"),
    ] {
        let source = format!("value: {annotation}");
        let module = parse_module(&source).unwrap();
        let [Stmt::AnnAssign(assignment)] = module.suite().as_slice() else {
            panic!("expected an annotated assignment");
        };
        assert_eq!(
            unparse_annotation(&assignment.annotation, &source),
            expected,
            "{annotation}",
        );
    }
}

#[test]
fn annotation_unparsing_uses_tokens_for_nested_delimiters() {
    for (source, expected) in [
        ("value: tuple[(int, str)]", "tuple[int, str]"),
        (
            "value: dict[str, tuple[(int, str)]]",
            "dict[str, tuple[int, str]]",
        ),
        (
            "value: Literal[\"[()]\", (int | str)]",
            "Literal['[()]', int | str]",
        ),
        (
            "value: Callable[[(int, str)], None]",
            "Callable[[(int, str)], None]",
        ),
    ] {
        let module = parse_module(source).unwrap();
        let [Stmt::AnnAssign(assignment)] = module.suite().as_slice() else {
            panic!("expected an annotated assignment");
        };
        assert_eq!(unparse_annotation(&assignment.annotation, source), expected);
    }
}

#[test]
fn annotation_unparsing_matches_cpython_string_repr_categories() {
    for (literal, expected) in [
        (r#""\u200e""#, r"'\u200e'"),
        (r#""\u2028""#, r"'\u2028'"),
        (r#""\ue000""#, r"'\ue000'"),
        (r#""\u0378""#, r"'\u0378'"),
        (r#""\u00a0""#, r"'\xa0'"),
        (r#""\x08\x0c""#, r"'\x08\x0c'"),
        (r#""\U0001f600""#, "'😀'"),
    ] {
        let source = format!("value: {literal}");
        let module = parse_module(&source).unwrap();
        let [Stmt::AnnAssign(assignment)] = module.suite().as_slice() else {
            panic!("expected an annotated assignment");
        };
        assert_eq!(
            unparse_annotation(&assignment.annotation, &source),
            expected
        );
    }
}

#[test]
fn source_locations_pair_utf8_columns_crlf_boundaries_and_eof() {
    let source = "α = β\r\nγδ = ε";
    let parsed = parse_module(source).unwrap();
    let compiler = module_compiler(source, &parsed);
    let range = |start, end| TextRange::new(TextSize::new(start), TextSize::new(end));

    assert_eq!(
        compiler.source_location(range(0, 7)),
        SourceLocation::new(1, 1, 0, 7)
    );
    assert_eq!(
        compiler.source_location(range(5, 8)),
        SourceLocation::new(1, 1, 5, 8)
    );
    assert_eq!(
        compiler.source_location(range(5, 9)),
        SourceLocation::new(1, 2, 5, 0)
    );
    assert_eq!(
        compiler.source_location(range(9, 18)),
        SourceLocation::new(2, 2, 0, 9)
    );
    assert_eq!(
        compiler.source_location(range(5, 18)),
        SourceLocation::new(1, 2, 5, 9)
    );

    let source = "α = β\r\nγδ = ε\r\n";
    let parsed = parse_module(source).unwrap();
    let compiler = module_compiler(source, &parsed);

    assert_eq!(
        compiler.source_location(range(9, 19)),
        SourceLocation::new(2, 2, 0, 10)
    );
    assert_eq!(
        compiler.source_location(range(9, 20)),
        SourceLocation::new(2, 3, 0, 0)
    );
}

#[test]
fn source_positions_visit_adjacent_lines_in_both_directions_and_fall_back_for_jumps() {
    let source = "a = 1\r\nb = 2\n\n# comment\nc = 3\r\nd = 4\n";
    let parsed = parse_module(source).unwrap();
    let compiler = module_compiler(source, &parsed);
    let expected = ruff_source_file::LineIndex::from_source_text(source);
    let lines = expected.line_starts();
    for line in [0, 1, 2, 3, 4, 5, 6, 5, 4, 3, 2, 1, 0, 5, 2, 6, 0] {
        let start = lines[line];
        let end = lines
            .get(line + 1)
            .map_or(source.len(), |next| usize::from(*next) - 1);
        for offset in [usize::from(start), end] {
            let offset = u32::try_from(offset).unwrap();
            assert_eq!(
                compiler.source_position(offset),
                (
                    i32::try_from(line + 1).unwrap(),
                    i32::try_from(offset - u32::from(start)).unwrap()
                ),
            );
        }
    }
}

#[test]
fn source_positions_use_utf8_byte_columns() {
    let source = "α = 1\nβγ = 2\n";
    let parsed = parse_module(source).unwrap();
    let compiler = module_compiler(source, &parsed);

    assert_eq!(compiler.source_position(0), (1, 0));
    assert_eq!(compiler.source_position(2), (1, 2));
    assert_eq!(compiler.source_position(7), (2, 0));
    assert_eq!(compiler.source_position(9), (2, 2));
    assert_eq!(compiler.source_position(11), (2, 4));
    assert_eq!(compiler.line_number(11), 2);
}

#[test]
fn source_locations_preserve_multiline_endpoints_when_revisited() {
    let source = "α\nβγ\nδ\n";
    let parsed = parse_module(source).unwrap();
    let compiler = module_compiler(source, &parsed);
    let range = |start, end| TextRange::new(TextSize::new(start), TextSize::new(end));

    for _ in 0..2 {
        assert_eq!(
            compiler.source_location(range(0, 7)),
            SourceLocation::new(1, 2, 0, 4),
        );
        assert_eq!(
            compiler.source_location(range(0, 2)),
            SourceLocation::new(1, 1, 0, 2),
        );
        assert_eq!(
            compiler.source_location(range(5, 11)),
            SourceLocation::new(2, 4, 2, 0),
        );
        assert_eq!(compiler.source_position(8), (3, 0));
        assert_eq!(compiler.source_position(u32::MAX), (4, 0));
    }
}

#[test]
fn source_positions_clamp_to_eof() {
    let source = "first\né\n";
    let parsed = parse_module(source).unwrap();
    let compiler = module_compiler(source, &parsed);
    let eof = u32::try_from(source.len()).unwrap();

    assert_eq!(compiler.source_position(eof), (3, 0));
    assert_eq!(compiler.source_position(u32::MAX), (3, 0));
    assert_eq!(compiler.line_number(u32::MAX), 3);
}

#[test]
fn source_positions_preserve_crlf_boundaries_and_eof() {
    let source = "one\r\ntwo";
    let parsed = parse_module(source).unwrap();
    let compiler = module_compiler(source, &parsed);

    assert_eq!(compiler.source_position(2), (1, 2));
    assert_eq!(compiler.source_position(5), (2, 0));
    assert_eq!(compiler.source_position(8), (2, 3));
    assert_eq!(compiler.source_position(4), (1, 4));
    assert_eq!(compiler.source_position(u32::MAX), (2, 3));

    let source = "one\r\ntwo\r\n";
    let parsed = parse_module(source).unwrap();
    let compiler = module_compiler(source, &parsed);

    assert_eq!(compiler.source_position(8), (2, 3));
    assert_eq!(compiler.source_position(9), (2, 4));
    assert_eq!(compiler.source_position(10), (3, 0));
    assert_eq!(compiler.source_position(5), (2, 0));
    assert_eq!(compiler.source_position(u32::MAX), (3, 0));
}

#[test]
fn coroutine_handler_finally_pop_blocks_match_cpython_owners() {
    let source = r"async def fresh():
    try:
        await source
    except Exception:
        recover()
        raise
    finally:
        cleanup()

async def owned(self):
    try:
        await source
    except Exception:
        recover()
        raise
    finally:
        self.cleanup()
";
    let parsed = parse_module(source).unwrap();
    let module = module_compiler(source, &parsed)
        .compile_module(parsed.suite())
        .unwrap();

    for (name, expected) in [("fresh", [false, false]), ("owned", [false, false])] {
        let function = module
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) if code.name == name => Some(code),
                _ => None,
            })
            .unwrap_or_else(|| panic!("module should contain {name}"));
        let mut ranges = Vec::new();
        let mut index = 0;
        while index < function.exception_table.len() {
            let start = read_exception_varint(&function.exception_table, &mut index);
            let length = read_exception_varint(&function.exception_table, &mut index);
            read_exception_varint(&function.exception_table, &mut index);
            read_exception_varint(&function.exception_table, &mut index);
            ranges.push(start..start + length);
        }
        let nop_owners = function
            .bytecode
            .as_chunks::<2>()
            .0
            .iter()
            .enumerate()
            .filter(|(_, instruction)| instruction[0] == NOP.code())
            .map(|(offset, _)| {
                let offset = u32::try_from(offset).unwrap();
                ranges.iter().any(|range| range.contains(&offset))
            })
            .collect::<Vec<_>>();

        assert_eq!(nop_owners, expected, "unexpected NOP owners for {name}");
    }
}

#[test]
fn coroutine_boolean_value_markers_follow_current_cfg_slot_history() {
    let source = r#"async def later_value_marker(func, instance, args, kwargs):
    tool_call = get_argument_value(args, kwargs, 0, "tool_call", True)
    tool_name = getattr(tool_call, "tool_name", None) or "Pydantic Tool"
    tool_manager_tools = getattr(instance, "tools", {}) or {}
    tool_instance = tool_manager_tools.get(tool_name) or None
    return await traced_tool_run(func, tool_instance, args, kwargs, tool_name)

async def first_value_marker(value, run):
    selected = value or None
    return await run(selected)

async def nested_region(value, run):
    try:
        selected = value or None
    except LookupError:
        selected = None
    return await run(selected)

async def after_suspension(value, run):
    await run(None)
    selected = value or None
    return selected

async def one_slot_after_condition(value, fallback, run, flag):
    if flag:
        run(flag)
    selected = bool(value or fallback)
    return await run(selected)

async def iterator_one_slot_after_condition(items, fallback, run, flag):
    for item in items:
        if flag:
            run(flag)
        selected = bool(item.value or fallback)
        run(selected)

async def iterator_keyword_gap(items, fallback, run):
    result = []
    for item in items:
        if (value := item.value) and not run(value):
            value = fallback
        result.append(Record(value=item.value or fallback, other=item.other))
    return result
"#;
    let parsed = parse_module(source).unwrap();
    let module = module_compiler(source, &parsed)
        .compile_module(parsed.suite())
        .unwrap();

    for (name, expected) in [
        ("later_value_marker", &[true, false, false][..]),
        ("first_value_marker", &[true][..]),
        ("nested_region", &[true, true][..]),
        ("after_suspension", &[false][..]),
        ("one_slot_after_condition", &[true, false][..]),
        ("iterator_one_slot_after_condition", &[true, false][..]),
        ("iterator_keyword_gap", &[true, false, false][..]),
    ] {
        let function = module
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) if code.name == name => Some(code),
                _ => None,
            })
            .unwrap_or_else(|| panic!("module should contain {name}"));
        let mut ranges = Vec::new();
        let mut index = 0;
        while index < function.exception_table.len() {
            let start = read_exception_varint(&function.exception_table, &mut index);
            let length = read_exception_varint(&function.exception_table, &mut index);
            read_exception_varint(&function.exception_table, &mut index);
            read_exception_varint(&function.exception_table, &mut index);
            ranges.push(start..start + length);
        }
        let marker_owners = function
            .bytecode
            .as_chunks::<2>()
            .0
            .iter()
            .enumerate()
            .filter(|(_, instruction)| instruction[0] == NOT_TAKEN.code())
            .map(|(offset, _)| {
                let offset = u32::try_from(offset).unwrap();
                ranges.iter().any(|range| range.contains(&offset))
            })
            .collect::<Vec<_>>();

        assert_eq!(
            marker_owners, expected,
            "unexpected marker owners for {name}"
        );
    }
}

#[test]
fn async_with_terminal_condition_markers_respect_direct_body_and_argument_boundaries() {
    let source = r"async def hazardous_method_ifexp_arg(manager, state, value, fallback):
    async with manager:
        if state.first:
            return value
        if not state.check(value if value is None else fallback):
            raise LocalError(value)
    return fallback

async def later_nested_if_terminal(manager, state, value):
    async with manager:
        if state.first:
            return value
        if state.second:
            if state.inner:
                return value
            else:
                raise LocalError(value)
    return value

async def cap_16(manager, state, value):
    async with manager:
        if state.first:
            return value
        a1 = value
        a2 = value
        a3 = value
        a4 = value
        a5 = value
        a6 = value
        a7 = value
        a8 = value
        a9 = value
        a10 = value
        a11 = value
        a12 = value
        a13 = value
        a14 = value
        if state.second:
            return value
    return value

async def cap_17(manager, state, value):
    async with manager:
        if state.first:
            return value
        a1 = value
        a2 = value
        a3 = value
        a4 = value
        a5 = value
        a6 = value
        a7 = value
        a8 = value
        a9 = value
        a10 = value
        a11 = value
        a12 = value
        a13 = value
        a14 = value
        a15 = value
        if state.second:
            return value
    return value

async def sibling_manager_reset(first, second, state, value):
    async with first:
        if state.first:
            return value
        if state.second:
            return value
    async with second:
        if state.third:
            return value
    return value

async def nested_manager_restore(outer, inner, state, value):
    async with outer:
        if state.first:
            return value
        async with inner:
            if state.inner_first:
                return value
            if state.inner_second:
                return value
        if state.second:
            return value
    return value

async def nested_manager_inner_single(outer, inner, state, value):
    async with outer:
        if state.first:
            return value
        async with inner:
            if state.inner_first:
                return value
        if state.second:
            return value
    return value
";
    let parsed = parse_module(source).unwrap();
    let module = module_compiler(source, &parsed)
        .compile_module(parsed.suite())
        .unwrap();

    for (name, expected) in [
        (
            "hazardous_method_ifexp_arg",
            &[true, true, false, false][..],
        ),
        ("later_nested_if_terminal", &[true, false, false, false][..]),
        ("cap_16", &[true, false, false][..]),
        ("cap_17", &[true, false, false][..]),
        (
            "sibling_manager_reset",
            &[true, false, true, false, false][..],
        ),
        (
            "nested_manager_restore",
            &[true, true, false, false, false, false][..],
        ),
        (
            "nested_manager_inner_single",
            &[true, true, false, false, false][..],
        ),
    ] {
        let function = module
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) if code.name == name => Some(code),
                _ => None,
            })
            .unwrap_or_else(|| panic!("module should contain {name}"));
        let mut ranges = Vec::new();
        let mut index = 0;
        while index < function.exception_table.len() {
            let start = read_exception_varint(&function.exception_table, &mut index);
            let length = read_exception_varint(&function.exception_table, &mut index);
            read_exception_varint(&function.exception_table, &mut index);
            read_exception_varint(&function.exception_table, &mut index);
            ranges.push(start..start + length);
        }
        let marker_owners = function
            .bytecode
            .as_chunks::<2>()
            .0
            .iter()
            .enumerate()
            .filter(|(_, instruction)| instruction[0] == NOT_TAKEN.code())
            .map(|(offset, _)| {
                let offset = u32::try_from(offset).unwrap();
                ranges.iter().any(|range| range.contains(&offset))
            })
            .collect::<Vec<_>>();

        assert_eq!(
            marker_owners, expected,
            "unexpected marker owners for {name}"
        );
    }
}

#[test]
fn multiline_comprehension_filter_markers_respect_owner_boundaries() {
    let source = r"def multiline_set(values, suffix):
    return {
        item
        for item in values
        if suffix is None or item.name.endswith(suffix)
    }

def multiline_dict(values, suffix):
    return {
        item.name: item
        for item in values
        if suffix is None or item.name.endswith(suffix)
    }

def multiline_try(values, suffix):
    try:
        return [
            item
            for item in values
            if suffix is None or item.name.endswith(suffix)
        ]
    except Exception:
        return []

def multiline_with(values, suffix, manager):
    with manager:
        return [
            item
            for item in values
            if suffix is None or item.name.endswith(suffix)
        ]

def multiline_is_not(values, suffix):
    return [
        item
        for item in values
        if suffix is not None or item.name.endswith(suffix)
    ]

def multiline_trailing_truth(values, suffix):
    return [
        item
        for item in values
        if suffix is None or item.name
    ]

def multiline_leading_named(values, suffix):
    return [
        item
        for item in values
        if (selected := suffix.value) is None or item.name in selected
    ]

def multiline_leading_subscript(values, suffix):
    return [
        item
        for item in values
        if suffix[0] is None or item.name in suffix[0]
    ]

def multiline_three_arm(values, suffix, backup):
    return [
        item
        for item in values
        if suffix is None or backup is None or item.name in suffix
    ]

def multiline_second_filter(values, suffix, ready):
    return [
        item
        for item in values
        if ready(item)
        if suffix is None or item.name.endswith(suffix)
    ]

def multiline_nested_comprehension_filter(groups, suffix):
    return [
        item
        for item in groups
        if suffix is None
        or [value for value in item.values if value.name.endswith(suffix)]
    ]

def multiline_singleton_after_outer_filter(values, suffix, ready):
    return [
        (item, extra)
        for item in values
        if ready(item)
        for extra in (item,)
        if suffix is None or extra.name.endswith(suffix)
    ]

def unfiltered_nested_inner(a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15, groups, array):
    return [
        [value if replacement is None or array.kind == 'O' else replacement for value, replacement in group]
        for group in groups
    ]

def unfiltered_try(a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15, values, replacements, array):
    try:
        return [
            value if replacement is None or array.kind == 'O' else replacement
            for value, replacement in zip(values, replacements, strict=True)
        ]
    except Exception:
        return []

def unfiltered_finally(a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15, values, replacements, array, cleanup):
    try:
        return [
            value if replacement is None or array.kind == 'O' else replacement
            for value, replacement in zip(values, replacements, strict=True)
        ]
    finally:
        cleanup()

def unfiltered_with(a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15, values, replacements, array, manager):
    with manager:
        return [
            value if replacement is None or array.kind == 'O' else replacement
            for value, replacement in zip(values, replacements, strict=True)
        ]
";
    let parsed = parse_module(source).unwrap();
    let module = module_compiler(source, &parsed)
        .compile_module(parsed.suite())
        .unwrap();

    for (name, expected) in [
        ("multiline_set", &[true, false][..]),
        ("multiline_dict", &[true, false][..]),
        ("multiline_try", &[true, false, true][..]),
        ("multiline_with", &[true, false, true][..]),
        ("multiline_is_not", &[true, false][..]),
        ("multiline_trailing_truth", &[true, false][..]),
        ("multiline_leading_named", &[true, false][..]),
        ("multiline_leading_subscript", &[true, false][..]),
        ("multiline_three_arm", &[true, true, false][..]),
        ("multiline_second_filter", &[false, true, false][..]),
        (
            "multiline_nested_comprehension_filter",
            &[true, false, false][..],
        ),
        (
            "multiline_singleton_after_outer_filter",
            &[false, true, false][..],
        ),
        ("unfiltered_nested_inner", &[true, true][..]),
        ("unfiltered_try", &[true, true, true][..]),
        ("unfiltered_finally", &[true, true][..]),
        ("unfiltered_with", &[true, true, true][..]),
    ] {
        let function = module
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) if code.name == name => Some(code),
                _ => None,
            })
            .unwrap_or_else(|| panic!("module should contain {name}"));
        let mut ranges = Vec::new();
        let mut index = 0;
        while index < function.exception_table.len() {
            let start = read_exception_varint(&function.exception_table, &mut index);
            let length = read_exception_varint(&function.exception_table, &mut index);
            read_exception_varint(&function.exception_table, &mut index);
            read_exception_varint(&function.exception_table, &mut index);
            ranges.push(start..start + length);
        }
        let marker_owners = function
            .bytecode
            .as_chunks::<2>()
            .0
            .iter()
            .enumerate()
            .filter(|(_, instruction)| instruction[0] == NOT_TAKEN.code())
            .map(|(offset, _)| {
                let offset = u32::try_from(offset).unwrap();
                ranges.iter().any(|range| range.contains(&offset))
            })
            .collect::<Vec<_>>();

        assert_eq!(
            marker_owners, expected,
            "unexpected marker owners for {name}"
        );
    }
}

#[test]
fn coroutine_iterator_assignment_conditions_respect_prefix_and_tail_boundaries() {
    let cases = [
        (
            "async def f(values, get_state, use):\n    for item in values:\n        state = get_state(item)\n        if state:\n            use(state)\n        use(item)\n",
            true,
        ),
        (
            "async def f(values, get_state, use):\n    for item in values:\n        state_dict = None\n        if state := get_state(item):\n            use(state)\n        use(state_dict)\n",
            true,
        ),
        (
            "async def f(values, get_state, use):\n    for item in values:\n        state = get_state(item)\n        state_dict = None\n        if state:\n            use(state)\n        use(state_dict)\n",
            true,
        ),
        (
            "async def f(values, get_state, use):\n    for item in values:\n        state = get_state(item)\n        if state:\n            use(state)\n",
            false,
        ),
        (
            "async def f(values, get_state, use):\n    for item in values:\n        state = get_state(item)\n        state_dict = None\n        observed = False\n        if state:\n            use(state)\n        use(observed, state_dict)\n",
            false,
        ),
        (
            "async def f(values, get_state, use):\n    for item in values:\n        use(item)\n        state = get_state(item)\n        if state:\n            use(state)\n        use(item)\n",
            false,
        ),
        (
            "async def f(values, ready, get_state, use):\n    for item in values:\n        state = get_state(item) if ready(item) else None\n        if state:\n            use(state)\n        use(item)\n",
            false,
        ),
        (
            "async def f(values, get_state, use):\n    for item in values:\n        state = [get_state(value) for value in item]\n        if state:\n            use(state)\n        use(item)\n",
            false,
        ),
        (
            "async def f(values, get_state, use):\n    for item in values:\n        state_dict = None\n        if state := [get_state(value) for value in item]:\n            use(state)\n        use(state_dict)\n",
            false,
        ),
        (
            "async def f(values, get_state, use):\n    for item in values:\n        first = second = get_state(item)\n        if first:\n            use(first)\n        use(item)\n",
            false,
        ),
        (
            "async def f(values, get_state, use):\n    for item in values:\n        state = get_state(item)\n        if state.ready:\n            use(state)\n        use(item)\n",
            false,
        ),
        (
            "async def f(values, get_state, use):\n    for item in values:\n        state = get_state(item)\n        if not state:\n            use(item)\n        use(item)\n",
            false,
        ),
    ];

    for (source, expected) in cases {
        let parsed = parse_module(source).unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!("expected one function definition");
        };
        let [Stmt::For(statement)] = function.body.as_slice() else {
            panic!("expected one iterator loop");
        };
        assert_eq!(
            first_coroutine_iterator_assignment_condition(&statement.body).is_some(),
            expected,
            "unexpected iterator assignment condition for {source}"
        );
    }
}

#[test]
fn coroutine_iterator_condition_markers_respect_region_and_scope_boundaries() {
    let source = r"async def protected_loop(values, get_state, use):
    try:
        for item in values:
            state = get_state(item)
            if state:
                use(state)
            use(item)
    except Exception:
        use(None)

async def with_loop(values, get_state, use, manager):
    with manager:
        for item in values:
            state = get_state(item)
            if state:
                use(state)
            use(item)

async def async_with_loop(values, get_state, use, manager):
    async with manager:
        for item in values:
            state = get_state(item)
            if state:
                use(state)
            use(item)

async def handler_loop(values, get_state, use):
    try:
        use(None)
    except Exception:
        for item in values:
            state = get_state(item)
            if state:
                use(state)
            use(item)

async def finally_loop(values, get_state, use):
    try:
        use(None)
    finally:
        for item in values:
            state = get_state(item)
            if state:
                use(state)
            use(item)

async def async_for_control(values, get_state, use):
    async for item in values:
        state = get_state(item)
        if state:
            use(state)
        use(item)

def generator_control(values, get_state, use):
    for item in values:
        state = get_state(item)
        if state:
            use(state)
        yield item

async def async_generator_control(values, get_state, use):
    for item in values:
        state = get_state(item)
        if state:
            use(state)
        yield item
";
    let parsed = parse_module(source).unwrap();
    let module = module_compiler(source, &parsed)
        .compile_module(parsed.suite())
        .unwrap();

    for (name, expected) in [
        ("protected_loop", &[true, true][..]),
        ("with_loop", &[true, true][..]),
        ("async_with_loop", &[true, false][..]),
        ("handler_loop", &[true, true][..]),
        ("finally_loop", &[true, true][..]),
        ("async_for_control", &[true, true][..]),
        ("generator_control", &[true][..]),
        ("async_generator_control", &[true][..]),
    ] {
        let function = module
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) if code.name == name => Some(code),
                _ => None,
            })
            .unwrap_or_else(|| panic!("module should contain {name}"));
        let mut ranges = Vec::new();
        let mut index = 0;
        while index < function.exception_table.len() {
            let start = read_exception_varint(&function.exception_table, &mut index);
            let length = read_exception_varint(&function.exception_table, &mut index);
            read_exception_varint(&function.exception_table, &mut index);
            read_exception_varint(&function.exception_table, &mut index);
            ranges.push(start..start + length);
        }
        let marker_owners = function
            .bytecode
            .as_chunks::<2>()
            .0
            .iter()
            .enumerate()
            .filter(|(_, instruction)| instruction[0] == NOT_TAKEN.code())
            .map(|(offset, _)| {
                let offset = u32::try_from(offset).unwrap();
                ranges.iter().any(|range| range.contains(&offset))
            })
            .collect::<Vec<_>>();

        assert_eq!(
            marker_owners, expected,
            "unexpected iterator marker owners for {name}"
        );
    }
}

#[test]
fn chained_generator_filters_keep_their_first_comparison_marker_owned() {
    let source = r"def integer_filter(items):
    return (item for item in items if not (0 < item < 10))

def surrogate_filter(items):
    return (item for item in items if not ('\ud800' <= item <= '\udfff'))
";
    let parsed = parse_module(source).unwrap();
    let module = module_compiler(source, &parsed)
        .compile_module(parsed.suite())
        .unwrap();

    for name in ["integer_filter", "surrogate_filter"] {
        let function = module
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) if code.name == name => Some(code),
                _ => None,
            })
            .unwrap_or_else(|| panic!("module should contain {name}"));
        let generator = function
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) if code.name == "<genexpr>" => Some(code),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{name} should contain a generator expression"));

        let mut ranges = Vec::new();
        let mut index = 0;
        while index < generator.exception_table.len() {
            let start = read_exception_varint(&generator.exception_table, &mut index);
            let length = read_exception_varint(&generator.exception_table, &mut index);
            read_exception_varint(&generator.exception_table, &mut index);
            read_exception_varint(&generator.exception_table, &mut index);
            ranges.push(start..start + length);
        }
        let marker_owners = generator
            .bytecode
            .as_chunks::<2>()
            .0
            .iter()
            .enumerate()
            .filter(|(_, instruction)| instruction[0] == NOT_TAKEN.code())
            .map(|(offset, _)| {
                let offset = u32::try_from(offset).unwrap();
                ranges.iter().any(|range| range.contains(&offset))
            })
            .collect::<Vec<_>>();

        assert_eq!(marker_owners, [true, false], "unexpected owners for {name}");
    }
}

#[test]
fn copied_returns_omit_nops_at_original_conditional_joins() {
    let source = r#"import json

def event_shape(event, issues, sink):
    kind = event.get("kind")
    if kind == "request":
        if event.get("accepted"):
            if event.get("served"):
                sink(issues)
            else:
                if not event.get("first"):
                    sink(issues)
                if event.get("first") and event.get("first") == event.get("last"):
                    for field in ("first", "last"):
                        if field not in issues:
                            issues.append(field)
        elif event.get("requested"):
            sink(issues)
        elif event.get("observed"):
            if "context" not in issues:
                issues.append("context")
    elif kind == "attempt":
        sink(issues)
    return issues

def validate_payload(state, text, errors):
    if state == "empty":
        if text:
            errors.append("unexpected text")
    elif state in ("partial", "complete"):
        if state == "complete":
            if not isinstance(text, str) or not text.strip():
                errors.append("missing text")
            else:
                try:
                    decoded = json.loads(text)
                except json.JSONDecodeError:
                    decoded = None
                if not isinstance(decoded, dict):
                    errors.append("expected an object")
        elif text not in ("", None):
            errors.append("unexpected partial text")
    elif state in ("invalid", "mixed"):
        if text not in ("", None):
            errors.append("unexpected invalid text")
    return state
"#;
    let parsed = parse_module(source).unwrap();
    let module = module_compiler(source, &parsed)
        .compile_module(parsed.suite())
        .unwrap();

    for (name, expected_nops) in [("event_shape", 0), ("validate_payload", 1)] {
        let function = module
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) if code.name == name => Some(code),
                _ => None,
            })
            .unwrap_or_else(|| panic!("module should contain {name}"));
        let nops = function
            .bytecode
            .as_chunks::<2>()
            .0
            .iter()
            .filter(|instruction| instruction[0] == NOP.code())
            .count();

        // The decode function retains its try-statement marker. Neither copied
        // return adds a line event at the preceding conditional's shared join.
        assert_eq!(nops, expected_nops, "unexpected NOP count for {name}");
    }
}

#[test]
fn copied_finally_exits_keep_statement_and_assertion_nops() {
    let source = r"def boolean_condition(manager, left, right):
    with manager:
        try:
            assert body()
        finally:
            if left and right:
                cleanup()

def conditional_expression(manager, condition, left, right):
    with manager:
        try:
            assert body()
        finally:
            if left if condition else right:
                cleanup()

def negated_boolean_condition(manager, left, right):
    with manager:
        try:
            assert body()
        finally:
            if not (left or right):
                cleanup()
";
    let parsed = parse_module(source).unwrap();
    let module = module_compiler(source, &parsed)
        .compile_module(parsed.suite())
        .unwrap();

    for name in [
        "boolean_condition",
        "conditional_expression",
        "negated_boolean_condition",
    ] {
        let function = module
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) if code.name == name => Some(code),
                _ => None,
            })
            .unwrap_or_else(|| panic!("module should contain {name}"));
        let nops = function
            .bytecode
            .as_chunks::<2>()
            .0
            .iter()
            .filter(|instruction| instruction[0] == NOP.code())
            .count();

        // CPython keeps the try-statement and assertion markers; the copied conditional
        // fallthrough reaches an unlocated exit and does not retain another NOP.
        assert_eq!(nops, 2, "unexpected NOP count for {name}");
    }
}

#[test]
fn copied_finally_exits_drop_simple_condition_nops() {
    let source = r"def expression_compare(manager, flag):
    with manager:
        try:
            work()
        finally:
            if flag is True:
                cleanup()

def assignment_compare(manager, flag):
    with manager:
        try:
            value = work()
        finally:
            if flag is None:
                cleanup()

def assert_two_cleanups(manager, value):
    with manager:
        try:
            assert value
        finally:
            if value is not None:
                stop()
                cleanup()

def conditional_name(manager, flag, other):
    with manager:
        try:
            if other:
                work()
        finally:
            if flag:
                cleanup()

def loop_name(manager, flag, items):
    with manager:
        try:
            for item in items:
                work(item)
        finally:
            if flag:
                cleanup()
";
    let parsed = parse_module(source).unwrap();
    let module = module_compiler(source, &parsed)
        .compile_module(parsed.suite())
        .unwrap();

    for (name, expected_nops) in [
        ("expression_compare", 1),
        ("assignment_compare", 1),
        ("assert_two_cleanups", 2),
        ("conditional_name", 1),
        ("loop_name", 1),
    ] {
        let function = module
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) if code.name == name => Some(code),
                _ => None,
            })
            .unwrap_or_else(|| panic!("module should contain {name}"));
        let nops = function
            .bytecode
            .as_chunks::<2>()
            .0
            .iter()
            .filter(|instruction| instruction[0] == NOP.code())
            .count();

        assert_eq!(nops, expected_nops, "unexpected NOP count for {name}");
    }
}

#[test]
fn protected_with_breaks_keep_loop_exit_nops() {
    let source = r"def protected_break(manager, active, stop):
    with manager:
        try:
            while active:
                if stop:
                    break
                step()
        except Error:
            cleanup()

def nested_finally_break(manager, active, stop):
    with manager:
        while True:
            try:
                while active:
                    if stop:
                        break
                    step()
            finally:
                cleanup()
";
    let parsed = parse_module(source).unwrap();
    let module = module_compiler(source, &parsed)
        .compile_module(parsed.suite())
        .unwrap();

    for (name, expected_nops) in [("protected_break", 2), ("nested_finally_break", 2)] {
        let function = module
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) if code.name == name => Some(code),
                _ => None,
            })
            .unwrap_or_else(|| panic!("module should contain {name}"));
        let nops = function
            .bytecode
            .as_chunks::<2>()
            .0
            .iter()
            .filter(|instruction| instruction[0] == NOP.code())
            .count();

        assert_eq!(nops, expected_nops, "unexpected NOP count for {name}");
    }
}

#[test]
fn comparison_markers_distinguish_forward_and_threaded_loop_fallthroughs() {
    let source = r"def f(container1, container2):
    a_idx, b_idx = 0, 0
    max_a, max_b = len(container1), len(container2)
    while a_idx < max_a and b_idx < max_b:
        if container1[a_idx] == container2[b_idx]:
            yield (container1[a_idx], container2[b_idx])
            a_idx += 1
            b_idx += 1
        else:
            while container1[a_idx] < container2[b_idx]:
                a_idx += 1
            while container1[a_idx] > container2[b_idx]:
                b_idx += 1
";
    let parsed = parse_module(source).unwrap();
    let module = module_compiler(source, &parsed)
        .compile_module(parsed.suite())
        .unwrap();
    let function = module
        .constants
        .iter()
        .find_map(|constant| match constant {
            Constant::Code(code) if code.name == "f" => Some(code),
            _ => None,
        })
        .expect("module should contain f");

    let mut ranges = Vec::new();
    let mut index = 0;
    while index < function.exception_table.len() {
        let start = read_exception_varint(&function.exception_table, &mut index);
        let length = read_exception_varint(&function.exception_table, &mut index);
        read_exception_varint(&function.exception_table, &mut index);
        read_exception_varint(&function.exception_table, &mut index);
        ranges.push(start..start + length);
    }
    let marker_owners = function
        .bytecode
        .as_chunks::<2>()
        .0
        .iter()
        .enumerate()
        .filter(|(_, instruction)| instruction[0] == NOT_TAKEN.code())
        .map(|(offset, _)| {
            let offset = u32::try_from(offset).unwrap();
            ranges.iter().any(|range| range.contains(&offset))
        })
        .collect::<Vec<_>>();

    // CPython appends the first four markers to compacted forward-jump blocks, reusing their
    // stale generator owner. The final nested while exit threads to the outer backedge first,
    // so jump normalization creates its marker in a fresh, ownerless block.
    assert_eq!(marker_owners, [true, true, true, true, false]);
}

#[test]
fn cleans_docstrings_like_cpython() {
    assert_eq!(
        clean_doc("  first\n        second\n      third\n"),
        "first\n  second\nthird\n"
    );
    assert_eq!(clean_doc("\tfirst\n\t\tsecond"), "first\nsecond");
}

#[test]
fn erased_comprehension_branch_assertions_preserve_original_return_donors() {
    for scope in ["def", "async def"] {
        for comprehension in [
            "[item for item in items]",
            "{item for item in items}",
            "{item: item.value for item in items}",
        ] {
            for (left_prefix, right_prefix) in [
                ("", ""),
                ("        prefix = [item for item in items]\n", ""),
                ("", "        prefix = [item for item in items]\n"),
                (
                    "        prefix = [item for item in items]\n",
                    "        prefix = [item for item in items]\n",
                ),
                (
                    "        prefix = {item for item in items if item}\n",
                    "        prefix = {item: item.value for item in items}\n",
                ),
                (
                    "        prefix = [item for group in items if group for item in group if item]\n",
                    "        prefix = [item for group in items for item in group]\n",
                ),
            ] {
                let source = format!(
                    "{scope} f(flag, items):\n    if flag:\n{left_prefix}        out = {comprehension}\n        assert out\n    else:\n{right_prefix}        out = {comprehension}\n        assert out\n    assert flag\n"
                );
                let neighbor = source.replace("        assert out", "");
                for python_version in [
                    crate::PythonVersion::Py312,
                    crate::PythonVersion::Py313,
                    crate::PythonVersion::Py314,
                    crate::PythonVersion::Py315,
                ] {
                    for optimization_level in [
                        crate::OptimizationLevel::Zero,
                        crate::OptimizationLevel::One,
                        crate::OptimizationLevel::Two,
                    ] {
                        let options = crate::CompileOptions {
                            python_version,
                            optimization_level,
                            ..crate::CompileOptions::default()
                        };
                        let compile = |source: &str| {
                            crate::compile_with_options(source, "comprehension_footer.py", &options)
                                .unwrap()
                                .marshal()
                        };
                        let actual = compile(&source);
                        let expected = compile(&neighbor);
                        if optimization_level == crate::OptimizationLevel::Zero {
                            assert_ne!(actual, expected);
                        } else {
                            assert_eq!(
                                actual, expected,
                                "{python_version:?} {optimization_level:?}: {source}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn uncopied_assertion_backedges_keep_source_positions() {
    use crate::target::opcodes::JUMP_BACKWARD;

    // Decode only record widths and whether a position exists. This invariant also
    // holds when future lowering duplicates the backedge onto each success edge.
    fn located_words(table: &[u8]) -> Vec<bool> {
        let mut result = Vec::new();
        let mut cursor = 0;
        while cursor < table.len() {
            let header = table[cursor];
            assert_ne!(header & 0x80, 0);
            cursor += 1;
            let kind = (header >> 3) & 15;
            result.extend(std::iter::repeat_n(kind != 15, usize::from(header & 7) + 1));
            match kind {
                0..=9 => cursor += 1,
                10..=12 => cursor += 2,
                13 | 14 => {
                    for _ in 0..if kind == 13 { 1 } else { 4 } {
                        loop {
                            let value = table[cursor];
                            cursor += 1;
                            if value & 0x40 == 0 {
                                break;
                            }
                        }
                    }
                }
                15 => {}
                _ => unreachable!(),
            }
        }
        assert_eq!(cursor, table.len());
        result
    }

    for condition in [
        "left(value) or right(value)",
        "not (left(value) and right(value))",
    ] {
        let source = format!(
            "def f(items, left, right):\n    for value in items:\n        try:\n            assert {condition}\n        except Exception:\n            raise\n    return items\n"
        );
        let parsed = parse_module(&source).unwrap();
        let module = module_compiler(&source, &parsed)
            .compile_module(parsed.suite())
            .unwrap();
        let function = module
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) if code.name == "f" => Some(code),
                _ => None,
            })
            .unwrap();
        let positions = located_words(&function.line_table);
        assert_eq!(positions.len() * 2, function.bytecode.len());
        let backedges = function
            .bytecode
            .as_chunks::<2>()
            .0
            .iter()
            .enumerate()
            .filter(|(_, word)| word[0] == JUMP_BACKWARD.code())
            .collect::<Vec<_>>();
        assert!(!backedges.is_empty());
        for (offset, _) in backedges {
            assert!(
                positions[offset],
                "missing backedge location for {condition}"
            );
        }
    }
}

mod iterator_width_return_cleanup {
    use crate::code_object::{CodeObject, Constant};
    use crate::target::opcodes::{CACHE, CALL, POP_TOP, RETURN_VALUE, SWAP};
    use crate::{CompileOptions, PythonVersion, compile_with_options};

    fn function(source: &str, python_version: PythonVersion) -> CodeObject {
        compile_with_options(
            source,
            "iterator_return.py",
            &CompileOptions {
                python_version,
                ..CompileOptions::default()
            },
        )
        .unwrap()
        .code
        .constants
        .into_iter()
        .find_map(|constant| match constant {
            Constant::Code(code) => Some(*code),
            _ => None,
        })
        .unwrap()
    }

    fn assert_native_exception_tables(source: &str, python314: &[u8], python315: &[u8]) {
        // Captured from the pinned CPython 3.14.5 and 3.15.0rc2 oracles.
        for (version, expected) in [
            (PythonVersion::Py314, python314),
            (PythonVersion::Py315, python315),
        ] {
            assert_eq!(
                function(source, version).exception_table,
                expected,
                "{version:?}"
            );
        }
    }

    #[test]
    fn iterators_unwind_on_their_respective_sides_of_manager_exit() {
        let source = r"def f(xs, ys, manager):
    try:
        for outer in xs:
            with manager:
                for inner in ys:
                    if inner:
                        return
    finally:
        finish()
";
        for (version, slots) in [(PythonVersion::Py314, 1), (PythonVersion::Py315, 2)] {
            for generator in [false, true] {
                let source = if generator {
                    source.replacen("    try:", "    yield 0\n    try:", 1)
                } else {
                    source.to_string()
                };
                let code = function(&source, version);
                let words = code
                    .bytecode
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .copied()
                    .filter(|word| word[0] != CACHE.code_for(version))
                    .collect::<Vec<_>>();
                let exit = words
                    .iter()
                    .position(|word| *word == [CALL.code_for(version), 3])
                    .unwrap();
                // The inner iterator leaves before the three __exit__ arguments.
                assert!(
                    words[exit - 3 - slots..exit - 3]
                        .iter()
                        .all(|word| word[0] == POP_TOP.code_for(version))
                );
                // The call result leaves first, then the surrounding iterator.
                assert!(
                    words[exit + 1..exit + 2 + slots]
                        .iter()
                        .all(|word| word[0] == POP_TOP.code_for(version))
                );
            }
        }
    }

    #[test]
    fn nested_handler_iterator_cleanup_retains_the_outer_exception_owner() {
        assert_native_exception_tables(
            r"def f(items):
    try:
        fail()
    except ValueError:
        for item in items:
            try:
                fail()
            except ValueError:
                return None
",
            &[
                0x82, 0x0a, 0x0e, 0x00, 0x8e, 0x10, 0x41, 0x05, 0x03, 0x9f, 0x0a, 0x2b, 0x04, 0xa9,
                0x02, 0x41, 0x05, 0x03, 0xab, 0x0b, 0x3c, 0x07, 0xb6, 0x02, 0x41, 0x05, 0x03, 0xbb,
                0x01, 0x3c, 0x07, 0xbc, 0x05, 0x41, 0x05, 0x03, 0xc1, 0x04, 0x01, 0x41, 0x05, 0x03,
            ],
            &[
                0x83, 0x0a, 0x0f, 0x00, 0x8f, 0x09, 0x41, 0x08, 0x03, 0x99, 0x07, 0x41, 0x08, 0x03,
                0xa1, 0x0a, 0x2d, 0x06, 0xab, 0x02, 0x41, 0x08, 0x03, 0xad, 0x09, 0x3f, 0x09, 0xb7,
                0x01, 0x3f, 0x09, 0xb8, 0x03, 0x41, 0x08, 0x03, 0xbe, 0x01, 0x3f, 0x09, 0xbf, 0x05,
                0x41, 0x08, 0x03, 0xc1, 0x07, 0x01, 0x41, 0x08, 0x03,
            ],
        );
        assert_native_exception_tables(
            r"def f(items):
    try:
        fail()
    except ValueError:
        for item in items:
            try:
                fail()
            except ValueError:
                if condition():
                    return value()
                continue
",
            &[
                0x82, 0x0a, 0x0e, 0x00, 0x8e, 0x10, 0x41, 0x23, 0x03, 0x9f, 0x0a, 0x2b, 0x04, 0xa9,
                0x02, 0x41, 0x23, 0x03, 0xab, 0x25, 0x41, 0x1a, 0x07, 0xc1, 0x10, 0x04, 0x41, 0x23,
                0x03, 0xc1, 0x16, 0x03, 0x41, 0x23, 0x03, 0xc1, 0x19, 0x01, 0x41, 0x1a, 0x07, 0xc1,
                0x1a, 0x05, 0x41, 0x23, 0x03, 0xc1, 0x22, 0x01, 0x41, 0x23, 0x03,
            ],
            &[
                0x83, 0x0a, 0x0f, 0x00, 0x8f, 0x09, 0x41, 0x26, 0x03, 0x99, 0x07, 0x41, 0x26, 0x03,
                0xa1, 0x0a, 0x2d, 0x06, 0xab, 0x02, 0x41, 0x26, 0x03, 0xad, 0x09, 0x41, 0x1d, 0x09,
                0xb7, 0x10, 0x41, 0x1d, 0x09, 0xc1, 0x08, 0x0a, 0x41, 0x1d, 0x09, 0xc1, 0x12, 0x05,
                0x41, 0x26, 0x03, 0xc1, 0x19, 0x03, 0x41, 0x26, 0x03, 0xc1, 0x1c, 0x01, 0x41, 0x1d,
                0x09, 0xc1, 0x1d, 0x05, 0x41, 0x26, 0x03, 0xc1, 0x25, 0x01, 0x41, 0x26, 0x03,
            ],
        );
    }

    #[test]
    fn async_generator_finalizer_retains_the_intervening_handler_owner() {
        assert_native_exception_tables(
            r"async def f(items):
    try:
        for item in items:
            try:
                while True:
                    try:
                        yield item
                        return
                    except BaseException:
                        raise
                    finally:
                        cleanup()
            except ValueError:
                raise
    except BaseException:
        raise
    finally:
        finish()
",
            &[
                0x82, 0x01, 0x42, 0x06, 0x01, 0x84, 0x05, 0x41, 0x27, 0x00, 0x8a, 0x01, 0x41, 0x17,
                0x02, 0x8c, 0x05, 0x38, 0x02, 0x92, 0x0a, 0x41, 0x17, 0x02, 0x9c, 0x01, 0x41, 0x27,
                0x00, 0x9d, 0x0c, 0x42, 0x06, 0x01, 0xa9, 0x02, 0x41, 0x27, 0x00, 0xac, 0x0c, 0x42,
                0x06, 0x01, 0xb8, 0x0d, 0x41, 0x05, 0x05, 0xc1, 0x05, 0x03, 0x41, 0x08, 0x02, 0xc1,
                0x08, 0x0c, 0x41, 0x14, 0x05, 0xc1, 0x14, 0x03, 0x41, 0x17, 0x02, 0xc1, 0x17, 0x0d,
                0x41, 0x24, 0x05, 0xc1, 0x24, 0x03, 0x41, 0x27, 0x00, 0xc1, 0x27, 0x0d, 0x41, 0x34,
                0x03, 0xc1, 0x34, 0x03, 0x41, 0x37, 0x00, 0xc1, 0x37, 0x0c, 0x42, 0x03, 0x03, 0xc2,
                0x03, 0x03, 0x42, 0x06, 0x01,
            ],
            &[
                0x82, 0x02, 0x42, 0x0a, 0x01, 0x85, 0x06, 0x41, 0x2b, 0x00, 0x8c, 0x01, 0x41, 0x1b,
                0x04, 0x8e, 0x06, 0x3c, 0x04, 0x95, 0x0a, 0x41, 0x1b, 0x04, 0x9f, 0x02, 0x41, 0x2b,
                0x00, 0xa1, 0x0c, 0x42, 0x0a, 0x01, 0xad, 0x02, 0x41, 0x2b, 0x00, 0xb0, 0x0c, 0x42,
                0x0a, 0x01, 0xbc, 0x09, 0x41, 0x09, 0x07, 0xc1, 0x06, 0x03, 0x41, 0x09, 0x07, 0xc1,
                0x09, 0x03, 0x41, 0x0c, 0x04, 0xc1, 0x0c, 0x0c, 0x41, 0x18, 0x07, 0xc1, 0x18, 0x03,
                0x41, 0x1b, 0x04, 0xc1, 0x1b, 0x09, 0x41, 0x28, 0x07, 0xc1, 0x25, 0x03, 0x41, 0x28,
                0x07, 0xc1, 0x28, 0x03, 0x41, 0x2b, 0x00, 0xc1, 0x2b, 0x09, 0x41, 0x38, 0x03, 0xc1,
                0x35, 0x03, 0x41, 0x38, 0x03, 0xc1, 0x38, 0x03, 0x41, 0x3b, 0x00, 0xc1, 0x3b, 0x0c,
                0x42, 0x07, 0x03, 0xc2, 0x07, 0x03, 0x42, 0x0a, 0x01,
            ],
        );
    }
    #[test]
    fn awaited_finalizer_copies_iterator_return_across_outer_try_boundary() {
        let source = r"async def f(items, ready):
    try:
        for value in items:
            try:
                return value
            finally:
                if ready:
                    try:
                        await cleanup()
                    except Exception as error:
                        recover(error)
    except Exception:
        raise
";
        assert_native_exception_tables(
            source,
            &[
                0x82, 0x01, 0x42, 0x2c, 0x01, 0x84, 0x05, 0x42, 0x1c, 0x00, 0x8a, 0x01, 0x41, 0x17,
                0x02, 0x8b, 0x08, 0x42, 0x1c, 0x00, 0x94, 0x0d, 0x32, 0x04, 0xa1, 0x01, 0x30, 0x08,
                0xa2, 0x04, 0x32, 0x04, 0xa6, 0x02, 0x42, 0x1c, 0x00, 0xa8, 0x01, 0x42, 0x2c, 0x01,
                0xa9, 0x02, 0x42, 0x1c, 0x00, 0xab, 0x01, 0x42, 0x2c, 0x01, 0xac, 0x02, 0x42, 0x1c,
                0x00, 0xae, 0x02, 0x42, 0x2c, 0x01, 0xb0, 0x01, 0x32, 0x04, 0xb2, 0x0b, 0x41, 0x14,
                0x07, 0xbd, 0x0b, 0x41, 0x0f, 0x07, 0xc1, 0x08, 0x06, 0x42, 0x1c, 0x00, 0xc1, 0x0e,
                0x01, 0x42, 0x2c, 0x01, 0xc1, 0x0f, 0x05, 0x41, 0x14, 0x07, 0xc1, 0x14, 0x03, 0x42,
                0x1c, 0x00, 0xc1, 0x17, 0x09, 0x42, 0x19, 0x05, 0xc1, 0x21, 0x0d, 0x41, 0x35, 0x06,
                0xc1, 0x2e, 0x01, 0x41, 0x31, 0x0a, 0xc1, 0x2f, 0x05, 0x41, 0x35, 0x06, 0xc1, 0x34,
                0x01, 0x42, 0x19, 0x05, 0xc1, 0x35, 0x0b, 0x42, 0x15, 0x09, 0xc2, 0x00, 0x0b, 0x42,
                0x10, 0x09, 0xc2, 0x0b, 0x05, 0x42, 0x19, 0x05, 0xc2, 0x10, 0x05, 0x42, 0x15, 0x09,
                0xc2, 0x15, 0x04, 0x42, 0x19, 0x05, 0xc2, 0x19, 0x03, 0x42, 0x1c, 0x00, 0xc2, 0x1c,
                0x0d, 0x42, 0x29, 0x03, 0xc2, 0x29, 0x03, 0x42, 0x2c, 0x01,
            ],
            &[
                0x82, 0x02, 0x42, 0x35, 0x01, 0x85, 0x06, 0x42, 0x25, 0x00, 0x8c, 0x01, 0x41, 0x1e,
                0x04, 0x8d, 0x07, 0x42, 0x25, 0x00, 0x96, 0x0e, 0x38, 0x06, 0xa4, 0x01, 0x36, 0x0c,
                0xa5, 0x05, 0x38, 0x06, 0xaa, 0x03, 0x42, 0x25, 0x00, 0xad, 0x01, 0x42, 0x35, 0x01,
                0xae, 0x03, 0x42, 0x25, 0x00, 0xb1, 0x01, 0x42, 0x35, 0x01, 0xb2, 0x02, 0x42, 0x25,
                0x00, 0xb4, 0x02, 0x42, 0x35, 0x01, 0xb6, 0x01, 0x38, 0x06, 0xb8, 0x09, 0x41, 0x1b,
                0x09, 0xc1, 0x02, 0x01, 0x41, 0x1b, 0x09, 0xc1, 0x03, 0x0b, 0x41, 0x16, 0x09, 0xc1,
                0x0e, 0x07, 0x42, 0x25, 0x00, 0xc1, 0x15, 0x01, 0x42, 0x35, 0x01, 0xc1, 0x16, 0x05,
                0x41, 0x1b, 0x09, 0xc1, 0x1b, 0x03, 0x42, 0x25, 0x00, 0xc1, 0x1e, 0x08, 0x42, 0x22,
                0x07, 0xc1, 0x28, 0x0e, 0x41, 0x3e, 0x08, 0xc1, 0x36, 0x01, 0x41, 0x3a, 0x0e, 0xc1,
                0x37, 0x06, 0x41, 0x3e, 0x08, 0xc1, 0x3d, 0x01, 0x42, 0x22, 0x07, 0xc1, 0x3e, 0x09,
                0x42, 0x1e, 0x0b, 0xc2, 0x08, 0x01, 0x42, 0x1e, 0x0b, 0xc2, 0x09, 0x0b, 0x42, 0x19,
                0x0b, 0xc2, 0x14, 0x05, 0x42, 0x22, 0x07, 0xc2, 0x19, 0x05, 0x42, 0x1e, 0x0b, 0xc2,
                0x1e, 0x04, 0x42, 0x22, 0x07, 0xc2, 0x22, 0x03, 0x42, 0x25, 0x00, 0xc2, 0x25, 0x09,
                0x42, 0x32, 0x03, 0xc2, 0x2f, 0x03, 0x42, 0x32, 0x03, 0xc2, 0x32, 0x03, 0x42, 0x35,
                0x01,
            ],
        );
        // Both pinned CPython versions give the await-success, conditional and
        // named-handler exits separate cleanup tails with their own locations.
        for (version, slots) in [(PythonVersion::Py314, 1), (PythonVersion::Py315, 2)] {
            let code = function(source, version);
            let words = code.bytecode.as_chunks::<2>().0;
            let mut tail = vec![[SWAP.code_for(version), slots + 1]];
            tail.extend(std::iter::repeat_n(
                [POP_TOP.code_for(version), 0],
                usize::from(slots),
            ));
            tail.push([RETURN_VALUE.code_for(version), 0]);
            assert_eq!(
                words
                    .windows(tail.len())
                    .filter(|words| *words == tail)
                    .count(),
                3,
                "{version:?}"
            );
        }
    }
}

#[test]
fn mapping_pattern_lengths_use_the_full_integer_value() {
    for size in [255_u32, 256] {
        let keys = (0..size)
            .map(|key| format!("{key}: _"))
            .collect::<Vec<_>>()
            .join(", ");
        let source = format!("match subject:\n    case {{{keys}}}:\n        pass\n");
        let module = crate::compile(&source, "mapping_pattern_length.py").unwrap();
        let mut instructions = module.code.bytecode.as_chunks::<2>().0.iter();
        instructions
            .find(|instruction| instruction[0] == GET_LEN.code())
            .unwrap();
        let load = instructions.next().unwrap();
        if size == 255 {
            assert_eq!(*load, [LOAD_SMALL_INT.code(), 255]);
        } else {
            assert_eq!(load[0], LOAD_CONST.code());
            assert!(matches!(
                module.code.constants[usize::from(load[1])],
                Constant::Int(value) if value == u64::from(size)
            ));
        }
    }
}

#[test]
fn class_explicit_globals_reset_definition_qualified_names_for_all_targets() {
    fn collect_qualified_names(
        code: &CodeObject,
        names: &mut Vec<String>,
        annotations: &mut Vec<String>,
    ) {
        for constant in &code.constants {
            if let Constant::Code(child) = constant {
                if child.name == "__annotate__" {
                    annotations.push(child.qualified_name.clone());
                } else if !child.name.starts_with('<') {
                    names.push(child.qualified_name.clone());
                }
                collect_qualified_names(child, names, annotations);
            }
        }
    }

    let source = include_str!("../../tests/fixtures/class_global_qualnames.py");
    for python_version in [
        crate::PythonVersion::Py312,
        crate::PythonVersion::Py313,
        crate::PythonVersion::Py314,
        crate::PythonVersion::Py315,
    ] {
        let module = crate::compile_with_options(
            source,
            "class_global_qualnames.py",
            &crate::CompileOptions {
                python_version,
                ..crate::CompileOptions::default()
            },
        )
        .unwrap();
        let mut names = Vec::new();
        let mut annotations = Vec::new();
        collect_qualified_names(&module.code, &mut names, &mut annotations);
        let expected_annotations: &[&str] = match python_version {
            crate::PythonVersion::Py312 | crate::PythonVersion::Py313 => &[],
            crate::PythonVersion::Py314 => &["Parent.__annotate__", "Parent.__annotate__"],
            crate::PythonVersion::Py315 => &[
                "Parent.function.__annotate__",
                "Parent.generic.__annotate__",
            ],
        };
        assert_eq!(annotations, expected_annotations, "{python_version:?}");
        assert_eq!(
            names,
            [
                "Parent",
                "function",
                "Child",
                "Child.method",
                "generic",
                "Parent.local",
                "Parent.Local",
                "Outer",
                "Outer.Inner",
                "nested_function",
                "NestedChild",
                "enclosing",
                "enclosing.<locals>.Parent",
                "enclosed_function",
                "enclosing.<locals>.Parent.local",
            ],
            "{python_version:?}",
        );
    }
}
