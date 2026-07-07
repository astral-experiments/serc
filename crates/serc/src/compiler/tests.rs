use ruff_python_ast::token::TokenKind;
use ruff_python_ast::{Expr, Stmt};
use ruff_python_parser::parse_module;
use ruff_text_size::Ranged;

use crate::assembler::SourceLocation;
use crate::code_object::Constant;
use crate::target::opcodes::{BUILD_TUPLE, LOAD_CONST, RAISE_VARARGS, RERAISE, RETURN_VALUE};

use super::{
    Compiler, CompilerStackEffect, FunctionPlan, NameId, NameInterner, ReraiseStackState,
    ScopeBodyFacts, clean_doc, compiler_stack_effect, validate_compiler_stack_effect,
};

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

#[test]
fn interns_adversarial_identifier_families_densely() {
    let mut interner = NameInterner::default();
    let names = (0..4096)
        .map(|index| format!("collision_prefix_{index:04}_collision_suffix"))
        .collect::<Vec<_>>();

    for (index, name) in names.iter().enumerate() {
        let id = interner.intern(name);
        assert_eq!(id, NameId(u32::try_from(index).unwrap()));
        assert_eq!(interner.resolve(id), name);
    }
    for (index, name) in names.iter().enumerate().rev() {
        assert_eq!(interner.intern(name), NameId(u32::try_from(index).unwrap()));
    }
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
    assert_eq!(source_order[0], NameId(0));
    assert_eq!(source_order[1], NameId(1));

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
    let source = r"class Container:
    __value = 1
    _Container__value = 2

    def method(self):
        return self.__value
";
    let parsed = parse_module(source).unwrap();
    let module = Compiler::module("example.py", source, parsed.tokens().clone())
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

    assert_eq!(
        class
            .names
            .iter()
            .filter(|name| name.as_str() == "_Container__value")
            .count(),
        1
    );
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
    let module = Compiler::module("example.py", source, parsed.tokens().clone())
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

    // A self-referencing assignment target precedes every load in its value, while a local that
    // is loaded before its eventual store keeps the position of that first load. A referenced
    // annotation-only local follows the same rule.
    assert_eq!(
        function.locals,
        ["target", "later", "earlier", "annotation_only"]
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
    let module = Compiler::module("example.py", source, parsed.tokens().clone())
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
    assert!(facts.has_simple_annotations);

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
fn compiles_a_simple_module() {
    let source = "answer = 6 * 7\nprint(answer)\n";
    let parsed = parse_module(source).unwrap();
    let code = Compiler::module("example.py", source, parsed.tokens().clone())
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
    Compiler::module("example.py", source, parsed.tokens().clone())
        .compile_module(parsed.suite())
        .unwrap();
}

#[test]
fn retains_parser_tokens_in_the_shared_compilation_context() {
    let source = "value = (\n    1  # comment\n)\n";
    let parsed = parse_module(source).unwrap();
    let expected = parsed.tokens().clone();
    let compiler = Compiler::module("example.py", source, parsed.tokens().clone());

    assert_eq!(compiler.context.tokens, expected);
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
    let compiler = Compiler::module("example.py", source, parsed.tokens().clone());
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
fn trailing_semicolon_locations_follow_token_boundaries() {
    let source = r#"first = "not; boundary"; second = 2
continued = 1 \
    ; third = 3
commented = 4  # ; not a boundary
"#;
    let parsed = parse_module(source).unwrap();
    let compiler = Compiler::module("example.py", source, parsed.tokens().clone());
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
        let compiler = Compiler::module("example.py", source, parsed.tokens().clone());
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
fn source_positions_use_utf8_byte_columns() {
    let source = "α = 1\nβγ = 2\n";
    let parsed = parse_module(source).unwrap();
    let compiler = Compiler::module("example.py", source, parsed.tokens().clone());

    assert_eq!(compiler.source_position(0), (1, 0));
    assert_eq!(compiler.source_position(2), (1, 2));
    assert_eq!(compiler.source_position(7), (2, 0));
    assert_eq!(compiler.source_position(9), (2, 2));
    assert_eq!(compiler.source_position(11), (2, 4));
    assert_eq!(compiler.line_number(11), 2);
}

#[test]
fn source_positions_clamp_to_eof() {
    let source = "first\né\n";
    let parsed = parse_module(source).unwrap();
    let compiler = Compiler::module("example.py", source, parsed.tokens().clone());
    let eof = u32::try_from(source.len()).unwrap();

    assert_eq!(compiler.source_position(eof), (3, 0));
    assert_eq!(compiler.source_position(u32::MAX), (3, 0));
    assert_eq!(compiler.line_number(u32::MAX), 3);
}

#[test]
fn source_positions_preserve_crlf_boundaries_and_eof() {
    let source = "one\r\ntwo";
    let parsed = parse_module(source).unwrap();
    let compiler = Compiler::module("example.py", source, parsed.tokens().clone());

    assert_eq!(compiler.source_position(2), (1, 2));
    assert_eq!(compiler.source_position(5), (2, 0));
    assert_eq!(compiler.source_position(8), (2, 3));
    assert_eq!(compiler.source_position(4), (1, 4));
    assert_eq!(compiler.source_position(u32::MAX), (2, 3));

    let source = "one\r\ntwo\r\n";
    let parsed = parse_module(source).unwrap();
    let compiler = Compiler::module("example.py", source, parsed.tokens().clone());

    assert_eq!(compiler.source_position(8), (2, 3));
    assert_eq!(compiler.source_position(9), (2, 4));
    assert_eq!(compiler.source_position(10), (3, 0));
    assert_eq!(compiler.source_position(5), (2, 0));
    assert_eq!(compiler.source_position(u32::MAX), (3, 0));
}

#[test]
fn cleans_docstrings_like_cpython() {
    assert_eq!(
        clean_doc("  first\n        second\n      third\n"),
        "first\n  second\nthird\n"
    );
    assert_eq!(clean_doc("\tfirst\n\t\tsecond"), "first\nsecond");
}
