use ruff_python_ast::token::TokenKind;
use ruff_python_ast::{Expr, ModModule, Stmt, StringLike};
use ruff_python_parser::{Parsed, parse_module};
use ruff_text_size::Ranged;

use crate::assembler::SourceLocation;
use crate::code_object::Constant;
use crate::target::code_flags::{CO_FUTURE_ANNOTATIONS, CO_FUTURE_BARRY_AS_BDFL};
use crate::target::opcodes::{
    BUILD_TUPLE, LOAD_CONST, NOP, NOT_TAKEN, RAISE_VARARGS, RERAISE, RETURN_VALUE,
};

use super::definition::clean_doc;
use super::scope::{FunctionPlan, NameId, NameInterner, ScopeBodyFacts};
use super::source::{
    debug_text_range, interpolated_string_content_range, strip_expression_comments,
    unparse_annotation,
};
use super::statement::{
    NestedIfExitLocation, nested_if_exit_location, try_propagates_protected_loop_tail_mode,
};
use super::symbol::{
    CompilerStackEffect, ReraiseStackState, compiler_stack_effect, validate_compiler_stack_effect,
};
use super::{Compiler, SHADOWED_SUPER_SENTINEL};

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
fn protected_loop_tail_mode_requires_assign_then_raise_handlers() {
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

fn module_compiler(source: &str, parsed: &Parsed<ModModule>) -> Compiler {
    Compiler::module(
        "example.py",
        source,
        parsed.tokens().clone(),
        parsed.suite(),
    )
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
    let plan = &compiler.context.module_plan;
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
    let expected = parsed.tokens().clone();
    let compiler = module_compiler(source, &parsed);

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
        assert_eq!(unparse_annotation(&assignment.annotation), expected);
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
        let module = parse_module(&format!("value: {literal}")).unwrap();
        let [Stmt::AnnAssign(assignment)] = module.suite().as_slice() else {
            panic!("expected an annotated assignment");
        };
        assert_eq!(unparse_annotation(&assignment.annotation), expected);
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

    for (name, expected) in [("fresh", [false, false]), ("owned", [false, true])] {
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
            .chunks_exact(2)
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
            .chunks_exact(2)
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
fn chained_generator_filters_keep_their_first_comparison_marker_owned() {
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
            .chunks_exact(2)
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
fn copied_finally_exits_keep_multi_edge_condition_nops() {
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
            .chunks_exact(2)
            .filter(|instruction| instruction[0] == NOP.code())
            .count();

        assert_eq!(nops, 3, "unexpected NOP count for {name}");
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
            .chunks_exact(2)
            .filter(|instruction| instruction[0] == NOP.code())
            .count();

        assert_eq!(nops, expected_nops, "unexpected NOP count for {name}");
    }
}

#[test]
fn comparison_markers_distinguish_forward_and_threaded_loop_fallthroughs() {
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
        .chunks_exact(2)
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
