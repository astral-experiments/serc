mod common;

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::fmt::Write as _;

    use super::common::{
        assert_executes, assert_matches_cpython_marshal, assert_matches_cpython_marshal_bytes,
    };
    use serc::{
        CPYTHON_TARGET, CompileError, compile, compile_bytes_to_pyc, compile_to_pyc, decode_source,
    };
    use serc_workloads as benchmark_workloads;

    #[test]
    fn exposes_the_pinned_cpython_source_identity() {
        assert_eq!(CPYTHON_TARGET.tag, "v3.14.5");
        assert_eq!(
            CPYTHON_TARGET.commit,
            "5607950ef232dad16d75c0cf53101d9649d89115"
        );
    }

    #[test]
    fn emits_a_python_3145_pyc_header() {
        let pyc = compile_to_pyc("answer = 42\n", "answer.py", 123).unwrap();
        assert_eq!(&pyc[..4], &CPYTHON_TARGET.magic_number);
        assert_eq!(&pyc[4..8], &0_u32.to_le_bytes());
        assert_eq!(&pyc[8..12], &123_u32.to_le_bytes());
        assert_eq!(&pyc[12..16], &12_u32.to_le_bytes());
    }

    #[test]
    fn compiles_source_bytes_and_uses_the_raw_size_in_pyc_headers() {
        let source = b"\xef\xbb\xbf# coding: utf-8\r\nanswer = 42\r\n";
        let pyc = compile_bytes_to_pyc(source, "answer.py", 123).unwrap();

        assert_eq!(&pyc[8..12], &123_u32.to_le_bytes());
        assert_eq!(
            &pyc[12..16],
            &u32::try_from(source.len()).unwrap().to_le_bytes()
        );
        assert_eq!(
            &pyc[16..],
            compile("# coding: utf-8\nanswer = 42\n", "answer.py")
                .unwrap()
                .marshal()
        );
    }

    #[test]
    fn exposes_source_decoding_without_filesystem_io() {
        assert_eq!(
            decode_source(b"\xef\xbb\xbf# coding: utf-8\r\nvalue = '\xc3\xa9'\r").unwrap(),
            "# coding: utf-8\nvalue = '\u{e9}'\n"
        );
        assert_eq!(
            decode_source(b"# coding: latin-1\nvalue = '\xe9'\n").unwrap(),
            "# coding: latin-1\nvalue = '\u{e9}'\n"
        );
        assert!(compile("# coding: latin-1\nvalue = 'already decoded'\n", "value.py").is_ok());
    }

    #[test]
    fn matches_cpython_marshal_for_supported_source_encodings() {
        let cases: [(&str, &[u8]); 8] = [
            ("latin_1.py", b"# coding: latin-1\nvalue = '\xe9'\n"),
            ("euc_jp.py", b"# coding: euc-jp\nvalue = '\xa1\xc1'\n"),
            ("koi8_r.py", b"# coding: koi8-r\nvalue = '\xe1'\n"),
            ("cp1251.py", b"# coding: cp1251\nvalue = '\xc0'\n"),
            ("iso_8859_1.py", b"# coding: iso.8859.1\nvalue = '\xe9'\n"),
            ("iso_ir_100.py", b"# coding: iso.ir.100\nvalue = '\xe9'\n"),
            ("u_jis.py", b"# coding: u.jis\nvalue = '\xa1\xc1'\n"),
            (
                "windows_1251.py",
                b"# coding: windows.1251\nvalue = '\xc0'\n",
            ),
        ];
        for (filename, source) in cases {
            assert_matches_cpython_marshal_bytes(source, filename);
        }
    }

    #[test]
    fn normalizes_bom_and_newlines_before_compiling() {
        let source =
            "\u{feff}def greet(name):\r\n    message = f\"héllo, {name}\"\r    return message\r\n";
        let normalized = "def greet(name):\n    message = f\"héllo, {name}\"\n    return message\n";

        assert_eq!(
            compile(source, "greet.py").unwrap().marshal(),
            compile(normalized, "greet.py").unwrap().marshal()
        );
    }

    #[test]
    fn structured_parse_error_preserves_ruff_context() {
        let source = "if True print('no')\n";
        let error = compile(source, "invalid.py").unwrap_err();

        let CompileError::Parse(parse_error) = &error else {
            panic!("expected Ruff parse error, got {error:?}");
        };
        assert_eq!(error.range(), Some(parse_error.location));
        assert_eq!(&source[parse_error.location], "print");

        let source_error = error.source().unwrap();
        assert!(source_error.is::<ruff_python_parser::ParseError>());
        assert!(source_error.source().is_some());
        assert_eq!(
            error.to_string(),
            format!("failed to parse Python source: {parse_error}")
        );
    }

    #[test]
    fn structured_parse_error_separates_invalid_patterns() {
        let source = "match value:\n    case [left] | [right]:\n        pass\n";
        let error = compile(source, "invalid_pattern.py").unwrap_err();

        let CompileError::InvalidPattern { message, range } = &error else {
            panic!("expected invalid pattern error, got {error:?}");
        };
        assert_eq!(message, "alternative patterns bind different names");
        assert_eq!(error.range(), Some(*range));
        assert_eq!(&source[*range], "[right]");
        assert!(error.source().is_none());
        assert_eq!(
            error.to_string(),
            "failed to parse Python source: alternative patterns bind different names"
        );
    }

    #[test]
    fn structured_parse_error_ranges_duplicate_pattern_bindings() {
        let source = "match value:\n    case [x, [x] | [x]]:\n        pass\n";
        let error = compile(source, "duplicate_pattern.py").unwrap_err();

        let CompileError::InvalidPattern { message, range } = &error else {
            panic!("expected invalid pattern error, got {error:?}");
        };
        assert_eq!(message, "multiple assignments to name 'x' in pattern");
        assert_eq!(&source[*range], "[x] | [x]");
    }

    #[test]
    fn matches_cpython_marshal_for_an_empty_module() {
        assert_matches_cpython_marshal("", "empty.py");
    }

    #[test]
    fn benchmark_workloads_match_cpython_marshal() {
        for workload in benchmark_workloads::COMPILE_MICRO_WORKLOADS
            .iter()
            .chain(benchmark_workloads::COMPOSITE_WORKLOADS)
            .chain(benchmark_workloads::LARGE_WORKLOADS)
            .chain(benchmark_workloads::MARSHAL_MICRO_WORKLOADS)
        {
            assert_matches_cpython_marshal(workload.source, workload.filename);
        }
    }

    #[test]
    fn matches_cpython_marshal_for_a_module_containing_only_global_statements() {
        let source = "global x\nglobal x, y, z\n";
        assert_matches_cpython_marshal(source, "global.py");
    }

    #[test]
    fn matches_cpython_marshal_when_a_match_case_ends_in_a_constant_expression() {
        let source = include_str!("fixtures/benchmark/match_control_flow.py");
        assert_matches_cpython_marshal(source, "match.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_match_case_ending_in_if() {
        let source = r#"match shopper:
    case "Jane":
        consume(shopper)
        if condition:
            consume(condition)
    case _:
        fallback(shopper)
"#;
        assert_matches_cpython_marshal(source, "match_tail_if.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_match_case_starting_with_if() {
        let source = r"match subject:
    case 1:
        if first or second:
            consume(subject)
    case 2:
        consume(subject)
";
        assert_matches_cpython_marshal(source, "match_leading_if.py");
    }

    #[test]
    fn matches_cpython_marshal_for_match_success_next_locations() {
        let source = r"def tuple_result(subject):
    match subject:
        case 1:
            return make_first(), make_second()
        case 2:
            raise Failure()
        case _:
            return None

async def await_result(subject):
    match subject:
        case 1:
            value = await produce()
            return value
        case 2:
            await consume()
        case _:
            return None

def nested_match(subject, nested):
    match subject:
        case 1:
            match nested:
                case 2:
                    return [nested]
                case _:
                    return []
        case _:
            return None
";
        assert_matches_cpython_marshal(source, "match_success_next_location.py");
    }

    #[test]
    fn matches_cpython_marshal_for_class_footer_control_flow() {
        let source = "class WithFooter:\n    with manager:\n        def method(self): ...\n\nclass WildcardFooter:\n    match value:\n        case int(): ...\n        case _:\n            def method(self): ...\n\nclass FoldedBranchFooter:\n    if ...:\n        ...\n    else:\n        with manager:\n            for value in values:\n                def method(self): ...\n";
        assert_matches_cpython_marshal(source, "class_footer.py");
    }

    #[test]
    fn matches_cpython_marshal_for_long_or_pattern_case_boundaries() {
        let source = "match x:\n    case \"abcd\" | \"abcd\" | \"abcd\" :\n        pass\n    case \"abcd\" | \"abcd\" | \"abcd\" | \"abcd\" | \"abcd\" | \"abcd\" | \"abcd\" | \"abcd\" | \"abcd\" | \"abcd\" | \"abcd\" | \"abcd\" | \"abcd\" | \"abcd\" | \"abcd\":\n        pass\n    case xxxxxxxxxxxxxxxxxxxxxxx:\n        pass\n";
        assert_matches_cpython_marshal(source, "pattern_matching_long.py");
    }

    #[test]
    fn reports_the_original_duplicate_pattern_binding_name() {
        let source = "class Container:\n    match value:\n        case [__captured, [__captured] | [__captured]]:\n            pass\n";
        let error = compile(source, "duplicate_pattern.py").unwrap_err();

        assert_eq!(
            error.to_string(),
            "failed to parse Python source: multiple assignments to name '__captured' in pattern"
        );
    }

    #[test]
    fn matches_cpython_marshal_for_private_pattern_capture_names() {
        let source = "class Container:\n    def capture(self, value):\n        match value:\n            case [__captured]:\n                return __captured\n        return None\n";
        assert_matches_cpython_marshal(source, "private_pattern.py");
    }

    #[test]
    fn matches_cpython_marshal_for_an_unreachable_generic_type_alias() {
        let source = "raise Exception\nassert False\ntype X[T] = T\n";
        assert_matches_cpython_marshal(source, "type_alias.py");
    }

    #[test]
    fn matches_cpython_marshal_for_generic_function_defaults() {
        let source = "def positional[T](value: T = T):\n    pass\n\ndef keyword[T](*, value: T = T):\n    pass\n\ndef both[T](value: T = T, *, other: T = T):\n    pass\n";
        assert_matches_cpython_marshal(source, "generic_defaults.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_generic_class_scopes() {
        let source = "class Outer[T]:\n    class Inner:\n        function = lambda: None\n        values = (value for value in ())\n\n        def method(self, value: T) -> T:\n            return value\n\n        def generic[U](self, value: U) -> U:\n            return value\n";
        assert_matches_cpython_marshal(source, "nested_generic.py");
    }

    #[test]
    fn matches_cpython_marshal_for_generic_wrapper_closures() {
        let source = r"def outer():
    value = int

    class Nested[T]:
        item: value

    return Nested


class Generic[T]:
    def method[U](self):
        return T, U


class ____PrivateContainer:
    def outer(self):
        __value = int

        def generic[T: __value]() -> __value:
            return T, __value

        return generic
";
        assert_matches_cpython_marshal(source, "generic_closures.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_function_decorator_closures() {
        let source = r"def method_decorators(decorator, decorator_factory):
    class Container:
        @decorator
        def method(self):
            pass

        @classmethod
        @decorator
        def class_method(cls):
            pass

        @decorator
        @staticmethod
        def static_method():
            pass

        @decorator_factory(1)
        async def coroutine(self):
            pass

    return Container


def nested_function_decorator(decorator):
    def parent():
        @decorator
        def child():
            pass

        return child

    return parent


def shadowed_class_decorator(decorator, local):
    class Container:
        decorator = local

        @decorator
        def method(self):
            pass

    return Container
";
        assert_matches_cpython_marshal(source, "function_decorator_closures.py");
    }

    #[test]
    fn matches_cpython_marshal_for_decorator_bindings_and_delete_only_locals() {
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
        assert_matches_cpython_marshal(source, "decorator_bindings.py");
    }

    #[test]
    fn matches_cpython_marshal_for_direct_definition_headers() {
        let source = r"def ordering():
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


def function_default_closure(value):
    def parent():
        def plain(item=value):
            return item

        def generic[T](item=value):
            return item

        return plain, generic

    return parent


def class_decorator_closure(decorator):
    def parent():
        @decorator
        class Child:
            pass

        return Child

    return parent
";
        assert_matches_cpython_marshal(source, "definition_headers.py");
    }

    #[test]
    fn matches_cpython_marshal_for_plain_class_header_closures() {
        let source = r"def function_headers(base, metaclass, bases, options, captured):
    def parent():
        class Direct(base, metaclass=metaclass):
            pass

        class Expanded(*bases, **options):
            pass

        class Nested((lambda: captured)()):
            pass

        return Direct, Expanded, Nested

    return parent


def class_headers(base, metaclass, bases, options):
    class Container:
        class Direct(base, metaclass=metaclass):
            pass

        class Expanded(*bases, **options):
            pass

    return Container


def local_headers(base, metaclass):
    def parent(base, metaclass):
        class Child(base, metaclass=metaclass):
            pass

        return Child

    return parent


def shadowed_class_headers(base, metaclass, local_base, local_meta):
    class Container:
        base = local_base
        metaclass = local_meta

        class Child(base, metaclass=metaclass):
            pass

    return Container


def generic_headers(base, metaclass, bound, default):
    def parent():
        class Child[T: bound = default](base, metaclass=metaclass):
            value: T

        return Child

    return parent


def generic_type_parameter_shadow(T):
    def parent():
        class Child[T](T):
            pass

        return Child

    return parent


class Global(global_base, metaclass=global_meta):
    pass
";
        assert_matches_cpython_marshal(source, "class_header_closures.py");
    }

    #[test]
    fn matches_cpython_marshal_for_closure_load_fusion_boundaries() {
        let source = "def tuple_value(label, url):\n    return label, lambda: url\n\ndef keyword_call(label, url):\n    return register(label=label, command=lambda: open(url))\n";
        assert_matches_cpython_marshal(source, "closure_fusion.py");
    }

    #[test]
    fn matches_cpython_marshal_for_generator_targets_shadowing_enclosing_freevars() {
        let source = r"def lambda_parent(all_elements, element):
    return lambda owner: all(owner.contains(element) for element in all_elements)

def function_parent(all_elements, element):
    def child(owner):
        return all(owner.contains(element) for element in all_elements)
    return child

def lambda_in_generator_parent(items, predicate):
    def child():
        return any((lambda: predicate(item))() for item in items)
    return child

def comprehension_shadow_parent(items, captured):
    def child():
        values = [captured for captured in items]
        generated = (item for item in items)
        return captured, values, generated
    return child
";
        assert_matches_cpython_marshal(source, "generator_target_shadow.py");
    }

    #[test]
    fn matches_cpython_marshal_for_decorated_generic_function_closures() {
        let source = r"class Container:
    @decorator
    def method[T](self, value: T) -> T: ...

def parent():
    captured = object()
    @decorator
    async def function[T: captured](value: T) -> T: ...
    return function
";
        assert_matches_cpython_marshal(source, "decorated_generic_closure.py");
    }

    #[test]
    fn matches_cpython_marshal_for_annotation_scope_closures() {
        let source = include_str!("fixtures/benchmark/annotation_scopes.py");
        assert_matches_cpython_marshal(source, "annotation_scopes.py");
    }

    #[test]
    fn matches_cpython_marshal_for_class_type_aliases() {
        let source = "from __future__ import annotations\n\nclass Container:\n    Local = int\n    type Alias = Local\n    type Generic[T] = tuple[Local, T]\n    type Factory[T] = lambda: (Local, T)\n";
        assert_matches_cpython_marshal(source, "class_alias.py");
    }

    #[test]
    fn matches_cpython_marshal_for_string_slice_constants() {
        let source = "subject[\"fine\":\"fine\"]\nsubject[\"x\":\"x\"]\n";
        assert_matches_cpython_marshal(source, "slice.py");
    }

    #[test]
    fn matches_cpython_marshal_for_noninterned_constant_metadata_collisions() {
        let source = r#"from package.config import exported, café
from other import *

if error.name == "package.config":
    recover()

class Outer:
    class Inner:
        pass
"#;
        assert_matches_cpython_marshal(source, "string_metadata_collision.py");
    }

    #[test]
    fn matches_cpython_marshal_for_multiline_literal_return_locations() {
        let source = r#"def triple_quoted():
    return """alpha
beta
gamma"""


def triple_bytes():
    return b"""alpha
beta"""


def implicit_concatenation(flag):
    if flag:
        return ("left "
                "right")
    return (b"fallback "
            b"bytes")


def parenthesized_string():
    return (
        "parenthesized"
    )


def parenthesized_scalar():
    return (
        42
    )


def one_line_literal():
    return "one line"


def multiline_nonliteral(value):
    return (
        value
    )
"#;
        assert_matches_cpython_marshal(source, "multiline_literal_return.py");
    }

    #[test]
    fn matches_cpython_marshal_for_unary_not_loop_exit_locations() {
        let source = r"def terminal_call():
    while not predicate():
        work()

def terminal_nested_not():
    while not not predicate():
        work()

def terminal_and(flag):
    while flag and not predicate():
        work()

def protected(lock):
    with lock:
        while not lock.done():
            work()
    work()
";
        assert_matches_cpython_marshal(source, "while_not_exit.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nonfinal_while_exits_in_with_bodies() {
        let source = r"def final_sync(manager, flag):
    with manager:
        while flag:
            step()
    outside()


def followed_sync(manager, flag):
    with manager:
        while flag:
            step()
        inside()
    outside()


def nested_followed_sync(manager, flag, outer):
    with manager:
        if outer:
            while flag:
                step()
            inside()
    outside()


def loop_else_sync(manager, flag):
    with manager:
        while flag:
            step()
        else:
            fallback()
        inside()
    outside()


async def followed_async(manager, flag):
    async with manager:
        while flag:
            step()
        inside()
    outside()
";
        assert_matches_cpython_marshal(source, "nonfinal_while_with.py");
    }

    #[test]
    fn matches_cpython_marshal_for_final_while_exit_markers_in_with_bodies() {
        let source = r"def single_exit(manager, flag):
    with manager:
        while flag:
            step()
    outside()


def multiple_condition_exits(manager, first, second):
    with manager:
        while first and second:
            step()
    outside()


def chained_comparison(manager, low, value, high):
    with manager:
        while low < value < high:
            step()
    outside()


def conditional_expression(manager, condition, first, second):
    with manager:
        while first if condition else second:
            step()
    outside()
";
        assert_matches_cpython_marshal(source, "final_while_with.py");
    }

    #[test]
    fn matches_cpython_marshal_for_unary_not_synthetic_exit_locations() {
        let source = r"def branch_join(flag):
    if flag:
        if not predicate():
            return False
    else:
        recover()
    return True


def terminal_assert(flag):
    if flag:
        assert predicate()
    else:
        assert not predicate()


def handler_exit():
    try:
        action()
    except Error as error:
        if not predicate():
            recover()


def finally_exit():
    try:
        action()
    finally:
        if not predicate():
            recover()


def loop_tail():
    while outer():
        if not predicate():
            work()
";
        assert_matches_cpython_marshal(source, "unary_not_synthetic_exits.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_try_at_the_end_of_a_loop() {
        let source = "async def show_status():\n    while True:\n        try:\n            if report_host:\n                data = value\n        except Exception as error:\n            pass\n";
        assert_matches_cpython_marshal(source, "loop_try.py");
    }

    #[test]
    fn matches_cpython_marshal_for_multi_exit_try_loop_tails() {
        let source = r"def conditional(items):
    for item in items:
        try:
            if first(item):
                consume(item)
            elif second(item):
                consume_other(item)
        except LookupError:
            recover(item)


def complete_branch(items):
    for item in items:
        try:
            if accepted(item):
                consume(item)
            else:
                reject(item)
        except LookupError:
            recover(item)


def single_exit_return(items):
    for item in items:
        try:
            if accepted(item):
                return item
        except LookupError:
            continue


def single_exit_break(items):
    for item in items:
        try:
            if accepted(item):
                break
        except LookupError:
            continue
";
        assert_matches_cpython_marshal(source, "try_loop_tail_normal_exits.py");
    }

    #[test]
    fn matches_cpython_marshal_for_protected_non_final_branch_loop_tails() {
        let source = r"def protected_state_machine(items):
    for index, item in enumerate(items):
        try:
            if state == SCANNING:
                if item.startswith(tag):
                    if item.endswith(end):
                        state = STATE_RETTYPE
                    else:
                        state = STATE_DOC
            elif state == STATE_DOC:
                if item.startswith(end):
                    state = STATE_RETTYPE
                else:
                    output.append(item)
            elif state == STATE_RETTYPE:
                state = STATE_NAME
            elif state == STATE_NAME:
                state = STATE_ARGS
            elif state == STATE_ARGS:
                if item.startswith(start):
                    state = SCANNING
                else:
                    arguments.append(item)
        except ParseError:
            raise
        except Exception as error:
            message = 'details'
            raise ParseError(index, message) from error


def single_normal_exit_control(items):
    for item in items:
        try:
            if accepted(item):
                consume(item)
        except LookupError:
            continue
";
        assert_matches_cpython_marshal(source, "protected_non_final_branch_loop_tails.py");
    }

    #[test]
    fn matches_cpython_marshal_for_shared_artificial_loop_backedges() {
        let source = r"def try_else(values, output):
    for value in values:
        try:
            callback = lookup(value)
        except KeyError:
            pass
        else:
            if callback(value):
                output.append(value)
    return output


def try_finally(values):
    for value in values:
        try:
            consume()
        finally:
            if enabled():
                cleanup()
";
        assert_matches_cpython_marshal(source, "shared_artificial_loop_backedges.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_try_suite_implicit_returns() {
        let source = r"def body_if(flag):
    try:
        if flag:
            action()
    except Exception:
        pass

def leading_and_terminal_if(first, second):
    try:
        if first:
            before()
        if second:
            action()
    except Exception:
        pass

def body_if_else(flag):
    try:
        if flag:
            action()
        else:
            other()
    except Exception:
        pass

def body_while(flag):
    try:
        while flag:
            action()
            break
    except Exception:
        pass

def body_match(value):
    try:
        match value:
            case 1:
                action()
    except Exception:
        pass

def else_if(flag):
    try:
        action()
    except Exception:
        recover()
    else:
        if flag:
            success()

def conditional_expression(flag):
    try:
        action() if flag else other()
    except Exception:
        pass

def boolean_expression(flag):
    try:
        flag and action()
    except Exception:
        pass

def chained_comparison(left, middle, right):
    try:
        left < middle < right
    except Exception:
        pass

def tuple_conditional_expression(flag):
    try:
        (before(), action() if flag else other())
    except Exception:
        pass

async def coroutine_if(flag):
    try:
        if flag:
            await action()
    except Exception:
        pass
";
        assert_matches_cpython_marshal(source, "terminal_try_returns.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_terminal_try_condition_ownership() {
        let source = r"def nested_try(a, b):
    try:
        if a:
            try:
                b and action()
            except ValueError:
                inner()
    except Exception:
        recover()

def complex_condition(a, b, c):
    try:
        if a and (b if c else other()):
            action()
    except Exception:
        recover()

def guarded_match(value, flag):
    try:
        match value:
            case 1 if flag and predicate():
                action()
    except Exception:
        recover()

def nested_try_generator(a, b):
    yield 1
    try:
        if a:
            try:
                b and action()
            except ValueError:
                inner()
    except Exception:
        recover()

async def nested_try_coroutine(a, b):
    await before()
    try:
        if a:
            try:
                b and await action()
            except ValueError:
                inner()
    except Exception:
        recover()

async def nested_try_async_generator(a, b):
    yield 1
    try:
        if a:
            try:
                b and action()
            except ValueError:
                inner()
    except Exception:
        recover()

async def coroutine_nested_finally(a):
    try:
        if a:
            try:
                action()
            finally:
                cleanup()
    except Exception:
        recover()

async def coroutine_nested_else(a):
    try:
        if a:
            try:
                action()
            except ValueError:
                inner()
            else:
                success()
    except Exception:
        recover()

async def await_late_in_bool_condition(a):
    try:
        if a and await predicate():
            try:
                action()
            except ValueError:
                inner()
    except Exception:
        recover()

async def await_before_nested_finally(a):
    await before()
    try:
        if a:
            try:
                action()
            finally:
                cleanup()
    except Exception:
        recover()
";
        assert_matches_cpython_marshal(source, "terminal_try_condition_ownership.py");
    }
    #[test]
    fn matches_cpython_marshal_for_terminal_try_dynamic_condition_owners() {
        let source = r"def call_then_local():
    try:
        flag = source()
        if flag:
            action()
    except Exception:
        recover()

def unpack_then_local(values):
    try:
        first, second = values
        if first:
            action()
    except Exception:
        recover()

def fast_pair(left, right):
    try:
        if left + right:
            action()
    except Exception:
        recover()

def fast_const(left):
    try:
        if left + 1:
            action()
    except Exception:
        recover()

def global_call():
    try:
        if predicate():
            action()
    except Exception:
        recover()

def local_call(predicate):
    try:
        if predicate():
            action()
    except Exception:
        recover()

def method_call(obj):
    try:
        if obj.predicate():
            action()
    except Exception:
        recover()

def fast_dict(key, value):
    try:
        if {key: value}:
            action()
    except Exception:
        recover()

def constant_dict(value):
    try:
        if {'key': value}:
            action()
    except Exception:
        recover()

def unpack_dict(mapping):
    try:
        if {**mapping}:
            action()
    except Exception:
        recover()

def prior_call_then_while(flag):
    try:
        seed()
        while flag:
            action()
            break
    except Exception:
        recover()
";
        assert_matches_cpython_marshal(source, "terminal_try_dynamic_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_plain_try_condition_marker_owners() {
        let source = r#"def early_return(elements):
    try:
        if not elements:
            return None
        return elements[0]
    except Exception as error:
        return str(error)

def conditional_value(data):
    try:
        value = data["key"] if data else ""
        if not isinstance(value, str):
            raise TypeError
    except (KeyError, TypeError):
        return None
    return value

def comprehension_then_condition(values, flag):
    try:
        items = [value for value in values]
        if flag:
            return items
    except Exception:
        return None

class Derived(Base):
    def specialized_super_then_condition(self, flag):
        try:
            super().prepare()
            if flag:
                return True
        except Exception:
            return False

def adjacent_regions(flag):
    try:
        prepare()
    except ValueError:
        recover()
    try:
        if flag:
            action()
    except Exception:
        recover()

def branch_before_try(branch, flag):
    if branch:
        action()
    try:
        if flag:
            action()
    except Exception:
        recover()
    return 1

def no_default_definition_before_try(flag):
    def inner():
        return 1
    try:
        if flag:
            action()
    except Exception:
        recover()
    return inner

def folded_default_definition_before_try(flag):
    def inner(value=0):
        return value
    try:
        if flag:
            action()
    except Exception:
        recover()
    return inner

def no_default_lambda_before_try(flag):
    inner = lambda: 1
    try:
        if flag:
            action()
    except Exception:
        recover()
    return inner

def folded_default_lambda_before_try(flag):
    inner = lambda value=0: value
    try:
        if flag:
            action()
    except Exception:
        recover()
    return inner

def while_plain_try(flag):
    while flag:
        try:
            if flag:
                action()
        except Exception:
            recover()
        break
"#;
        assert_matches_cpython_marshal(source, "plain_try_condition_marker_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_shifted_plain_try_condition_markers() {
        let source = r"def retained(items):
    for item in items:
        consume(item, 0)
        consume(item, 1)
        consume(item, 2)
        consume(item, 3)
        consume(item, 4)
        consume(item, 5)
        consume(item, 6)
        consume(item, 7)
        try:
            value = item.make()
            if value.flags.owned:
                continue
            value = value.view(Type)
        except Error:
            continue
        use(value)


def shifted(items):
    for item in items:
        consume(item, 0)
        consume(item, 1)
        consume(item, 2)
        consume(item, 3)
        consume(item, 4)
        consume(item, 5)
        consume(item, 6)
        consume(item, 7)
        consume(item, 8)
        try:
            value = item.make()
            if value.flags.owned:
                continue
            value = value.view(Type)
        except Error:
            continue
        use(value)


def direct_condition(items, first, second):
    for item in items:
        consume(first, second)
        consume(first, second)
        try:
            if item:
                continue
            use(item)
        except Error:
            continue
";
        assert_matches_cpython_marshal(source, "shifted_plain_try_condition_markers.py");
    }

    #[test]
    fn matches_cpython_marshal_for_generator_like_terminal_elif_owners() {
        let source = r"def generator_elif(first, second):
    yield 1
    try:
        if first:
            one()
        elif second:
            two()
    except Exception:
        recover()

async def async_generator_elif(first, second):
    yield 1
    try:
        if first:
            one()
        elif second:
            two()
    except Exception:
        recover()

async def coroutine_after_await(first, second, final):
    await before()
    try:
        if first:
            one()
        elif second:
            two()
        if final:
            three()
    except Exception:
        recover()
";
        assert_matches_cpython_marshal(source, "terminal_try_generator_elif.py");
    }

    #[test]
    fn matches_cpython_marshal_for_generator_none_condition_owners() {
        let source = r"async def post_await_conditions(wait, first, second):
    if first is None:
        before()
    await wait()
    assert first is not None
    if first is None:
        use(first)
    elif second is None:
        use(second)
    if first is None and second is not None:
        use(second)
    return use(first) if second is None else fallback()


async def loop_control(wait, values):
    await wait()
    for value in values:
        if value is None:
            continue


async def try_control(wait, value):
    await wait()
    try:
        if value is None:
            use(value)
    except Error:
        recover()


async def with_control(wait, manager, value):
    await wait()
    with manager:
        if value is None:
            use(value)


async def finally_control(wait, value):
    await wait()
    try:
        use(value)
    finally:
        if value is None:
            cleanup()


def generator_conditions(first, second):
    yield before()
    if first is None and second is not None:
        yield second


def generator_while(value):
    while value is None:
        consume()
    yield value


async def async_generator_after_await(wait, value):
    await wait()
    while value is not None:
        yield value


async def async_generator_after_yield(value):
    yield value
    while value is None:
        consume()
";
        assert_matches_cpython_marshal(source, "generator_none_condition_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_loop_tail_assertion_owners() {
        let source = r"async def coroutine_direct(wait, items, value):
    await wait()
    for item in items:
        assert value is None


def generator_or(items, value, other):
    yield value
    for item in items:
        assert value is None or other


async def async_generator_and(items, first, second):
    yield first
    for item in items:
        assert first is None and second


async def protected_try(wait, items, value):
    await wait()
    try:
        for item in items:
            assert value is None
    except Error:
        recover()


async def protected_with(wait, items, value, manager):
    await wait()
    with manager:
        for item in items:
            assert value is None or item


async def protected_finally(wait, items, value):
    try:
        await wait()
    finally:
        for item in items:
            assert value is None
";
        assert_matches_cpython_marshal(source, "loop_tail_assertion_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_try_exits_across_exception_boundaries() {
        let source = r"try:
    1 / 0
except ZeroDivisionError:
    pass

try:
    raise ValueError
except ValueError as error:
    print(error)

try:
    pass
except Exception:
    pass
finally:
    try:
        pass
    except BaseException as error:
        raise error

try:
    pass
except Exception as error:
    try:
        raise error
    except BaseException:
        pass

try:
    try:
        pass
    except BaseException as error:
        raise error
except Exception:
    pass
";
        assert_matches_cpython_marshal(source, "try_exit.py");
    }

    #[test]
    fn matches_cpython_marshal_for_small_with_exits() {
        let source = "def normal():\n    with manager:\n        value = 0\n    value = 1\n\ndef terminal():\n    with manager:\n        value = 0\n        return 1\n    value = 1\n\ndef nested_suppression():\n    with first, \\\n        second:\n        raise Error\n    return result\n";
        assert_matches_cpython_marshal(source, "with_exit.py");
    }

    #[test]
    fn matches_cpython_marshal_for_converted_pop_blocks_in_synchronous_with_bodies() {
        let source = r"def terminal_handler(manager):
    with manager:
        try:
            action()
        except Error:
            raise

def terminal_else(manager):
    with manager:
        try:
            action()
        except Error:
            recover()
        else:
            raise Other

def nested_return(outer, inner):
    with outer:
        with inner:
            return value

def nested_raise(outer, inner):
    with outer:
        with inner:
            raise Error

def no_duplicate_after_finally(manager):
    with manager:
        try:
            action()
        finally:
            cleanup()

def no_duplicate_after_if(manager, condition):
    with manager:
        try:
            action()
            if condition:
                other()
        except Error:
            raise

def multiple_normal_predecessors(manager):
    with manager:
        try:
            action()
        except Error:
            recover()
";
        assert_matches_cpython_marshal(source, "with_converted_pop_block.py");
    }

    #[test]
    fn matches_cpython_marshal_for_handler_fed_pop_blocks_in_synchronous_with_bodies() {
        let source = r"def named(manager):
    with manager:
        try:
            raise Other
        except Error as error:
            recover(error)

def unnamed(manager):
    with manager:
        try:
            raise Error
        except Error:
            recover()

def assertion(manager, condition):
    with manager:
        try:
            raise Other
        except Error:
            assert condition

def body_falls_through(manager):
    with manager:
        try:
            action()
        except Error:
            recover()

def terminal_loop(manager):
    with manager:
        try:
            while True:
                action()
        except Error:
            pass
";
        assert_matches_cpython_marshal(source, "with_handler_fed_pop_block.py");
    }

    #[test]
    fn matches_cpython_marshal_for_trailing_finally_exits_in_synchronous_with_bodies() {
        let source = r"def direct(manager):
    with manager:
        try:
            body()
        finally:
            cleanup()

def followed(manager):
    with manager:
        try:
            body()
        finally:
            cleanup()
    after()

def leading_statement(manager):
    with manager:
        before()
        try:
            body()
        finally:
            cleanup()
    after()

def multiple_items(outer, inner):
    with outer, inner:
        try:
            body()
        finally:
            cleanup()
    after()

def enclosing_try(manager):
    try:
        with manager:
            try:
                body()
            finally:
                cleanup()
        after()
    except Exception:
        recover()

def multiple_finalbody_statements(manager):
    with manager:
        try:
            body()
        finally:
            cleanup_one()
            cleanup_two()
    after()

def nested_managers(outer, inner):
    with outer:
        with inner:
            try:
                body()
            finally:
                cleanup()
    after()

async def synchronous_inside_async(async_manager, sync_manager):
    async with async_manager:
        with sync_manager:
            try:
                await body()
            finally:
                await cleanup()
    await after()

def nontrailing():
    with manager:
        try:
            body()
        finally:
            cleanup()
        inside_after()
    outside_after()

def nested_if(manager, flag):
    with manager:
        if flag:
            try:
                body()
            finally:
                cleanup()
    after()

def nested_loop(manager, items):
    with manager:
        for item in items:
            try:
                body(item)
            finally:
                cleanup(item)
    after()

def unreachable_trailing_try(manager):
    with manager:
        return value()
        try:
            body()
        finally:
            cleanup()

async def asynchronous_manager(manager):
    async with manager:
        try:
            await body()
        finally:
            await cleanup()
    await after()

async def synchronous_outside_async(sync_manager, async_manager):
    with sync_manager:
        async with async_manager:
            try:
                await body()
            finally:
                await cleanup()
    await after()
";
        assert_matches_cpython_marshal(source, "with_finally_exit.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_assertions_in_with_bodies() {
        let source = "def simple(manager, condition):\n    with manager:\n        assert condition\n\ndef message(manager, condition):\n    with manager:\n        use()\n        assert condition, make_message()\n\ndef boolean(manager, first, second):\n    with manager:\n        assert first or second\n\ndef conjunction(manager, first, second):\n    with manager:\n        assert first and second\n\ndef conditional(manager, first, second, condition):\n    with manager:\n        assert first if condition else second\n\ndef constant_true():\n    with manager:\n        assert True\n\ndef constant_false():\n    with manager:\n        assert False\n\ndef followed(manager, condition):\n    with manager:\n        assert condition\n    use()\n\ndef same_line(manager, condition):\n    with manager: assert condition\n\ndef nested(outer, inner, condition):\n    try:\n        with outer:\n            with inner:\n                assert condition\n    finally:\n        cleanup()\n\nasync def asynchronous(manager, condition):\n    async with manager:\n        assert condition\n";
        assert_matches_cpython_marshal(source, "with_assert.py");
    }

    #[test]
    fn matches_cpython_marshal_for_final_fallthrough_branches_in_with_bodies() {
        let source = r"def simple(manager, first, second):
    with manager:
        if first:
            value = 1
        elif second:
            value = 2
        else:
            value = 3
    return value


def boolean_elif(manager, first, second, third):
    with manager:
        if first:
            value = 1
        elif second and third:
            value = 2
        else:
            value = 3
    return value
";
        assert_matches_cpython_marshal(source, "with_final_fallthrough_branches.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_two_operand_conditions_in_with_bodies() {
        let source = r"def terminal(manager, first, second):
    with manager:
        value = make()
        if not first or not second:
            raise Error(value)


def negated_conjunction(z, msgform):
    with np.errstate(invalid='ignore'):
        if not (np.isnan(z.real) and np.isinf(z.imag)):
            raise AssertionError(msgform % (z.real, z.imag))


def nonterminal(manager, first, second):
    with manager:
        if not first or not second:
            raise Error
        consume()


def fallthrough_body(manager, first, second):
    with manager:
        if not first or not second:
            consume()


def outside_with(first, second):
    if not first or not second:
        raise Error


async def asynchronous(manager, first, second):
    async with manager:
        if not first or not second:
            raise Error
";
        assert_matches_cpython_marshal(source, "with_terminal_two_operand_condition.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nonfinal_branching_with_body_control() {
        let source = r"def nonfinal(manager, first, second):
    with manager:
        if first:
            value = 1
        elif second:
            value = 2
        else:
            value = 3
        consume(value)
    return value
";
        assert_matches_cpython_marshal(source, "with_nonfinal_branching_control.py");
    }

    #[test]
    fn matches_cpython_marshal_for_async_with_returns() {
        let source = "async def value(manager):\n    async with manager as result:\n        return result.value\n\nasync def constant(manager):\n    async with manager:\n        return 1\n\nasync def nested(outer, inner):\n    async with outer:\n        with inner:\n            return value\n";
        assert_matches_cpython_marshal(source, "async_with_return.py");
    }

    #[test]
    fn matches_cpython_marshal_for_loop_control_unwinding() {
        let source = "def sync_context(manager, items):\n    for item in items:\n        with manager:\n            if item > 0:\n                continue\n            break\n\ndef exception_handler(items):\n    for item in items:\n        try:\n            consume(item)\n        except Exception:\n            handle(item)\n            continue\n        use(item)\n\ntry:\n    for item in items:\n        consume(item)\n        break\nexcept TypeError:\n    handle()\nafter()\n";
        assert_matches_cpython_marshal(source, "loop_control_unwind.py");
    }

    #[test]
    fn matches_cpython_marshal_for_protected_iterator_cleanup_owners() {
        let source = r"def protected_with_continue(manager, items, magic_key):
    values = set()
    with manager:
        for item in items:
            if not item.startswith(magic_key):
                continue
            pieces = [piece.strip() for piece in item.split(':', 1)]
            if len(pieces) == 2:
                values = values.union(pieces[1].upper().split())
    return values


def protected_iterator_return(items):
    try:
        for item in items:
            if item.ready():
                return item.value().lower()
    except Exception:
        pass
    return None


async def protected_async_iterator_return(items):
    try:
        async for item in items:
            return make(item)
    except Exception:
        pass


def inner_try_return(items):
    for item in items:
        try:
            return item.value()
        except Exception:
            pass


def unprotected_continue(items):
    for item in items:
        if not item.ready():
            continue
        consume(item)
";
        assert_matches_cpython_marshal(source, "protected_iterator_cleanup_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_authored_breaks_after_cold_block_ordering() {
        let source = r"def authored_break(items):
    for item in items:
        try:
            action(item)
        except Error:
            recover(item)
            break
        use(item)
    after()


def synthetic_cold_fallthrough(items):
    for item in items:
        try:
            action(item)
        except Error:
            recover(item)
        use(item)
    after()
";
        assert_matches_cpython_marshal(source, "cold_handler_break.py");
    }

    #[test]
    fn matches_cpython_marshal_for_inlined_break_line_nops() {
        let source = r"def conditional(flag):
    while True:
        if flag:
            break
        step()
    return value

def direct():
    while True:
        break
    return value

def same_line():
    while True: break
    return value

def same_line_after_expression():
    while True: step(); break
    return value

def loop_else(flag):
    while flag:
        if done():
            break
        step()
    else:
        fallback()
    return value

def large_exit(flag):
    while True:
        if flag:
            break
        step()
    first(); second(); third(); fourth(); fifth()
    return value

def implicit_return(flag):
    while True:
        if flag:
            break
        step()

def managed(manager, flag):
    while True:
        with manager:
            if flag:
                break
            step()
    return value

def iteration(items, flag):
    for item in items:
        if flag:
            break
    return value
";
        assert_matches_cpython_marshal(source, "inlined_break_nop.py");
    }

    #[test]
    fn matches_cpython_marshal_for_folded_while_constant_order() {
        let source = r#"async def empty_pool_retained(action, consume):
    while True:
        await action()
        consume(flag=True)

async def prefixed_unused(action):
    "doc"
    while True:
        await action()

async def prefixed_retained(action, consume):
    "doc"
    while True:
        await action()
        consume(flag=True)

async def prefixed_retained_assignment(action):
    "doc"
    while True:
        await action()
        marker = True

async def await_before_loop(action, consume):
    await action()
    while True:
        await action()
        consume(flag=True)
"#;
        assert_matches_cpython_marshal(source, "folded_while_constant_order.py");
    }

    #[test]
    fn matches_cpython_marshal_for_loop_tail_condition_orientation() {
        let source = r"def terminal_while_break():
    while True:
        value = step()
        if stop(value):
            break
        if condition(value):
            consume(value)

def terminal_while_compound_control():
    while True:
        value = step()
        if stop(value):
            break
        if condition(value):
            consume(value)
        else:
            reject(value)

def nonfinal_nested_while(items):
    for item in items:
        if first(item):
            while condition(item):
                consume(item)
        elif second(item):
            other(item)

def direct_nested_while_control(items):
    for item in items:
        while condition(item):
            consume(item)

def protected_nested_while_control(items):
    try:
        for item in items:
            while condition(item):
                consume(item)
    except Exception:
        recover()

def nonfinal_assertions(items):
    for item in items:
        if first(item):
            assert condition(item)
        else:
            assert other(item)

def nonfinal_break(items):
    for item in items:
        if first(item):
            if stop(item):
                break
        else:
            consume(item)

def nonfinal_raise(items):
    for item in items:
        if first(item):
            if rejected(item):
                raise ValueError(item)
        else:
            consume(item)

def nonfinal_return(items):
    for item in items:
        if first(item):
            if accepted(item):
                return item
        else:
            consume(item)

def direct_assertion_control(items):
    for item in items:
        assert condition(item)
";
        assert_matches_cpython_marshal(source, "loop_tail_condition_orientation.py");
    }

    #[test]
    fn matches_cpython_marshal_for_protected_loop_tail_first_branch_owners() {
        let source = r"def with_owner(manager, items, match, accept, reject):
    with manager:
        for item in items:
            found = match(item)
            if found:
                if accept(found):
                    accept(item)
                else:
                    reject(found)
            else:
                reject(item)
";
        assert_matches_cpython_marshal(source, "protected_loop_tail_first_branch_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_async_comprehension_cleanup_exits() {
        let source = "async def discarded():\n    [item async for item in source]\n\nasync def returned():\n    return [item async for item in source]\n";
        assert_matches_cpython_marshal(source, "async_comprehension.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_async_comprehension_cleanup_order() {
        let source =
            "async def test(): return [[x async for x in elements(n)] async for n in range(3)]\n";
        assert_matches_cpython_marshal(source, "nested_async_comprehension.py");
    }

    #[test]
    fn matches_cpython_marshal_for_multiline_constant_match_guards() {
        let source = "match 1:\n    case _ if (True):\n        pass\n\nmatch 1:\n    case _ if (\n        True\n    ):\n        pass\n";
        assert_matches_cpython_marshal(source, "match_guard.py");
    }

    #[test]
    fn matches_cpython_marshal_for_pass_only_try_else_finally() {
        let source = "try:\n    foo()\nexcept Exception:\n    pass\nelse:\n    pass\nfinally:\n    pass\n\ntry:\n    pass\nexcept Exception:\n    pass\nraise (\n    Exception\n)\n";
        assert_matches_cpython_marshal(source, "try.py");
    }

    #[test]
    fn matches_cpython_marshal_for_noop_finally_bodies() {
        let source = r"try:
    body()
except Error:
    recover()
finally:
    ...

try:
    body()
except Error:
    recover()
finally:
    pass
    pass

try:
    try:
        body()
    finally:
        pass
except Error:
    recover()
";
        assert_matches_cpython_marshal(source, "try_finally_noops.py");
    }

    #[test]
    fn matches_cpython_marshal_for_multiple_exception_group_handlers() {
        let source = "try:\n    body()\nexcept* ValueError:\n    first()\nexcept* TypeError:\n    second()\n";
        assert_matches_cpython_marshal(source, "try_star.py");
    }

    #[test]
    fn matches_cpython_marshal_for_ownerless_late_exception_dispatch_markers() {
        let source = r"async def coroutine_multiple_handlers():
    await source()
    try:
        action()
    except ValueError:
        first()
    except TypeError:
        second()


def generator_multiple_handlers():
    yield source()
    try:
        action()
    except ValueError:
        first()
    except TypeError:
        second()


def with_multiple_handlers(manager):
    with manager:
        try:
            action()
        except ValueError:
            first()
        except TypeError:
            second()


def finally_multiple_handlers():
    try:
        try:
            action()
        except ValueError:
            first()
        except TypeError:
            second()
    finally:
        cleanup()


async def conditional_exception_type():
    await source()
    try:
        action()
    except (FirstError if condition else SecondError):
        recover()


async def single_handler_control():
    await source()
    try:
        action()
    except ValueError:
        recover()


async def tuple_handler_control():
    await source()
    try:
        action()
    except (ValueError, TypeError):
        recover()


async def bare_fallback_control():
    await source()
    try:
        action()
    except ValueError:
        recover()
    except:
        fallback()
";
        assert_matches_cpython_marshal(source, "handler_dispatch_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_boolean_exception_type_marker_owners() {
        let cases = [
            (
                "handler_boolop_plain.py",
                r"def f():
    try:
        action()
    except A or B or C:
        recover()
",
            ),
            (
                "handler_boolop_generator.py",
                r"def f():
    yield source()
    try:
        action()
    except A and B:
        recover()
",
            ),
            (
                "handler_boolop_with.py",
                r"def f(manager):
    with manager:
        try:
            action()
        except A or B or C:
            recover()
",
            ),
            (
                "handler_boolop_finally.py",
                r"def f():
    try:
        try:
            action()
        except A and B:
            recover()
    finally:
        cleanup()
",
            ),
            (
                "handler_boolop_later.py",
                r"def f():
    try:
        action()
    except X:
        first()
    except A or B or C:
        recover()
",
            ),
            (
                "handler_boolop_dynamic.py",
                r"def f():
    try:
        action()
    except X:
        first()
    except f(g(h())) or B:
        recover()
",
            ),
            (
                "handler_boolop_conditional.py",
                r"def f():
    try:
        action()
    except (A if C else B) or D:
        recover()
",
            ),
            (
                "handler_boolop_comprehension.py",
                r"def f(values):
    try:
        action()
    except [x or y for x in values] or B:
        recover()
",
            ),
            (
                "handler_boolop_exception_group.py",
                r"def f():
    try:
        action()
    except* A or B:
        recover()
",
            ),
        ];

        for (filename, source) in cases {
            assert_matches_cpython_marshal(source, filename);
        }
    }

    #[test]
    fn matches_cpython_marshal_for_a_return_from_an_exception_handler() {
        let source = "def f():\n    value = source()\n    try:\n        raise ValueError\n    except ValueError:\n        return value\n\ndef nested():\n    try:\n        body()\n    except ValueError as error:\n        try:\n            pass\n        except TypeError:\n            raise error\n\ndef nested_message():\n    try:\n        body()\n    except ValueError as error:\n        try:\n            finish()\n        except TypeError:\n            log(f\"error: {error}\")\n";
        assert_matches_cpython_marshal(source, "handler_return.py");
    }

    #[test]
    fn matches_cpython_marshal_for_named_exception_cleanup() {
        let source = r"class Container:
    def nested_handler(self):
        try:
            body()
        except ValueError as __error:
            try:
                pass
            except TypeError:
                raise __error
";
        assert_matches_cpython_marshal(source, "named_exception_cleanup.py");
    }

    #[test]
    fn matches_cpython_marshal_for_named_handler_return_cleanup() {
        let source = r"class Container:
    def return_from_handler(self):
        try:
            body()
        except Error as __error:
            return __error
";
        assert_matches_cpython_marshal(source, "named_handler_return.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nonterminal_handler_condition_fallthrough() {
        let source = r"def named_raise(flag):
    try:
        action()
    except Error as error:
        if flag:
            raise error
    after()


def named_return(flag):
    try:
        action()
    except Error as error:
        if flag:
            return error
    after()


def bare_raise(flag):
    try:
        action()
    except Error:
        if flag:
            raise
    after()


def final_elif(first, second):
    try:
        action()
    except Error as error:
        if first:
            raise error
        elif second:
            raise error
    after()


def explicit_else(flag):
    try:
        action()
    except Error as error:
        if flag:
            raise error
        else:
            consume(error)
    after()
";
        assert_matches_cpython_marshal(source, "nonterminal_handler_fallthrough.py");
    }

    #[test]
    fn matches_cpython_none_condition_marker_owners_in_exception_handlers() {
        let source = r"def is_none(value):
    try:
        action()
    except Error:
        if value is None:
            recover()
        finish()


def is_not_none(value, items):
    try:
        action()
    except Error:
        if value is not None:
            recover()
        result = [item for item in items]


def named(value):
    try:
        action()
    except Error as error:
        if value is None:
            recover(error)
        finish()


def nested(value):
    try:
        action()
    except Error:
        if predicate(value):
            if value is not None:
                recover()
        finish()


def reversed_control(value):
    try:
        action()
    except Error:
        if None is value:
            recover()
        finish()


def outside_handler_control(value):
    if value is None:
        recover()
    finish()


def later_bare_handler_control(value):
    try:
        action()
    except ValueError as error:
        consume(error)
    except TypeError:
        if value is None:
            recover()
        finish()
";
        assert_matches_cpython_marshal(source, "handler_none_condition_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_named_handler_nested_finally_cleanup() {
        let source = r"class Container:
    def fallthrough_from_nested_try(self):
        try:
            body()
        except Error as __error:
            try:
                consume(__error)
            finally:
                pass
";
        assert_matches_cpython_marshal(source, "named_handler_nested_finally.py");
    }

    #[test]
    fn matches_cpython_marshal_for_named_handler_trailing_with_cleanup() {
        let source = r"def named_handler_trailing_with(outer, inner):
    with outer:
        try:
            action()
        except Error as error:
            with inner:
                recover(error)
        else:
            finish()
";
        assert_matches_cpython_marshal(source, "named_handler_trailing_with.py");
    }

    #[test]
    fn matches_cpython_marshal_for_bare_handler_inlineable_cleanup() {
        let source = r"def trailing_try(key):
    try:
        value = first[key]
    except KeyError:
        try:
            value = second[key]
        except KeyError:
            raise NameError(key) from None
    return value


def trailing_if(flag):
    try:
        action()
    except Error:
        value = None
        if flag:
            value = recover()
        else:
            value = fallback()
    return value


def trailing_if_with_expression_join(flag):
    try:
        action()
    except Error:
        value = None
        if flag:
            value = (
                first()
                or second()
            )
        else:
            value = fallback()
    return value


def trailing_if_with_terminating_arm(first, second):
    try:
        action()
    except Error:
        if first:
            raise
        elif second:
            value = recover()
        else:
            value = fallback()
    return value
";
        assert_matches_cpython_marshal(source, "bare_handler_inlineable_cleanup.py");
    }

    #[test]
    fn matches_cpython_marshal_for_bare_handler_cleanup_controls() {
        let source = r"def implicit_fallthrough(flag):
    try:
        action()
    except Error:
        if flag:
            raise
    after()


def single_normal_exit(flag):
    try:
        action()
    except Error:
        if flag:
            raise
        else:
            recover()
    after()
";
        assert_matches_cpython_marshal(source, "bare_handler_cleanup_controls.py");
    }

    #[test]
    fn matches_cpython_marshal_for_bare_handler_call_cleanup_inside_with() {
        let source = r"def protected(manager, flag):
    with manager:
        try:
            action()
        except Error:
            result = check()
            if result:
                report(result)


def explicit_else_control(flag):
    try:
        action()
    except Error:
        first()
        if flag:
            second()
        else:
            third()
";
        assert_matches_cpython_marshal(source, "bare_handler_call_cleanup.py");
    }

    #[test]
    fn matches_cpython_marshal_for_folded_expressions_in_protected_regions() {
        let source = "def try_except():\n    try:\n        'try'\n        body()\n    except:\n        'except'\n        recover()\n\ndef except_only():\n    try:\n        body()\n    except:\n        'except'\n\ndef nested_handlers():\n    try:\n        try:\n            body()\n        except ValueError:\n            recover()\n        except TypeError:\n            recover()\n    finally:\n        cleanup()\n\ndef with_body(manager):\n    with manager:\n        'with'\n        body()\n\ndef with_only(manager):\n    with manager:\n        'with'\n\nasync def protected_condition():\n    await source\n    if test:\n        await other\n\ntry:\n    pass\nexcept (ValueError if True else TypeError):\n    pass\n\ntry:\n    pass\nexcept* (ValueError if True else TypeError):\n    pass\n\ndef protected_conditions(manager, first, second):\n    with manager:\n        if first:\n            body()\n        if second:\n            body()\n\ndef generator_try():\n    yield 1\n    try:\n        pass\n    except Exception:\n        pass\n\nif condition:\n    try:\n        with manager:\n            body()\n    except Error:\n        pass\n";
        assert_matches_cpython_marshal(source, "protected_nop.py");
    }

    #[test]
    fn matches_cpython_marshal_for_folded_large_integer_bitwise_operations() {
        let source = "bit_and = 99999999999999999999999 & 0o200000\nbit_or = 99999999999999999999999 | 0o777\nbit_xor = 18446744073709551616 ^ 18446744073709551616\nsum = 9999999999999999999999999999999999999999 + 9999999999999999999999999999999999999999\n";
        assert_matches_cpython_marshal(source, "large_bitwise.py");
    }

    #[test]
    fn matches_cpython_marshal_for_negative_large_integer_literals() {
        let source = "minimum_i64 = -9223372036854775808\nbelow_minimum_i64 = -9223372036854775809\nnegative_u64_max = -18446744073709551615\nnegative_u64_overflow = -18446744073709551616\n";
        assert_matches_cpython_marshal(source, "negative_large_integers.py");
    }

    #[test]
    fn matches_cpython_marshal_for_folded_negative_true_division() {
        let source = "fraction = -5 / 7\ntenth = -1 / 10\nlarge_denominator = -1 / 9_000_000_000_000_000_000\ndivision_by_zero = -1 / 0\n";
        assert_matches_cpython_marshal(source, "negative_true_division.py");
    }

    #[test]
    fn matches_cpython_marshal_for_folded_branches_in_a_with_statement() {
        let source = "from contextlib import suppress\n\ndef f():\n    with suppress(Exception):\n        if 1:\n            pass\n        elif 1:\n            pass\n        elif 1:\n            pass\n        try:\n            pass\n        except Exception:\n            pass\n        finally:\n            pass\n        if 2:\n            pass\n        while True:\n            pass\n";
        assert_matches_cpython_marshal(source, "protected_branches.py");
    }

    #[test]
    fn matches_cpython_marshal_for_coroutine_finally_condition() {
        let source = "async def f():\n    try:\n        await source\n    finally:\n        if test:\n            await other\n";
        assert_matches_cpython_marshal(source, "finally_condition.py");
    }

    #[test]
    fn matches_cpython_marshal_for_optimized_boolean_operands() {
        let source = "if a and f() and False and g():\n    pass\na or \"\" or True\na or () or True\na and \"value\" and False\na or (b or c) or d\n0 if a or [1] or True or [2] else 1\nif (\n    f()\n    is None\n):\n    pass\nif g() is not None:\n    pass\nx\n\ndef terminal_conditional_expression(wait):\n    call() if wait else None\n\nif first or low <= value < high:\n    pass\n\nmultiline = [\n    (\n        \"value\"\n        and condition\n        and result\n    )\n]\n";
        assert_matches_cpython_marshal(source, "bool.py");
    }

    #[test]
    fn matches_cpython_marshal_for_trailing_nested_boolean_locations() {
        let source = r"def conjunction(first, second, third):
    return first and (second and third)


def disjunction(first, second, third):
    return first or (second or third)


strip = lambda value: ((b'x' in value and value.strip()) or
                       ((b'x' not in value and value.lower()) or None))
";
        assert_matches_cpython_marshal(source, "trailing_nested_boolean_locations.py");
    }

    #[test]
    fn matches_cpython_marshal_for_unique_condition_fallthrough_locations() {
        let source = r#"def terminal_try(left, right):
    try:
        assert left and right
    except TypeError:
        pass


def terminal_branch(flag, left, right):
    if flag:
        assert left and right
    else:
        consume()


def protected(value):
    with manager(invalid="ignore"):
        result = transform(value)
        if result.real != 0 or result.imag != 0:
            raise AssertionError(format_result(result.real, result.imag))


def nested_join(flag, left, right):
    if flag:
        if not (left and right):
            raise ValueError
    elif alternate:
        consume()


def nested_loop(items, left, right):
    for item in items:
        while left or right:
            left, right = update(item)
"#;
        assert_matches_cpython_marshal(source, "unique_condition_fallthrough_locations.py");
    }

    #[test]
    fn matches_cpython_3145_boolean_exception_ranges() {
        let source = "async def choose(value):\n    return value or 'empty'\n\ntry:\n    body()\nexcept FirstError or SecondError as error:\n    recover(error)\n\ntry:\n    body()\nexcept FirstError and SecondError as error:\n    recover(error)\n";
        assert_matches_cpython_marshal(source, "boolean_exception_ranges.py");
    }

    #[test]
    fn matches_cpython_marshal_for_normalized_comparison_marker_owners() {
        let source = r"async def coroutine():
    await source
    if test == expected:
        await other


def generator(value):
    while value > 0:
        yield value
        value -= 1
";
        assert_matches_cpython_marshal(source, "normalized_comparison_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_boolean_value_marker_owners() {
        let source = "def generator_expression(items):\n    return (item or fallback for item in items)\n\ndef generator_value(value):\n    yield value\n    result = value and fallback\n    consume(result)\n\ndef generator_try(value):\n    yield value\n    try:\n        result = value or fallback\n        consume(result)\n    finally:\n        cleanup()\n\ndef ordinary_try(value):\n    try:\n        result = value or fallback\n        consume(result)\n    finally:\n        cleanup()\n\ndef nested_try(value):\n    try:\n        try:\n            result = value and fallback\n            consume(result)\n        finally:\n            inner_cleanup()\n    finally:\n        outer_cleanup()\n\ndef with_only(manager, value):\n    with manager:\n        result = value or fallback\n        consume(result)\n\ndef try_inside_with(manager, value):\n    with manager:\n        try:\n            result = value and fallback\n            consume(result)\n        finally:\n            cleanup()\n\ndef with_inside_try(manager, value):\n    try:\n        with manager:\n            result = value or fallback\n            consume(result)\n    finally:\n        cleanup()\n\ndef comprehension(items):\n    return [item and fallback for item in items]\n\ndef protected_comprehensions(manager, items):\n    try:\n        left = [item or fallback for item in items]\n        with manager:\n            right = {item and fallback for item in items}\n        return left, right\n    finally:\n        cleanup()\n\ndef generator_comprehension(items):\n    yield [item or fallback for item in items]\n\nasync def coroutine_before(value):\n    result = value or fallback\n    await source\n    consume(result)\n\nasync def coroutine_after(value):\n    await source\n    result = value and fallback\n    consume(result)\n\nasync def coroutine_try(value):\n    await source\n    try:\n        result = value or fallback\n        consume(result)\n    finally:\n        cleanup()\n\nasync def coroutine_with(manager, value):\n    await source\n    with manager:\n        result = value and fallback\n        consume(result)\n\nasync def async_generator(value):\n    yield value\n    result = value or fallback\n    consume(result)\n\nasync def async_generator_try(value):\n    yield value\n    try:\n        result = value and fallback\n        consume(result)\n    finally:\n        cleanup()\n";
        assert_matches_cpython_marshal(source, "boolean_value_marker_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_comprehension_boolean_stale_owners() {
        let source = "def unowned(items):\n    return [int(item or 0) for item in items]\n\ndef owned(items):\n    return [(int(timestamp), int(count or 0)) for count, timestamp in items]\n";
        assert_matches_cpython_marshal(source, "comprehension_boolean_stale_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_boolean_value_owner_edges() {
        let source = include_str!("fixtures/boolean_value_owner_edges.py");
        assert_matches_cpython_marshal(source, "boolean_value_owner_edges.py");
    }

    #[test]
    fn preserves_try_star_boolean_value_marker_owners() {
        let source = include_str!("fixtures/try_star_boolean_value_owners.py");
        assert_executes(source, "True\nTrue\n");
    }

    #[test]
    fn matches_cpython_marshal_for_path_sensitive_boolean_value_owners() {
        let source = include_str!("fixtures/boolean_value_owner_paths.py");
        assert_matches_cpython_marshal(source, "boolean_value_owner_paths.py");
    }

    #[test]
    fn matches_cpython_marshal_for_backward_boolean_assert_jumps() {
        let source = "def f(items):\n    for item in items:\n        assert (\"x\" in item) or (item in values), \"bad\"\n";
        assert_matches_cpython_marshal(source, "backward_bool.py");
    }

    #[test]
    fn matches_cpython_marshal_for_final_elif_short_circuit_loop_tails() {
        let source = r"def direct_and(items, first, second):
    for item in items:
        if item == 0:
            first()
        elif item and second:
            second()


def direct_not_or(items, first, second):
    while items:
        item = items.pop()
        if item == 0:
            first()
        elif not (item or second):
            second()


async def direct_async(items, first, second):
    async for item in items:
        if item == 0:
            first()
        elif item and second:
            second()


def nested_loop(items, first, second):
    for item in items:
        if item:
            for nested in item:
                if first:
                    first()
                elif nested and second:
                    second()
        elif second:
            second()
";
        assert_matches_cpython_marshal(source, "final_elif_short_circuit.py");
    }

    #[test]
    fn matches_cpython_marshal_for_non_final_branch_nested_loop_tails() {
        let source = r#"def for_loop(items):
    for item in items:
        if classify(item) == "first":
            if item:
                consume(item)
        elif classify(item) == "second":
            if item is not None:
                consume(item)
        else:
            fallback(item)


def while_loop(items):
    while items:
        item = items.pop()
        if is_first(item):
            if item:
                consume(item)
        elif is_second(item):
            consume(item)


def recursively_nested(items):
    for item in items:
        if is_first(item):
            if item:
                if accepted(item):
                    consume(item)
        elif is_second(item):
            consume(item)


def final_elif_in_non_final_branch(items):
    for item in items:
        if is_first(item):
            if accepted(item):
                consume(item)
            elif deferred(item):
                if ready(item):
                    consume(item)
        else:
            fallback(item)
"#;
        assert_matches_cpython_marshal(source, "non_final_branch_loop_tail.py");
    }

    #[test]
    fn matches_cpython_marshal_for_inherited_non_final_branch_loop_tails() {
        let source = r"def append_library(libraries, item):
    for existing in libraries:
        if is_sequence(existing):
            if is_sequence(item):
                if item[0] == existing[0]:
                    if item[1] is existing[1]:
                        return
                    warn(item[0])
                    break
            else:
                if item == existing[0]:
                    warn(item[0])
                    break
        else:
            if is_sequence(item):
                if item[0] == existing:
                    warn(item[0])
                    break
            else:
                if item == existing:
                    return
    libraries.append(item)


def final_else_control(items):
    for item in items:
        if is_sequence(item):
            consume(item)
        else:
            if accepted(item):
                consume(item)
";
        assert_matches_cpython_marshal(source, "inherited_non_final_branch_loop_tails.py");
    }

    #[test]
    fn uses_cpython_non_final_short_circuit_assignment_orientation() {
        let source = r#"import dis


def short_circuit_assignment(items, force, accepted, replacement):
    for item in items:
        if is_first(item):
            if force or item not in accepted:
                accepted[item] = replacement[item]
        else:
            fallback(item)


line = short_circuit_assignment.__code__.co_firstlineno + 3
print(" ".join(
    instruction.opname
    for instruction in dis.get_instructions(short_circuit_assignment)
    if instruction.positions.lineno == line
    and (
        instruction.opname.startswith("POP_JUMP")
        or instruction.opname == "JUMP_BACKWARD"
    )
))
"#;
        assert_executes(source, "POP_JUMP_IF_TRUE POP_JUMP_IF_FALSE JUMP_BACKWARD\n");
    }

    #[test]
    fn matches_cpython_marshal_for_loop_and_async_iteration_edges() {
        let source = include_str!("fixtures/benchmark/loop_async_edges.py");
        assert_matches_cpython_marshal(source, "loop_tail.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_async_for_in_with_bodies() {
        let source = "async def sync_with_direct(manager, items):\n    with manager:\n        async for item in items:\n            consume(item)\n\nasync def async_with_direct(manager, items):\n    async with manager:\n        async for item in items:\n            consume(item)\n\nasync def sync_then_async_with(sync_manager, async_manager, items):\n    with sync_manager:\n        async with async_manager:\n            async for item in items:\n                consume(item)\n\nasync def async_then_sync_with(async_manager, sync_manager, items):\n    async with async_manager:\n        with sync_manager:\n            async for item in items:\n                consume(item)\n\nasync def async_with_two_items(first, second, items):\n    async with first, second:\n        async for item in items:\n            consume(item)\n\nasync def async_with_inner_finally(manager, items):\n    async with manager:\n        try:\n            async for item in items:\n                consume(item)\n        finally:\n            cleanup()\n\nasync def async_with_nonfinal(manager, items):\n    async with manager:\n        async for item in items:\n            consume(item)\n        after()\n";
        assert_matches_cpython_marshal(source, "async_with_terminal_async_for.py");

        let source = "async def try_around_async_with(manager, items):\n    try:\n        async with manager:\n            async for item in items:\n                consume(item)\n    finally:\n        cleanup()\n";
        assert_matches_cpython_marshal(source, "try_finally_async_with.py");
    }

    #[test]
    fn matches_cpython_marshal_for_async_for_noop_body_backedges() {
        let source = "async def pass_body(items):\n    async for item in items:\n        pass\n\nasync def two_passes(items):\n    async for item in items:\n        pass\n        pass\n\nasync def docstring_and_pass(items):\n    async for item in items:\n        'docstring'\n        pass\n\nasync def nested(items):\n    async for outer in items:\n        async for inner in outer:\n            pass\n\nasync def with_else(items):\n    async for item in items:\n        pass\n    else:\n        pass\n";
        assert_matches_cpython_marshal(source, "async_for_noop_body.py");
    }

    #[test]
    fn matches_cpython_terminal_async_for_nop_ownership() {
        let source = "import dis\n\nasync def sync_with_direct(manager, items):\n    with manager:\n        async for item in items:\n            consume(item)\n\nasync def async_then_sync_with(async_manager, sync_manager, items):\n    async with async_manager:\n        with sync_manager:\n            async for item in items:\n                consume(item)\n\nasync def try_around_async_with(manager, items):\n    try:\n        async with manager:\n            async for item in items:\n                consume(item)\n    finally:\n        cleanup()\n\nasync def async_with_around_try(manager, items):\n    async with manager:\n        try:\n            async for item in items:\n                consume(item)\n        finally:\n            cleanup()\n\ndef has_end_nop(function):\n    instructions = list(dis.get_instructions(function))\n    return any(\n        left.opname == 'END_ASYNC_FOR' and right.opname == 'NOP'\n        for left, right in zip(instructions, instructions[1:])\n    )\n\nfor function in (sync_with_direct, async_then_sync_with, try_around_async_with, async_with_around_try):\n    print(has_end_nop(function))\n";
        assert_executes(source, "True\nTrue\nFalse\nTrue\n");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_boolean_comprehension() {
        let source = "foo or {x: None for x in bar}\n";
        assert_matches_cpython_marshal(source, "boolean_comprehension.py");
    }

    #[test]
    fn matches_cpython_marshal_for_short_circuit_comprehension_filters() {
        let source = "z = [a for a in range(5) if a or b or c or d and e]\n";
        assert_matches_cpython_marshal(source, "comprehension_filter.py");
    }

    #[test]
    fn matches_cpython_marshal_for_imported_attribute_calls_in_module_comprehensions() {
        let source = r#"import re
import re as captured_module
import re as inner_only_module
import re as lambda_default_module
import re as lambda_module
import re as later_target_module
import re as outer_lambda_module
import re as outer_target_module
import re as target_module
import re as target_expression_module
import re as walrus_module

ordinary = re.match("a", "a")
first_iterable = [item for item in re.split(",", "a,b")]
filtered = [item for item in ("a", "b") if re.match("a", item)]
elements = [re.split(",", item) for item in ("a,b", "c,d")]
dictionary = {item: re.compile(item) for item in ("a", "b")}
unique = {re.escape(item) for item in ("a", "b")}
nested = [
    re.match("a", inner)
    for outer in (1,)
    for inner in re.split(",", "a,b")
]
generator = (re.match("a", item) for item in ("a", "b"))

target_shadow = [
    target_module.match("a", "a")
    for target_module in (target_module,)
]
later_target_shadow = [
    later_target_module.match("a", "a")
    for item in (1,)
    for later_target_module in (later_target_module,)
]
target_expression = [
    None
    for target_expression_module.match("a", "a").target in (1,)
]
captured_target = [
    (lambda: captured_module.match("a", "a"))
    for captured_module in (captured_module,)
]
walrus_shadow = [
    walrus_module.match("a", item)
    for item in ("a",)
    if (walrus_module := walrus_module)
]
inner_only_values = (inner_only_module,)
inner_target_only = [
    [
        inner_only_module.match("a", "a")
        for inner_only_module in inner_only_values
    ]
    for outer in (1,)
]
outer_target_inner_free = [
    [outer_target_module.match("a", "a") for inner in (1,)]
    for outer_target_module in (outer_target_module,)
]
lambda_only = [
    (lambda: lambda_module.match("a", "a"))
    for item in (1,)
]
lambda_default = [
    (lambda value=lambda_default_module.match("a", "a"): value)
    for item in (1,)
]
outer_free_and_lambda = [
    (
        outer_lambda_module.match("a", "a"),
        lambda: outer_lambda_module.match("a", "a"),
    )
    for item in (1,)
]

def function_scope():
    return [re.match("a", item) for item in ("a", "b")]
"#;
        assert_matches_cpython_marshal(source, "imported_attribute_comprehensions.py");
    }

    #[test]
    fn matches_cpython_marshal_for_class_comprehension_global_lookups() {
        let source = r"GLOBAL = 10
groups = ((1, 2),)

def decorate(value):
    return lambda function: function

class Example:
    source = (1, 2, 3)
    callbacks = (str,)
    values = [GLOBAL + value for value in source]
    labels = {str(value) for value in source if bool(value)}
    mapping = {value: tuple((GLOBAL, value)) for value in source}
    nested = [[str(inner) for inner in range(outer)] for outer in source]
    called = [callback(value) for callback in callbacks for value in source]
    shadowed = [GLOBAL for GLOBAL in source]

def enclosing(captured):
    class Nested:
        values = [GLOBAL + captured + value for value in range(3)]

        def method(self):
            return __class__, captured

    return Nested

class Decorated:
    @decorate([item for item in group] for group in groups)
    def method(self):
        return None
";
        assert_matches_cpython_marshal(source, "class_comprehension_globals.py");
    }

    #[test]
    fn matches_cpython_marshal_for_conditional_class_annotations() {
        let source = r"flag = True

class Conditional:
    direct: str
    if flag:
        enabled: int
    else:
        disabled: bytes

class WithClassCell:
    if flag:
        member: __class__

    def method(self):
        return __class__
";
        assert_matches_cpython_marshal(source, "conditional_class_annotations.py");
    }

    #[test]
    fn matches_cpython_marshal_for_negated_generator_filters() {
        let source =
            "def filtered(items):\n    return (item for item in items if not predicate(item))\n";
        assert_matches_cpython_marshal(source, "negated_generator_filter.py");
    }

    #[test]
    fn matches_cpython_marshal_for_comparison_filter_cleanup() {
        let source = r"def generator_disjunction(items, left, right):
    return (item for item in items if item == left or item == right)

def generator_chained(items, lower, upper, excluded):
    return (
        item
        for item in items
        if lower < item < upper and item != excluded
    )

def generator_ranges(items, first, lower, upper, last):
    return (
        item
        for item in items
        if item == first or lower <= item <= upper or item == last
    )

def negated_inlined(items, marker, other):
    return [item for item in items if not (item is marker and item == other)]

def negated_generator(items, marker, other):
    return (item for item in items if not (item is marker and item == other))

";
        assert_matches_cpython_marshal(source, "comparison_filter_cleanup.py");
    }

    #[test]
    fn matches_cpython_marshal_for_applied_comprehension_cleanup_swaps() {
        let source = r"def applied(manager, condition, items):
    with manager:
        result = [item for item in items]
        if condition:
            return result

def discarded(manager, condition, items):
    with manager:
        {item for item in items}
        if condition:
            return

def retained(manager, condition, items):
    with manager:
        result = consume([item for item in items])
        if condition:
            return result
";
        assert_matches_cpython_marshal(source, "comprehension_cleanup_swap.py");
    }

    #[test]
    fn matches_cpython_marshal_for_maybe_null_comprehension_restores() {
        let source = r#"def assigned(items):
    target = "outer"
    [target for target in items]
    first(target)
    second(target)

def parameter(target, items):
    [target for target in items]
    first(target)
    second(target)

def with_target(manager, items):
    with manager as target:
        [target for target in items]
        first(target)
        second(target)

def unrelated(items):
    value = "outer"
    [target for target in items]
    first(value)
    second(value)

def rebound(items):
    target = "outer"
    [target for target in items]
    target = "new"
    first(target)
    second(target)

def discarded(items):
    target = "outer"
    {target for target in items}
    first(target)
    second(target)
"#;
        assert_matches_cpython_marshal(source, "maybe_null_comprehension_restore.py");
    }

    #[test]
    fn leaves_comprehension_cell_restores_definitely_initialized() {
        let source = r#"import dis

def cell_restore(items):
    target = "outer"
    [target for target in items]
    return (target for _ in items)

for instruction in dis.get_instructions(cell_restore):
    if instruction.argval == "target" and instruction.opname.startswith("LOAD_FAST"):
        print(instruction.opname)
"#;
        assert_executes(source, "LOAD_FAST_AND_CLEAR\nLOAD_FAST_BORROW\n");
    }

    #[test]
    fn matches_cpython_marshal_for_inlined_comprehension_cells() {
        let source = r"module_result = [lambda: x for x in range(2)]

def captured_targets():
    lists = [lambda: x for x in range(2)]
    sets = {lambda: x for x in range(2)}
    dictionaries = {x: lambda: x for x in range(2)}
    return lists, sets, dictionaries

def unreachable_target():
    if False:
        return [lambda: item for item in range(3)]

def nested_targets(xs, ys):
    return [[lambda: x for x in ys] for x in xs]

def target_rebound_as_later_cell(types, values):
    normalized = {normalize(type_) for type_ in types}
    for type_ in types:
        converted = (convert(value, type_) for value in values)
    return normalized, converted
";
        assert_matches_cpython_marshal(source, "comprehension_cells.py");
    }

    #[test]
    fn matches_cpython_marshal_for_class_scope_super_resolution() {
        let source = r"class Outer:
    def method(self):
        class Inner:
            super(Outer, self).method()

            def nested(inner_self):
                super(Outer, self).method()


class Annotated:
    def method(self):
        __class__: object
        super


class Shadowed:
    super = factory
    value = super(Shadowed, instance).method()
";
        assert_matches_cpython_marshal(source, "class_super.py");
    }

    #[test]
    fn matches_cpython_marshal_for_captured_implicit_super_receivers() {
        let source = r"class Base:
    def method(self):
        pass


class Child(Base):
    def captured_self(self):
        def capture():
            return self
        return super().method()

    @classmethod
    def captured_cls(cls):
        def capture():
            return cls
        return super().method()

    def ordinary(self):
        return super().method()

    def explicit(self):
        def capture():
            return self
        return super(Child, self).method()
";
        assert_matches_cpython_marshal(source, "captured_super.py");
    }

    #[test]
    fn matches_cpython_marshal_for_multiline_super_method_arguments() {
        let source = "class Base:\n    pass\n\nclass Child(Base):\n    def implicit(self, value):\n        return super().method(\n            value\n        )\n\n    def explicit(self, value):\n        return super(Child, self).method(\n            value\n        )\n\n    def keyword(self, value):\n        return super().method(\n            option=value,\n        )\n\n    def split_super(self, value):\n        return super(\n        ).method(\n            value,\n        )\n\n    def multiline_attribute(self):\n        return super().\\\n            value\n";
        assert_matches_cpython_marshal(source, "super_method_lines.py");
    }

    #[test]
    fn matches_cpython_marshal_for_large_call_thresholds() {
        let keywords = |count| {
            (0..count)
                .map(|index| format!("k{index}={index}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let positional = |count| {
            (0..count)
                .map(|index| index.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let keywords_15 = keywords(15);
        let keywords_16 = keywords(16);
        let keywords_28 = keywords(28);
        let keywords_29 = keywords(29);
        let positional_27 = positional(27);
        let positional_28 = positional(28);
        let positional_31 = positional(31);
        let source = format!(
            r"import imported

def method_sixteen(obj):
    return obj.method({keywords_16})

def global_sixteen():
    return target({keywords_16})

def global_one_and_fifteen():
    return target(0, {keywords_15})

def global_thirty_one():
    return target({positional_31})

def imported_one_and_fifteen():
    return imported.target(0, {keywords_15})

class Base:
    def method(self):
        pass

class Child(Base):
    def super_sixteen(self):
        return super().method({keywords_16})

    def super_twenty_eight_and_one(self):
        return super().method({positional_28}, option=0)

def method_twenty_eight(obj):
    return obj.method({keywords_28})

def method_twenty_nine(obj):
    return obj.method({keywords_29})

def method_twenty_seven_and_one(obj):
    return obj.method({positional_27}, option=0)

def method_twenty_eight_and_one(obj):
    return obj.method({positional_28}, option=0)
"
        );
        assert_matches_cpython_marshal(&source, "large_call_thresholds.py");
    }

    #[test]
    fn matches_cpython_marshal_for_starred_global_call_arguments() {
        let source = r"global module_values, module_callable
module_explicit_global = consume(*module_values)
module_load_name = consume(*module_name)
module_both_global = module_callable(*module_values)
module_multiline = consume(
    *module_values
)
module_named_keyword = consume(*module_values, option=True)

module_direct_global = module_callable()

class ClassScope:
    global class_values, class_callable
    explicit_global = consume(*class_values)
    load_name = consume(*class_name)
    both_global = class_callable(*class_values)
    named_keyword = consume(*class_values, option=True)

    direct_global = class_callable()


class AnnotationScope:
    type Alias = consume(*annotation_values)


def global_callable():
    return consume(*values)

def local_callable(consume):
    return consume(*values)

def local_argument(values):
    return consume(*values)

def bound_callable(obj):
    return obj.consume(*values)

def computed_callable():
    return make_consumer()(*values)

def comprehension_shadow():
    global values
    return [consume(*values) for values in collections]

def called_argument():
    return consume(*make_values())

def leading_argument():
    return consume(1, *values)

def multiple_arguments():
    return consume(*values, *more_values)

def keyword_unpacking():
    return consume(*values, **options)
";
        assert_matches_cpython_marshal(source, "starred_global_argument.py");
    }

    #[test]
    fn matches_cpython_marshal_for_optimized_generator_calls() {
        let source = "module_any = any(value for value in values)\nmodule_all = all(value for value in values)\nmodule_tuple = tuple(value for value in values)\nany(value for value in values)\nall(value for value in values)\ntuple(value for value in values)\nafter = None\n\n\ndef optimized(values):\n    return (\n        any(value for value in values),\n        all(value for value in values),\n        tuple(value for value in values),\n    )\n\n\ndef rebound(values, any):\n    return any(value for value in values)\n\n\ndef terminal_discard(values):\n    all(value for value in values)\n";
        assert_matches_cpython_marshal(source, "optimized_generator_calls.py");
    }

    #[test]
    fn matches_cpython_marshal_for_optimized_generator_calls_in_try_bodies() {
        let source = r"def direct_nonterminal(values):
    try:
        result = tuple(int(value) for value in values)
        if result:
            consume(result)
    except ValueError:
        recover()
";
        assert_matches_cpython_marshal(source, "optimized_generator_try.py");
    }

    #[test]
    fn matches_cpython_marshal_for_generator_guard_marker_owners() {
        let source = r"def after_resume(values):
    yield before()
    result = tuple(value for value in values)
    yield result


def while_fresh(values):
    yield before()
    while True:
        result = tuple(value for value in values)
        yield result


def after_dynamic(flag, values):
    yield before()
    if flag:
        prepare()
        result = tuple(value for value in values)
        yield result


def iterator_owned(groups):
    yield before()
    for values in groups:
        result = tuple(value for value in values)
        yield result


def consecutive_fresh(values):
    yield before()
    while True:
        first = tuple(value for value in values)
        second = tuple(value for value in values)
        yield first, second
";
        assert_matches_cpython_marshal(source, "generator_guard_marker_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_boolean_optimized_generator_calls() {
        let source = "def disjunction(prefix, values):\n    return prefix or any(value for value in values)\n\ndef conjunction(prefix, values):\n    return prefix and all(value for value in values)\n\ndef nested(first, second, values):\n    return first or second or any(value for value in values)\n\nwrapped = lambda first, second, values: first and second and all(value for value in values)\n";
        assert_matches_cpython_marshal(source, "boolean_generator_calls.py");
    }

    #[test]
    fn matches_cpython_marshal_for_branch_result_ownership() {
        let source = "def ternary(name, suffix):\n    return name[:-len(suffix)] if name.endswith(suffix) else name\n\ndef simple(condition, left, right):\n    return left if condition else right\n\ndef terminal_attribute(condition, holder, left, right):\n    holder.value = left.value if condition else right\n\ndef boolean(left, right):\n    return left or right\n\ndef nested(condition, name):\n    return name.upper() if condition else name\n\ndef nested_boolean(left, middle, right):\n    return (left and middle) or right, (left or middle) and right\n\ndef folded_annotation(arg: 2 or None | int):\n    pass\n\nlambda_conditional = lambda condition, left, right: left if condition else right\nlambda_boolean = lambda left, right: left or right\nmodule_boolean = (left and middle) or right\n";
        assert_matches_cpython_marshal(source, "ownership.py");
    }

    #[test]
    fn matches_cpython_marshal_for_recursive_tail_conditional_lambdas() {
        let source = r"lambda_else = lambda outer, inner, first, second, third: (
    first if outer else second if inner else third
)
lambda_body = lambda outer, inner, first, second, third: (
    (first if inner else second) if outer else third
)


def assignment_control(holder, outer, inner, first, second, third):
    holder.value = (
        (first if inner else second) if outer else third
    )
";
        assert_matches_cpython_marshal(source, "recursive_tail_conditional_lambdas.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_folded_dictionary_comprehension_key() {
        let source = "def folded_key():\n    return {x if True else y: y for x in range(10) for y in range(10)}\n";
        assert_matches_cpython_marshal(source, "folded_key.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_named_comprehension_target() {
        let source = "[value for value in values]\n{key: (left, right) for key, (left, right) in values}\nmodule_result = {last := value for value in range(2)}\nnested_filter = [(key, value) for key, value in pairs if key in [field.name for field in fields]]\nnested_iterable = [(left, right) for left in values for right in [item.value for item in items]]\ngenerator_assignment = list((captured := value for value in values))\n\ndef f():\n    exponential, base_multiplier = 1, 2\n    hash_map = {\n        (exponential := (exponential * base_multiplier) % 3): i + 1\n        for i in range(2)\n    }\n    return hash_map\n";
        assert_matches_cpython_marshal(source, "named_comprehension.py");
    }

    #[test]
    fn matches_cpython_marshal_for_named_expression_local_order() {
        let source = r"def chained():
    return (outer := (inner := value))

lambda_body = lambda: (lambda_outer := (lambda_inner := lambda_value))

def mixed_global():
    global global_outer
    result = (global_outer := (local_inner := local_value))
    local_value = 1
    return result

def list_element_late_load(items):
    result = [(outer := (inner := seed)) for item in items]
    seed = 1
    return result

def list_two_filters(items):
    result = [item for item in items if (first := left) if (second := right)]
    left = right = 1
    return result

def set_element(items):
    result = {(outer := (inner := seed)) for item in items}
    seed = 1
    return result

def dict_key_then_value(items):
    result = {
        (key_target := key_seed): (value_target := value_seed)
        for item in items
    }
    key_seed = value_seed = 1
    return result

def nested_list(rows):
    result = [[(outer := (inner := seed)) for item in row] for row in rows]
    seed = 1
    return result

def generator_reverse_names(items):
    return ((a_outer := (z_inner := item)) for item in items)

def decorated():
    @(decorator_outer := (decorator_inner := decorator_value))
    def nested():
        pass

    decorator_value = 1
    return nested
";
        assert_matches_cpython_marshal(source, "named_expression_local_order.py");
    }

    #[test]
    fn matches_cpython_marshal_for_try_else_local_order() {
        let source = r"def ordered():
    try:
        initial = produce()
    except Exception as error:
        handler_only = fallback()
        result = handler_only
    else:
        result = success(initial)
    return result


def no_else_control():
    try:
        initial = produce()
    except Exception as error:
        handler_only = fallback()
    return initial
";
        assert_matches_cpython_marshal(source, "try_else_local_order.py");
    }

    #[test]
    fn matches_cpython_marshal_for_multiline_conditional_expressions() {
        let source = "value = (\n    'body'\n    if True\n    else 'otherwise'\n)\n\ndef nested():\n    target.attribute = (\n        (first if inner else second)\n        if outer\n        else third\n    )\n";
        assert_matches_cpython_marshal(source, "conditional_expression.py");
    }

    #[test]
    fn matches_cpython_marshal_for_conditional_expression_join_locations() {
        let source = r"def split_short_circuit(manager, value):
    manager.first = (value.items or []) if has_value(
        value, 'items') else []
    manager.second = (value.other or []) if has_value(
        value, 'other') else []


def same_block_control(condition, body, otherwise):
    return body if condition else otherwise


def call_control(condition):
    return body() if condition else otherwise()
";
        assert_matches_cpython_marshal(source, "conditional_expression_join_locations.py");
    }

    #[test]
    fn matches_cpython_marshal_for_unpacked_class_arguments() {
        let source = "class C1(Generic[T], str, **{'metaclass': type}):\n    ...\n\nclass C2(Generic[T], str, metaclass=type):\n    ...\n\nclass C3(Generic[T], metaclass=type, *[str]):\n    ...\n\nclass C4(x=1, **y):\n    ...\n";
        assert_matches_cpython_marshal(source, "class_arguments.py");
    }

    #[test]
    fn matches_cpython_marshal_for_import_originated_calls() {
        let source = "import sys\n\ndef reimported():\n    import sys\n    sys.exit(1)\n\ndef shadowed(sys):\n    sys.exit(1)\n\ndef only_local():\n    import os\n    os._exit(1)\n";
        assert_matches_cpython_marshal(source, "imported_calls.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_nested_if_branch_exit() {
        let source = "seed = 0\nif True:\n    body()\ndef between():\n    pass\nreused = True\n\nif a:\n    if b:\n        body()\nelif c:\n    other()\nafter1(); after2(); after3(); after4(); after5(); after6()\n\nif x:\n    if True:\n        body()\nelif y:\n    other()\n\nif False if True else False:\n    body()\nelif True:\n    other()\nelse:\n    unreachable()\n\ndef conditional_in_loop(src, dst):\n    for k, v in src:\n        if True if True else False:\n            dst[k] = v\n\ndef terminating_nested_if(x, y):\n    if x:\n        if y:\n            raise Exception()\n        else:\n            body()\n    else:\n        other()\n    after()\n\ndef nested_if_before_implicit_return(x, y):\n    if x:\n        if y:\n            return\n        else:\n            value = 2\n    else:\n        value = 3\n    return\n\ndef nested_if_with_fallthrough_return(x, y):\n    if x:\n        if y:\n            return y\n    else:\n        return x\n    return None\n\ndef nested_if_before_loop_control(items, condition):\n    for item in items:\n        if item:\n            if condition:\n                value = 1\n            else:\n                value = 2\n            continue\n        else:\n            value = 3\n        continue\n\ndef protected_loop_control(items):\n    for item in items:\n        try:\n            body()\n            if item:\n                break\n        except Exception:\n            recover()\n    for item in items:\n        try:\n            body()\n            continue\n        except Exception:\n            recover()\n\ndef nested_break_then_return(items, condition):\n    for item in items:\n        if item:\n            if condition:\n                break\n        else:\n            return condition\n        return None\n\nif True:\n    if first:\n        one()\n    elif second:\n        two()\n";
        assert_matches_cpython_marshal(source, "nested_if.py");
    }

    #[test]
    fn matches_cpython_marshal_for_an_assertion_branch_exit() {
        let source = r#"def select(result, states, failed):
    if failed:
        assert states == {"disabled"}
    else:
        assert states == {"normal"}

    return result
"#;
        assert_matches_cpython_marshal(source, "assert_branch_exit.py");
    }

    #[test]
    fn matches_cpython_marshal_for_later_assertion_branches_before_function_exits() {
        let source = r"def before_return(first, second):
    if first:
        consume()
    elif second:
        assert second.value
    else:
        consume()
    return


def before_raise(first, second):
    if first:
        consume()
    elif second:
        assert second.value
    else:
        consume()
    raise RuntimeError


def ordinary_successor_control(first, second):
    if first:
        consume()
    elif second:
        assert second.value
    else:
        consume()
    after()


def terminating_else_control(first, second):
    if first:
        consume()
    elif second:
        assert second.value
    else:
        return
    after()


def terminating_else_before_return(first, second):
    if first:
        consume()
    elif second:
        assert second.value
    else:
        return
    return


def loop_controls(items, first, second):
    for item in items:
        if first:
            consume()
        elif second:
            assert second.value
        else:
            consume()
        break
    for item in items:
        if first:
            consume()
        elif second:
            assert second.value
        else:
            consume()
        continue
";
        assert_matches_cpython_marshal(source, "later_assertion_function_exits.py");
    }

    #[test]
    fn matches_cpython_marshal_for_constant_false_assertions_in_final_else_suites() {
        let source = r#"def direct(condition, value):
    if condition:
        return
    else:
        assert False, value


def chained(kind, value):
    if kind == "first":
        first()
    elif kind == "second":
        second()
    else:
        assert 0, f"unknown: {value}"


def control(value):
    assert False, value
"#;
        assert_matches_cpython_marshal(source, "constant_false_assertion_else.py");
    }

    #[test]
    fn matches_cpython_marshal_for_assertion_chained_comparison_exit_nops() {
        let source = r#"def direct_chain(first, middle, last):
    assert first.value < middle.value < last.value


def long_chain(first, second, third, fourth):
    assert first.value == second.value == third.value == fourth.value


def chain_with_message(first, middle, last):
    assert first.value <= middle.value <= last.value, "out of order"


def simple_comparison(first, last):
    assert first.value < last.value


def non_assertion_chain(first, middle, last):
    if first.value < middle.value < last.value:
        consume()
    after()
"#;
        assert_matches_cpython_marshal(source, "assert_chained_comparison_exit_nops.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_negated_chained_comparison_exit_nops() {
        let source = r#"def direct_raise(low, value, high):
    if not (low <= value < high):
        raise IndexError
    consume()


def prefixed_elif(kind, enabled, low, value, high):
    if kind == "simple":
        consume()
    elif enabled and not (low <= value < high):
        raise IndexError
    finish()


def direct_return(low, value, high):
    if not (low <= value < high):
        return value
    consume()


def two_negated_chains(a, b, c, d, e, f):
    if not (a < b < c) and not (d < e < f):
        raise ValueError
    finish()


def terminating_else(low, value, high):
    if not (low <= value < high):
        raise IndexError
    else:
        raise ValueError


def loop_raise_after_try(items, enabled, low, value, high):
    for item in items:
        try:
            prepare(item)
        except ValueError:
            recover()
        if item is None:
            consume(item)
        elif enabled and not (low <= value < high):
            raise IndexError
        finish(item)


def final_operand_control(low, value, high, enabled):
    if not (low <= value < high) and enabled:
        raise IndexError
    consume()


def multi_statement_control(low, value, high):
    if not (low <= value < high):
        consume()
        raise IndexError
    finish()


def loop_controls(items, low, value, high):
    for item in items:
        if not (low <= value < high):
            break
        consume(item)
    for item in items:
        if not (low <= value < high):
            continue
        consume(item)
"#;
        assert_matches_cpython_marshal(source, "terminal_negated_chained_comparison_exit_nops.py");
    }

    #[test]
    fn matches_cpython_marshal_for_final_none_chained_comparison_jumps() {
        let source = r#"class Example:
    def __init__(self, first, second=None, prefix=None):
        if second is prefix is None:
            self.value = first
            self.first = None
            self.second = None
        else:
            self.value = prefix
            self.first = first
            self.second = second

    def render(self):
        first = self.first
        second = self.second
        if first is second is None:
            return self.value
        else:
            return f"{first}: {second}"
"#;
        assert_matches_cpython_marshal(source, "final_none_chained_comparison.py");
    }

    #[test]
    fn matches_cpython_marshal_for_final_try_assertion_pop_blocks() {
        let source = r#"def direct(value):
    try:
        assert value
    except ValueError:
        recover()
    return value


def message(value):
    try:
        assert value, "bad"
    except ValueError:
        recover()
    return value


def with_else(value):
    try:
        assert value
    except ValueError:
        recover()
    else:
        consume(value)
    return value


def multiple_handlers(value):
    try:
        assert value
    except ValueError:
        recover()
    except TypeError:
        recover()
    return value


def nonfinal(value):
    try:
        assert value
        consume(value)
    except ValueError:
        recover()
    return value


def constant_true(value):
    try:
        assert True
    except ValueError:
        recover()
    return value


def constant_false(value):
    try:
        assert False
    except AssertionError:
        recover()
    return value


def boolean_and(left, right):
    try:
        assert left and right
    except ValueError:
        recover()
    return right


def boolean_or(left, right):
    try:
        assert left or right
    except ValueError:
        recover()
    return right


def conditional(value, alternate, flag):
    try:
        assert value if flag else alternate
    except ValueError:
        recover()
    return value


def protected(manager, value):
    with manager:
        try:
            assert value
        except ValueError:
            recover()
    return value


def protected_loop(manager, values):
    with manager:
        for value in values:
            try:
                assert value
            except ValueError:
                recover()


def terminal(value):
    try:
        assert value
    except ValueError:
        recover()


def generator(value):
    try:
        assert value
    except ValueError:
        recover()
    yield value


async def coroutine(value):
    try:
        assert value
    except ValueError:
        recover()
    return value
"#;
        assert_matches_cpython_marshal(source, "final_try_assertions.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_if_final_clause_exits() {
        let source = r"def final_else(outer, inner):
    if outer:
        if inner:
            first()
        else:
            second()
    else:
        other()
    after()

def terminal_final_elif(outer, inner, last):
    if outer:
        if inner:
            first()
        elif last:
            raise Error()
    else:
        other()
    after()

def nonterminal_final_elif(outer, inner, last):
    if outer:
        if inner:
            raise Error()
        elif last:
            body()
    else:
        other()
    after()

def terminal_final_else(outer, inner):
    if outer:
        if inner:
            first()
        else:
            raise Error()
    else:
        other()
    after()

def no_else_nonterminal(outer, inner):
    if outer:
        if inner:
            body()
    else:
        other()
    after()

def folded_nested(outer):
    if outer:
        if True:
            body()
    else:
        other()
    after()

def threaded_three_way_cascade(major, minor, bugfix, other_major, other_minor, other_bugfix):
    if major == other_major:
        if minor == other_minor:
            if bugfix == other_bugfix:
                result = 0
            elif bugfix > other_bugfix:
                result = 1
            else:
                result = -1
        elif minor > other_minor:
            result = 1
        else:
            result = -1
    elif major > other_major:
        result = 1
    else:
        result = -1
    return result

def cascade_with_leading_statement(outer, middle, inner):
    if outer:
        prepare()
        if middle:
            if inner:
                first()
            elif alternate():
                second()
            else:
                third()
        elif alternate():
            fourth()
        else:
            fifth()
    elif alternate():
        sixth()
    else:
        seventh()
    after()
";
        assert_matches_cpython_marshal(source, "nested_if_final_clause.py");
    }

    #[test]
    fn matches_cpython_marshal_for_an_unreachable_assertion_message() {
        let source = "seed = '\u{a0}'\nassert True, print('hidden', print.__name__, sep=None)\ncondition = 'used'\n\ndef loop_tail_assert(values):\n    for value in values:\n        assert value\n";
        assert_matches_cpython_marshal(source, "assert.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_formatted_not_equal_t_string() {
        let source = "t\"{3!=4:}\"\n";
        assert_matches_cpython_marshal(source, "t_string.py");
    }

    #[test]
    fn matches_cpython_marshal_for_interpolated_string_boundaries() {
        let source = r#"
def render(item, nested):
    rf'raw\path {item}'
    f'''first
{item!r}
# literal
last'''
    r'left' f"middle {item}" '''right'''
    t'left' tr"""right {item}
# literal"""
    f"""start {(
        {'}': nested['{']}  # expression comment
    ) = } end"""
    t"""{(
        nested["key"]  # expression comment
    ) = }"""
"#;
        assert_matches_cpython_marshal(source, "interpolated_string_boundaries.py");
    }

    #[test]
    fn matches_cpython_marshal_for_debug_interpolation_text() {
        let source = r#"
def render(item, mapping, width):
    f"{  item   =   !r}"
    t"{  item   =   !r}"
    t"""{(
        mapping["="]  # = inside comment
    )   =   :>{width}}"""
    t"""{f"{item=} = literal" = !s:^10}"""
    f"""{item=  # after marker
}"""
    t"""{item=  # after marker
}"""
"#;
        assert_matches_cpython_marshal(source, "debug_interpolation_text.py");
    }

    #[test]
    fn matches_cpython_marshal_for_comment_trivia_in_interpolations() {
        let source = r##"
def render(value):
    f"""{(
        '# single',
        "# double",
        r'# raw',
        '''# triple''',
        """# triple double""",
    ) =  # debug comment
    }"""
    t"""{"escaped \\\" # still string" = }"""
    t"""{f"nested # {value}" = !r:^20}"""
    t"""{(
        '# literal',
        f"nested # {value}",
    )  # expression comment
    !r:^20}"""
"##;
        assert_matches_cpython_marshal(source, "comment_trivia_interpolations.py");
    }

    #[test]
    fn matches_cpython_marshal_for_an_irrefutable_constant_match_guard() {
        let source = "match subject:\n    case value if True:\n        pass\n    case _:\n        unreachable()\nafter()\n";
        assert_matches_cpython_marshal(source, "match.py");
    }

    #[test]
    fn matches_cpython_marshal_for_sequence_wildcard_star() {
        let source = "def prefix(value):\n    match value:\n        case [captured, *_]:\n            return captured\n\ndef suffix(value):\n    match value:\n        case [*_, captured]:\n            return captured\n\ndef both(value):\n    match value:\n        case [first, *_, last]:\n            return first, last\n\ndef nested(value):\n    match value:\n        case [[captured], *_]:\n            return captured\n";
        assert_matches_cpython_marshal(source, "sequence_wildcard.py");
    }

    #[test]
    fn matches_cpython_marshal_for_yield_in_try_finally() {
        let source = "def generator(flag):\n    if flag:\n        try:\n            yield\n        finally:\n            pass\n    else:\n        yield\n\ndef terminating_try_except_finally():\n    try:\n        raise StopIteration\n    except ValueError:\n        yield 1\n    finally:\n        pass\n";
        assert_matches_cpython_marshal(source, "generator.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_generator_qualnames() {
        let source = "def nested(groups):\n    return (sum(item for item in group) for group in groups)\n\ndef lambda_inside(values):\n    return ((lambda: value) for value in values)\n\nresult = consume(\n    # leading\n    item for item in values\n    # trailing\n)\n";
        assert_matches_cpython_marshal(source, "nested_generators.py");
    }

    #[test]
    fn matches_cpython_marshal_for_local_borrowing_across_exception_regions() {
        let source = "def try_return():\n    try:\n        value = process()\n        return value\n    except ValueError:\n        pass\n\ndef with_return(manager):\n    with manager:\n        return 1\n\ndef match_subject(provided):\n    match provided:\n        case True:\n            return captured\n\ndef comprehension(collection):\n    return [element for element in collection if element is not None]\n\ndef repeated_checked_load():\n    value = value[value.attribute == 'key']\n\ndef maybe_unbound(condition):\n    if condition:\n        value = 1\n    else:\n        use()\n    return value\n\ndef consecutive_try_imports():\n    try:\n        from first import First\n    except ImportError:\n        from fallback import First\n    First()\n    try:\n        from second import Second\n    except ImportError:\n        from fallback import Second\n    Second()\n\ndef folded_false(value):\n    if value == 1:\n        return 1\n    elif False:\n        return 2\n    elif value == 3:\n        return 3\n\ndef loaded_before_binding():\n    for item in values:\n        use(values, item)\n    values = []\n\ndef conditional_import(flag):\n    values = []\n    if flag:\n        values.append(flag)\n    from package import imported\n    return values, imported\n";
        assert_matches_cpython_marshal(source, "borrowing.py");
    }

    #[test]
    fn matches_cpython_marshal_for_try_with_normal_exits() {
        let source = r"def pass_body(value, manager):
    try:
        with manager:
            pass
    except Error:
        pass
    consume(value)

def active_body(value, manager):
    try:
        with manager:
            action()
    except Error:
        pass
    consume(value)

def following_try(value, manager):
    try:
        with manager:
            action()
    except Error:
        pass
    try:
        consume(value)
    except OtherError:
        raise

def terminal_handler(value, manager):
    try:
        with manager:
            action()
    except Error:
        return
    consume(value)

def nested_try(value, manager):
    try:
        try:
            with manager:
                action()
        except InnerError:
            recover()
        consume(value)
    except OuterError:
        recover_outer()
    return value

def finally_guard(value, manager):
    try:
        with manager:
            action()
    finally:
        cleanup()
    try:
        consume(value)
    except Error:
        raise

def combined_loop(manager, items):
    for item in items:
        try:
            with manager:
                action(item)
        except Error:
            recover()
        finally:
            cleanup()

def combined_following(manager):
    try:
        with manager:
            action()
    except Error:
        recover()
    finally:
        cleanup()
    after()

def combined_terminal(manager):
    try:
        with manager:
            action()
    except Error:
        recover()
    finally:
        cleanup()

def ordinary_guard(value):
    try:
        action()
    except Error:
        pass
    try:
        consume(value)
    except OtherError:
        raise
";
        assert_matches_cpython_marshal(source, "try_with_normal_exits.py");
    }

    #[test]
    fn uses_cpython_owned_loads_after_nested_handler_cold_joins() {
        let source = r#"import dis


def direct_handler_tail(value):
    try:
        result = action()
    except Error:
        try:
            recover()
        except OtherError:
            alternate()
    consume(value)
    return result


def conditional_handler_tail(value, flag):
    try:
        result = action()
    except Error:
        if flag:
            try:
                recover()
            except OtherError:
                alternate()
        else:
            fallback()
    consume(value)
    return result


def loop_handler_tail(value, items):
    try:
        result = action()
    except Error:
        for item in items:
            if item:
                try:
                    recover(item)
                except OtherError:
                    alternate(item)
        result = fallback()
    consume(value)
    return result


def load_after_consume(function):
    instructions = list(dis.get_instructions(function))
    consume = next(
        index
        for index, instruction in enumerate(instructions)
        if instruction.opname == "LOAD_GLOBAL" and instruction.argval == "consume"
    )
    return instructions[consume + 1].opname


for function in (direct_handler_tail, conditional_handler_tail, loop_handler_tail):
    print(load_after_consume(function))
"#;
        assert_executes(source, "LOAD_FAST\nLOAD_FAST\nLOAD_FAST\n");
    }

    #[test]
    fn uses_cpython_borrow_kinds_after_terminal_nested_handlers() {
        let source = r#"import dis


def terminal_nested_handler(value):
    try:
        result = action()
    except Error:
        try:
            recover()
        except OtherError:
            return value
        result = fallback()
    consume(value)
    return result


def fallthrough_nested_handler(value):
    try:
        result = action()
    except Error:
        try:
            recover()
        except OtherError:
            alternate()
        result = fallback()
    consume(value)
    return result


def load_after_consume(function):
    instructions = list(dis.get_instructions(function))
    consume = next(
        index
        for index, instruction in enumerate(instructions)
        if instruction.opname == "LOAD_GLOBAL" and instruction.argval == "consume"
    )
    return instructions[consume + 1].opname


print(load_after_consume(terminal_nested_handler))
print(load_after_consume(fallthrough_nested_handler))
"#;
        assert_executes(source, "LOAD_FAST\nLOAD_FAST_BORROW\n");
    }

    #[test]
    fn matches_cpython_marshal_for_empty_try_exit_borrow_boundaries() {
        let source = r"def fallthrough_if(value, flag):
    try:
        if flag:
            consume()
    except Exception:
        pass
    consume(value)
    return value


def terminal_if(value, flag):
    try:
        if flag:
            return value
    except Exception:
        pass
    consume(value)
    return value


def followed_in_try(value, flag):
    try:
        if flag:
            consume()
        other()
    except Exception:
        pass
    consume(value)
    return value


def plain_try(value):
    try:
        consume()
    except Exception:
        pass
    consume(value)
    return value


def shared_outer_join(value, flag, outer):
    if outer:
        try:
            if flag:
                consume()
        except Exception:
            pass
    consume(value)
    return value


def unthreaded_nested_if(value, flag, inner):
    try:
        if flag:
            if inner:
                return value
            else:
                consume()
    except Exception:
        pass
    consume(value)
    return value


def fallthrough_if_in_else(value, flag):
    try:
        consume()
    except Exception:
        pass
    else:
        if flag:
            consume()
    consume(value)
    return value


def complete_if_in_else(value, flag):
    try:
        consume()
    except Exception:
        pass
    else:
        if flag:
            consume()
        else:
            recover()
    consume(value)
    return value
";
        assert_matches_cpython_marshal(source, "empty_try_exit_borrow.py");
    }

    #[test]
    fn uses_cpython_borrow_kinds_for_threaded_empty_try_exits() {
        let source = r#"import dis


def threaded_nested_try(value, flag):
    try:
        if flag:
            try:
                consume()
            except ValueError:
                pass
    except Exception:
        pass
    consume(value)


def threaded_nested_with(value, flag, manager):
    try:
        if flag:
            with manager:
                consume()
    except Exception:
        pass
    consume(value)


def threaded_nested_if(value, flag, inner):
    try:
        if flag:
            if inner:
                consume()
            else:
                other()
    except Exception:
        pass
    consume(value)


def threaded_loop_break(value, flag, inner, items):
    try:
        if flag:
            for item in items:
                if inner:
                    break
                consume(item)
    except Exception:
        pass
    consume(value)


def threaded_match(value, flag, subject):
    try:
        if flag:
            match subject:
                case True:
                    consume()
                case _:
                    other()
    except Exception:
        pass
    consume(value)


def value_load(function):
    loads = [
        instruction.opname
        for instruction in dis.get_instructions(function)
        if instruction.argval == "value"
        and instruction.opname.startswith("LOAD_FAST")
    ]
    assert len(loads) == 1, loads
    return loads[0]


for function in (
    threaded_nested_try,
    threaded_nested_with,
    threaded_nested_if,
    threaded_loop_break,
    threaded_match,
):
    print(value_load(function))
"#;
        assert_executes(
            source,
            "LOAD_FAST_BORROW\nLOAD_FAST_BORROW\nLOAD_FAST_BORROW\nLOAD_FAST_BORROW\nLOAD_FAST_BORROW\n",
        );
    }

    #[test]
    fn matches_cpython_marshal_for_borrowing_after_constant_loop_breaks() {
        let source = r"def unprotected(flag, result):
    while True:
        if flag:
            break
        step()
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def copied_tiny_exit(flag, result):
    while True:
        step()
        if flag:
            break
    return result

def generator(flag, result):
    while True:
        if flag:
            break
        yield step()
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def tail_break(flag, result):
    while True:
        step()
        if flag:
            break
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def nested_terminal_break(flag, result, next_item, stop):
    while True:
        outer = next_item()
        if outer is None:
            break
        if flag:
            while True:
                item = next_item()
                if item is None:
                    break
                consume(item)
                if stop(item):
                    break
        first(result)
        second(result)
        third(result)
        fourth(result)
        fifth(result)
    return result

async def coroutine_terminal_break(result, read, stop):
    while True:
        item = await read()
        if item is None:
            break
        consume(item)
        if stop(item):
            break
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def earlier_and_tail_break(first_flag, last_flag, result):
    while True:
        if first_flag:
            break
        step()
        if last_flag:
            break
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result
";
        assert_matches_cpython_marshal(source, "constant_loop_break_borrowing.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_constant_loop_if_alternatives() {
        let source = r"def simple_final_if(flag, result):
    while True:
        step()
        if flag:
            break
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def final_if_with_fallthrough(result):
    while True:
        value = read()
        if value is None:
            break
        elif value is ignored:
            pass
        else:
            consume(value)
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result
";
        assert_matches_cpython_marshal(source, "terminal_constant_loop_if_alternatives.py");
    }

    #[test]
    fn matches_cpython_load_ownership_for_constant_loop_exit_shapes() {
        let source = r"import dis

def break_inside_try(fp, size, result):
    data = b''
    while True:
        try:
            chunk = fp.read(size - len(data))
            data += chunk
            if len(chunk) == 0 or len(data) == size:
                break
        except BlockingIOError:
            pass
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def break_inside_with(lock, result):
    while True:
        with lock:
            if ready():
                break
        wait()
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def breaking_fallthrough_alternative(first_flag, last_flag, result):
    while True:
        if first_flag:
            break
        else:
            step()
            if last_flag:
                break
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def final_else_break(first_flag, second_flag, result):
    while True:
        if first_flag:
            first(result)
        elif second_flag:
            second(result)
        else:
            break
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def result_loads(function):
    return [
        instruction.opname
        for instruction in dis.get_instructions(function)
        if instruction.argval == 'result'
        and instruction.opname.startswith('LOAD_FAST')
    ]

for function in (
    break_inside_try,
    break_inside_with,
    breaking_fallthrough_alternative,
    final_else_break,
):
    print(','.join(result_loads(function)))
";
        assert_executes(
            source,
            "LOAD_FAST_BORROW,LOAD_FAST_BORROW,LOAD_FAST_BORROW,LOAD_FAST_BORROW,LOAD_FAST_BORROW,LOAD_FAST_BORROW\nLOAD_FAST_BORROW,LOAD_FAST_BORROW,LOAD_FAST_BORROW,LOAD_FAST_BORROW,LOAD_FAST_BORROW,LOAD_FAST_BORROW\nLOAD_FAST,LOAD_FAST,LOAD_FAST,LOAD_FAST,LOAD_FAST,LOAD_FAST\nLOAD_FAST_BORROW,LOAD_FAST_BORROW,LOAD_FAST,LOAD_FAST,LOAD_FAST,LOAD_FAST,LOAD_FAST,LOAD_FAST\n",
        );
    }

    #[test]
    fn matches_cpython_marshal_for_superinstruction_location_propagation() {
        let source = r#"def load_pair(first, second, item):
    consume(first, second, message=("Expected %s" % item))

def store_pair(items):
    return ["pair %s/%s" % (first, second) for first, second in items]

def reversed_store_pair(first, second):
    left, right = convert(first), convert(second)
    return "pair %s/%s" % (left, right)
"#;
        assert_matches_cpython_marshal(source, "superinstruction_locations.py");
    }

    #[test]
    fn matches_cpython_marshal_for_borrowing_after_terminal_exception_handlers() {
        let source = "def returning(dependencies):\n    try:\n        before = load()\n        after = load()\n    except Exception:\n        return\n    for dependency in dependencies:\n        if dependency in after:\n            consume(dependency, before)\n\ndef reraising(dependencies):\n    try:\n        before = load()\n        after = load()\n    except Exception:\n        raise\n    for dependency in dependencies:\n        if dependency in after:\n            consume(dependency, before)\n";
        assert_matches_cpython_marshal(source, "terminal_handlers.py");
    }

    #[test]
    fn matches_cpython_marshal_for_name_returns_after_terminal_exception_handlers() {
        let source = r"def direct():
    try:
        value = produce()
    except Exception:
        raise RuntimeError
    return value


def nested():
    try:
        try:
            value = produce()
        except SyntaxError:
            raise SyntaxError
        return value
    except Exception:
        raise
";
        assert_matches_cpython_marshal(source, "terminal_handler_name_return.py");
    }

    #[test]
    fn matches_cpython_marshal_for_fallthrough_handler_finally_borrows() {
        let source = r"def pass_handler(value):
    try:
        work()
    except Error:
        pass
    finally:
        consume(value)


def work_handler(value):
    try:
        work()
    except Error:
        recover()
    finally:
        consume(value)


def two_fallthrough_handlers(value):
    try:
        work()
    except ValueError:
        recover()
    except Error:
        other()
    finally:
        consume(value)


def handler_with_else(value):
    try:
        work()
    except Error:
        recover()
    else:
        success()
    finally:
        consume(value)


def no_handler(value):
    try:
        work()
    finally:
        consume(value)


def raise_then_fallthrough(value):
    try:
        work()
    except ValueError:
        raise
    except Error:
        recover()
    finally:
        consume(value)


def fallthrough_then_raise(value):
    try:
        work()
    except ValueError:
        recover()
    except Error:
        raise
    finally:
        consume(value)


def bare_handler(value):
    try:
        work()
    except:
        recover()
    finally:
        consume(value)
";
        assert_matches_cpython_marshal(source, "fallthrough_handler_finally_borrows.py");
    }

    #[test]
    fn uses_cpython_finally_load_kinds_at_handler_boundaries() {
        let source = r#"import dis


def fallthrough_handler(value):
    try:
        work()
    except Error:
        recover()
    finally:
        consume(value)


def bare_handler(value):
    try:
        work()
    except:
        recover()
    finally:
        consume(value)


def loop_break_body(value, flag):
    try:
        while True:
            work()
            if flag:
                break
    except Error:
        recover()
    finally:
        consume(value)


def partial_raise_handler(value, flag):
    try:
        work()
    except Error:
        if flag:
            recover()
        else:
            raise
    finally:
        consume(value)


def value_loads(function):
    return " ".join(
        instruction.opname
        for instruction in dis.get_instructions(function)
        if instruction.argval == "value"
        and instruction.opname.startswith("LOAD_FAST")
    )


for function in (
    fallthrough_handler,
    bare_handler,
    loop_break_body,
    partial_raise_handler,
):
    print(value_loads(function))
"#;
        assert_executes(
            source,
            "LOAD_FAST_BORROW LOAD_FAST\nLOAD_FAST LOAD_FAST\nLOAD_FAST LOAD_FAST\nLOAD_FAST LOAD_FAST\n",
        );
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_handler_condition_exits() {
        let source = r"def unnamed(action, reraises):
    try:
        action()
    except Exception:
        if reraises:
            raise

def named(action, reraises):
    try:
        action()
    except Exception as error:
        if reraises(error):
            raise

def final_elif(action, first, second):
    try:
        action()
    except Exception:
        if first:
            raise
        elif second:
            raise
";
        assert_matches_cpython_marshal(source, "terminal_handler_condition_exits.py");
    }

    #[test]
    fn matches_cpython_marshal_for_shared_terminal_handler_borrow_edges() {
        let source = "def branch_raise(flag):\n    if flag:\n        try:\n            action()\n        except Exception:\n            raise\n    else:\n        consume(flag)\n    after(flag)\n\ndef branch_return(flag):\n    if flag:\n        try:\n            action()\n        except Exception:\n            return\n    else:\n        consume(flag)\n    after(flag)\n\ndef nested_loop(values):\n    result = None\n    for item in values:\n        try:\n            result = load(item)\n        except KeyError:\n            raise ValueError(item)\n    return result\n";
        assert_matches_cpython_marshal(source, "shared_terminal_handler_edges.py");
    }

    #[test]
    fn matches_cpython_marshal_for_with_passes_and_folded_iterables() {
        let source = "def pass_before_loop(manager, values):\n    with manager as target:\n        pass\n        for value in values:\n            target.write(value)\n\ndef pass_in_loop_and_else(manager, values):\n    with manager as target:\n        for value in values:\n            pass\n            target.write(value)\n        else:\n            pass\n\ndef folded_iterable(manager):\n    with manager as target:\n        for value in (1,) if True else (2,):\n            target.write(value)\n\nwith (\n    manager\n) as target: pass\n";
        assert_matches_cpython_marshal(source, "with_passes.py");
    }

    #[test]
    fn matches_cpython_marshal_for_multiline_boolean_returns_through_with() {
        let source = r"def multiline_and(manager, value, expected):
    with manager:
        return (
            value is not None
            and value.kind == expected.kind
            and value.name == expected.name
            and value.flags == expected.flags
        )
";
        assert_matches_cpython_marshal(source, "with_multiline_boolean_return.py");
    }

    #[test]
    fn matches_cpython_marshal_for_assert_none_condition_owners_in_with_bodies() {
        let source = include_str!("fixtures/with_assert_none_condition_owners.py");
        assert_matches_cpython_marshal(source, "with_assert_none_condition_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_try_exits_after_if_joins() {
        let source = "def retained(flag):\n    try:\n        action()\n    except Exception:\n        failed()\n    else:\n        if flag:\n            good()\n        else:\n            other()\n    after()\n\ndef ordinary(flag):\n    try:\n        action()\n    except Exception:\n        failed()\n    else:\n        if flag:\n            good()\n    after()\n";
        assert_matches_cpython_marshal(source, "try_if_join.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_if_trailing_try_exits() {
        let source = r"def nested_try(flag, outer):
    if outer:
        if flag:
            try:
                action()
            except Error:
                raise
    else:
        other()
    return None
";
        assert_matches_cpython_marshal(source, "nested_if_trailing_try.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_try_else_after_complete_if() {
        let source = r"def terminal_else(flag):
    try:
        if flag:
            action()
        else:
            other()
    except Error:
        failed()
    else:
        return None
    after()
";
        assert_matches_cpython_marshal(source, "terminal_try_else_complete_if.py");
    }

    #[test]
    fn matches_cpython_marshal_for_try_body_exits_after_if_joins() {
        let source = include_str!("fixtures/try_body_exit_joins.py");
        assert_matches_cpython_marshal(source, "try_body_exit_joins.py");
    }

    #[test]
    fn matches_cpython_marshal_for_direct_try_body_exit_nops() {
        let cases = [
            (
                "terminal_compare.py",
                r"def terminal_compare(value, name):
    try:
        if value <= 0:
            raise ValueError(name)
    except TypeError:
        raise ValueError(name) from None
    return value
",
            ),
            (
                "terminal_unique_or.py",
                r"def terminal_unique_or(first, second, name):
    try:
        if not (first <= 0) or second <= 0:
            raise ValueError(name)
    except TypeError:
        raise ValueError(name) from None
    return first
",
            ),
            (
                "multiple_false_edges.py",
                r"def multiple_false_edges(first, second, name):
    try:
        if first <= 0 and second <= 0:
            raise ValueError(name)
    except TypeError:
        raise ValueError(name) from None
    return first
",
            ),
            (
                "break_not_retained.py",
                r"def break_not_retained(values):
    for value in values:
        try:
            if value:
                break
        except KeyError:
            break
    return None
",
            ),
            (
                "terminal_elif.py",
                r"def terminal_elif(first, second):
    try:
        if first <= 0:
            raise ValueError
        elif second <= 0:
            raise RuntimeError
    except TypeError:
        raise OSError from None
    return first
",
            ),
        ];

        for (filename, source) in cases {
            assert_matches_cpython_marshal(source, filename);
        }
    }

    #[test]
    fn matches_cpython_marshal_for_a_constant_true_comprehension_filter() {
        let source = "value: int\n[x for x in values if True]\n[x for x in values if not None]\n[result for x in values if not predicate]\n{target.attr: None for target.attr in values}\n[\n    result\n    for x in values\n    if True\n]\n[\n    result\n    for x in values\n    if x\n    if not None\n]\n[x for x in values if left and right]\n{x for x in values if lower < x < upper if accepted}\n(x for x in values if left and right if accepted)\n(x for x in values if left and right async for y in async_values if accepted)\n\ndef restored_then_subscript(values):\n    result = {}\n    result['key'] = [value for value in values]\n\ndef assignment_reuses_target():\n    tasks = [task for task in tasks]\n";
        assert_matches_cpython_marshal(source, "comprehension.py");
    }

    #[test]
    fn matches_cpython_marshal_for_folded_tuple_not_nops() {
        let source = "multiline = (\n    not \"a\",\n    not \"b\",\n    (not \"c\",),\n)\nsame_line = (not \"a\", not \"b\")\nother_folds = (1 + 2, -3, ~4)\n";
        assert_matches_cpython_marshal(source, "folded_tuple.py");
    }

    #[test]
    fn matches_cpython_marshal_for_folded_boolean_addition() {
        let source = "seed = 'seed'\nvalue = True + False\n";
        assert_matches_cpython_marshal(source, "folded_boolean_addition.py");
    }

    #[test]
    fn matches_cpython_marshal_for_folded_mixed_numeric_multiplication() {
        let source = r"int_float = 65 * 1e9
float_int = 1e9 * 65
negative_int_float = -7 * 0.25
float_negative_int = 0.25 * -7
bool_float = True * 1.5
float_bool = 1.5 * False
signed_zero_left = -0.0 * 3
signed_zero_right = 3 * -0.0
overflow = 1e308 * 10
underflow = 1e-308 * 1e-308
";
        assert_matches_cpython_marshal(source, "folded_mixed_multiplication.py");
    }

    #[test]
    fn matches_cpython_marshal_for_folded_mixed_numeric_power() {
        let source = r"int_float = 7 ** 0.5
float_int = 2.0 ** 53
int_negative = 10 ** -10
signed_base_negative = (-2) ** -3
float_base_integer = (-2.0) ** 3
underflow = 10 ** -100000
true_negative = True ** -1
zero_negative = 0 ** -1
float_overflow = 1e308 ** 2
";
        assert_matches_cpython_marshal(source, "folded_mixed_power.py");
    }

    #[test]
    fn matches_cpython_marshal_for_arbitrary_integer_constant_folding() {
        let source = r"positive_shift = 1 << 64
negative_shift = -1 << 64
maximum_shift = 1 << 127
maximum_negative_shift = -1 << 127
unfolded_shift = 1 << 128
nested_shift = (1 << 64) << 63
negative_nested_shift = (-1 << 64) << 63
zero_huge_shift = 0 << 999999999999999999999999999999999999999
positive_minus_small = 18446744073709551616 - 1
positive_plus_small = 18446744073709551616 + 1
unbounded_add = 999999999999999999999999999999999999999999999999999999999999 + 1
unbounded_subtract = 1000000000000000000000000000000000000000000000000000000000000 - 1
unbounded_cancel = 999999999999999999999999999999999999999999999999999999999999 + -999999999999999999999999999999999999999999999999999999999999
unbounded_nested_add = (1 << 127) + (1 << 127)
small_minus_positive = 1 - 18446744073709551616
negative_minus_small = -18446744073709551616 - 1
negative_plus_small = -18446744073709551616 + 1
below_i64 = -(1 << 63) - 1
above_i64 = -(1 << 63) + 1
signed_sum = -9223372036854775808 + -9223372036854775808
positive_product = (1 << 63) * 2
negative_product = (1 << 63) * -2
maximum_product = 18446744073709551615 * 9223372036854775807
unfolded_product = 18446744073709551616 * 9223372036854775808
zero_huge_product = 0 * 999999999999999999999999999999999999999
maximum_power = 2 ** 64
unfolded_power = 2 ** 65
unfolded_huge_power = 2 ** 100
zero_huge_power = 0 ** 999999999999999999999999999999999999999
complex_positive_power = [(1 + 2j) ** 3, (2 + 3j) ** 4]
complex_negative_power = [(1 + 2j) ** -2, (2 + 3j) ** -3]
real_divided_by_complex = [1 / (1 + 2j), 1 / (2 + 3j)]
complex_divided_by_real = [(-11 + 2j) / 125, (-46 - 9j) / 2197]
unfolded_complex_power = (1e308 + 1j) ** 2
unfolded_zero_negative_power = 0j ** -1
negative_right_shift = -3 >> 1
large_negative_right_shift = -18446744073709551617 >> 64
huge_positive_right_shift = 1 >> 999999999999999999999999999999999999999
huge_negative_right_shift = -1 >> 999999999999999999999999999999999999999
";
        assert_matches_cpython_marshal(source, "folded_arbitrary_integers.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_large_constant_list() {
        let source = format!(
            "large = [{}]\n",
            (0..31)
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        assert_matches_cpython_marshal(&source, "large_constant_list.py");
    }

    #[test]
    fn matches_cpython_marshal_for_large_list_iterables() {
        let constants = (0..31)
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let dynamic = (0..31).map(|_| "value").collect::<Vec<_>>().join(", ");
        let source = format!(
            "def constant_loop():\n    for item in [{constants}]:\n        consume(item)\n\n\ndef dynamic_loop(value):\n    for item in [{dynamic}]:\n        consume(item)\n\n\ndef constant_contains(item):\n    return item in [{constants}]\n\n\ndef dynamic_contains(item, value):\n    return item in [{dynamic}]\n"
        );
        assert_matches_cpython_marshal(&source, "large_list_iterables.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_terminal_finally_suite() {
        let source = "\"\"\"doc\"\"\"\ntry:\n    raise ValueError\nfinally:\n    raise RuntimeError\nafter = 1\ntry:\n    action()\nexcept Error as captured:\n    consume(captured)\n";
        assert_matches_cpython_marshal(source, "terminal_finally.py");
    }

    #[test]
    fn matches_cpython_marshal_for_annotation_thunk_edges() {
        let source = "from typing import Annotated, List, TypedDict\n\n()\nvalue: int\n\ndef marker():\n    pass\n\nclass Item(TypedDict):\n    nodes: List[TypedDict('Node', {'name': str})]\n\nclass Metadata:\n    value: Annotated[int, marker()]\n\ndef outer():\n    from pathlib import Path\n\n    class Visitor:\n        def visit(self, value: Path) -> None:\n            pass\n";
        assert_matches_cpython_marshal(source, "annotation.py");
    }

    #[test]
    fn matches_cpython_marshal_for_fresh_annotation_locals_and_explicit_interns() {
        let source = r"def consume(**kwargs):
    return kwargs

class Container:
    value: (consume(format=int), consume(méthode=str))
";
        assert_matches_cpython_marshal(source, "marshal_hints.py");
    }

    #[test]
    fn matches_cpython_marshal_for_annotations_in_compound_suites() {
        let source = r"with manager:
    in_with: WithAnnotation = 1

try:
    in_try: TryAnnotation = 2
except Error:
    in_except: ExceptAnnotation = 3
else:
    in_else: ElseAnnotation = 4
finally:
    in_finally: FinallyAnnotation = 5

match subject:
    case 1:
        in_match: MatchAnnotation = 6
";
        assert_matches_cpython_marshal(source, "compound_annotations.py");
    }

    #[test]
    fn matches_cpython_marshal_for_unreachable_deferred_annotations() {
        let source =
            "seed = 0\nraise terminal\nvalue: Annotation = build()\ndeclaration: Annotation\n";
        assert_matches_cpython_marshal(source, "unreachable_annotation.py");
    }

    #[test]
    fn matches_cpython_marshal_for_an_inline_constant_with_body() {
        let source = "import io\nwith (\n    io  # comment\n    .open('file.txt') as file\n): ...\nwith manager:\n    while condition:\n        body()\n";
        assert_matches_cpython_marshal(source, "with.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_tuple_containing_a_constant_slice() {
        let source = "constant = lambda value: value[:, 1]\ndynamic = lambda value: value[::-1]\nwith_defaults = lambda value=1, other=2: value\nstring_slice = 'héllo'[1:4]\nstepped_string_slice = 'abcdef'[1:5:2]\nboolean_slice = 'abc'[False:True:True]\nbytes_slice = b'prefix'[2:]\ntuple_slice = (1, 2, 3)[1:]\nnegative_slice = 'abc'[:-1]\nstring_index = 'héllo'[1]\nfstring_index = f'abc'[0]\nbytes_index = b'prefix'[-1]\ntuple_index = (1, 2, 3)[-2]\ndynamic_list_index = [1, 2, 3][0]\nmember = item in [1, 2]\ndouble_negation = not not item\n5.0 ** 5.0\nTrue ** True\nFalse ** False\n() << 0 ** 99999999999999999999999999\n1 - 2\ninverted = ~1\nnegative_zero = -0000\nmixed = 1 + 2.5\nassert (False, 'x')\nassert (False,)\nassert ()\nassert True\npacked = fn('User', **{'name': str})\nunicode_keyword = fn(café=1)\nhuge = -999999999999999999999999999999999999999999\nadjacent = ('' f'prefix {item}')\nleft if condition else right\n'%s' % ('value',)\nif (1, 2):\n    pass\n";
        assert_matches_cpython_marshal(source, "example.py");
    }

    #[test]
    fn matches_cpython_marshal_for_shared_traversal_containers() {
        let source = r#"def outer():
    def nested(value):
        return (
            value in {("alpha", 1), ("beta", 2)},
            value[257:256:2],
            ((1, 2), (1, 2)),
        )

    return nested
"#;
        assert_matches_cpython_marshal(source, "marshal_traversal.py");
    }

    #[test]
    fn matches_cpython_marshal_for_lambda_edge_cases() {
        let source = "from __future__ import barry_as_FLUFL, generator_stop, print_function\nsimple = lambda: (yield None)\nyield_value = lambda value: (yield value)\ndelegating = lambda: (yield from source())\ndef eliminated():\n    while False:\n        value = lambda: missing\ndef   trailing_semicolon():\n    value = 1 \\\n        ;\ndef trailing_semicolon_try():\n    try:\n        value = 1;\n    except ValueError:\n        pass;\ndef overridden_return():\n    try:\n        return 1\n    finally:\n        return 2\ndef overridden_handler_return():\n    try:\n        pass\n    except:\n        return 1\n    finally:\n        return 2\ndef overridden_try_and_handler_returns():\n    try:\n        call()\n        return 1\n    except:\n        return 2\n    finally:\n        return 3\ndef assigned_after_try_except_finally():\n    value = None\n    try:\n        call()\n        value = 1\n    except:\n        value = 2\n    finally:\n        value = 3\n    return value\ndef empty_try_else():\n    try:\n        pass\n    except:\n        return 1\n    else:\n        return 2\ndef dead_loop_return(value):\n    while value > 0:\n        break\n        return 1\ndef return_inside_with():\n    with manager:\n        return 1\ndef finally_return_in_loop(items):\n    for item in items:\n        try:\n            use(item)\n        finally:\n            return\nasync  def terminal_async_comprehension():\n    if test:\n        values = [value async for value in source]\n";
        assert_matches_cpython_marshal(source, "example.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_return_through_finally() {
        let source = "def return_through_finally(items):\n    for item in items:\n        try:\n            pass\n        except Exception:\n            return\n        finally:\n            consume(item)\n\ndef preserve_return_value():\n    try:\n        pass\n        return produce()\n    finally:\n        consume()\n\ndef preserve_try_pass():\n    try:\n        pass\n        return produce()\n    except Exception:\n        recover()\n";
        assert_matches_cpython_marshal(source, "example.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_return_unwind_owners() {
        let source = r"def loop_value(items):
    try:
        for item in items:
            try:
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                return item
            except Error:
                pass
    finally:
        cleanup()
    return None


def literal_values(manager):
    try:
        with manager:
            prepare()
        try:
            action()
            return True
        except Error:
            return False
    finally:
        cleanup()


def nested_handler(compiler):
    try:
        command = compiler.compiler
    except AttributeError:
        try:
            if not compiler.initialized:
                compiler.initialize()
        except (SetupError, ValueError):
            return False
        command = [compiler.cc]
    return command
";
        assert_matches_cpython_marshal(source, "nested_return_unwinds.py");
    }

    #[test]
    fn matches_cpython_marshal_for_loop_tail_boolean_expression_exits() {
        let source = r"def conjunction(items):
    for item in items:
        left(item) and right(item)


def disjunction(items):
    for item in items:
        first(item) or second(item) or third(item)


def non_tail(items):
    for item in items:
        left(item) and right(item)
        consume(item)


def final_call(items):
    for item in items:
        consume(item)


def assigned(items):
    for item in items:
        result = left(item) and right(item)
";
        assert_matches_cpython_marshal(source, "loop_tail_boolean_exits.py");
    }

    #[test]
    fn matches_cpython_marshal_for_direct_nested_handler_return_owners() {
        let source = r"def direct_none(action):
    try:
        outer()
    except TypeError:
        try:
            action()
            raise AssertionError()
        except TypeError:
            return


def direct_value(action):
    try:
        outer()
    except TypeError:
        try:
            action()
            raise AssertionError()
        except ValueError:
            return False
        except TypeError:
            return True


def top_level(action):
    try:
        action()
    except TypeError:
        return


def nested_fallthrough(action):
    try:
        outer()
    except TypeError:
        try:
            action()
        except TypeError:
            recover()
";
        assert_matches_cpython_marshal(source, "nested_handler_return_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_finally_end_control_flow() {
        let source = "def conditional_return(flag):\n    try:\n        pass\n    finally:\n        if flag:\n            return\n\ndef nested_return():\n    try:\n        try:\n            pass\n        finally:\n            return\n    finally:\n        pass\n\ndef nested_handler_return():\n    try:\n        pass\n    finally:\n        try:\n            return\n        except Exception:\n            pass\n\ndef break_from_finally():\n    while True:\n        try:\n            pass\n        finally:\n            break\n\ndef continue_from_finally():\n    while True:\n        try:\n            pass\n        finally:\n            continue\n";
        assert_matches_cpython_marshal(source, "finally_end.py");
    }

    #[test]
    fn matches_cpython_marshal_for_pass_branches_and_surrogates() {
        let source = "def pass_elif(value):\n    if value:\n        pass\n    elif check(value):\n        value = update(value)\n    return value\n\ndef surrogates():\n    explicit_pair = \"\\ud800\\udc00heythere\"\n    lone_surrogate = \"\\ud800\"\n    mixed = \"\u{fffd}\\ud800\"\n    scalar = \"\\U00010000heythere\"\n    return explicit_pair, lone_surrogate, mixed, scalar\n";
        assert_matches_cpython_marshal(source, "example.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_percent_formatted_fstring() {
        let source = "f\"hello %s %s\" % (1, 2)\n\"%s %s\" % (1, 2)\n";
        assert_matches_cpython_marshal(source, "example.py");
    }

    #[test]
    fn matches_cpython_marshal_for_synthesized_percent_format_target_locations() {
        let source = r#"def terminal_false_target(flag, value):
    if flag:
        return value
    return "tail=%s" % value


def else_target(flag, value):
    if flag:
        result = "left=%s" % value
    else:
        result = "right=%s" % value
    return result


def final_elif_target(first, second, value):
    if first:
        return first
    elif second:
        return second
    return "tail=%s" % value


def entry_block(value):
    return "entry=%s" % value


def true_fallthrough(flag, value):
    if flag:
        return "body=%s" % value
    return value


def formatted_first(flag, value):
    if flag:
        return flag
    return "%s=tail" % value


def filtered_generator(items):
    return "".join(" %s=%s" % (key, value) for key, value in items if value)
"#;
        assert_matches_cpython_marshal(source, "percent_format_target.py");
    }

    #[test]
    fn matches_cpython_marshal_for_synthesized_percent_format_fallthrough_locations() {
        let source = r#"def named_handler(callback, value):
    try:
        return callback()
    except Exception as error:
        return "error=%s" % (error,)


def bare_handler(callback, value):
    try:
        return callback()
    except Exception:
        return "error=%s" % (value,)


def try_else(obj, value):
    try:
        name = obj.name
    except AttributeError:
        return value
    else:
        return "name=%s" % (name,)


def entry_block(value):
    return "entry=%s" % (value,)


def preceded_in_handler(callback, value):
    try:
        return callback()
    except Exception:
        result = value
        return "error=%s" % (result,)


def shared_join(flag, left, right):
    if flag:
        value = left
    else:
        value = right
    return "value=%s" % (value,)
"#;
        assert_matches_cpython_marshal(source, "percent_format_fallthrough.py");
    }

    #[test]
    fn matches_cpython_marshal_for_folded_not_and_terminal_while() {
        let source = "folded = not (\n    True\n)\n\ndef loop():\n    while not check() and left < right:\n        pass\n";
        assert_matches_cpython_marshal(source, "example.py");
    }

    #[test]
    fn executes_arithmetic_names_and_calls() {
        assert_executes("answer = 6 * 7\nprint(answer)\n", "42\n");
    }

    #[test]
    fn executes_control_flow() {
        let source = r#"
x = 3
total = 0
while x:
    total += x
    x -= 1

if total == 6:
    print("ok")
else:
    print("bad")
"#;
        assert_executes(source, "ok\n");
    }

    #[test]
    fn executes_simple_functions() {
        let source = r"
def add(left, right):
    result = left + right
    return result

print(add(20, 22))
";
        assert_executes(source, "42\n");
    }

    #[test]
    fn executes_literals_and_collections() {
        let source = r#"
values = [1, 2, 3]
pair = ("answer", 42)
mapping = {"answer": 42}
print(values[1], pair[0], mapping["answer"])
"#;
        assert_executes(source, "2 answer 42\n");
    }

    #[test]
    fn executes_short_circuit_expressions() {
        let source = r"
zero = 0
one = 1
print(zero and missing, one or missing, 1 if zero else 2, not zero)
";
        assert_executes(source, "0 1 2 True\n");
    }

    #[test]
    fn executes_break_continue_and_while_else() {
        let source = r"
x = 4
total = 0
while x:
    x -= 1
    if x == 2:
        continue
    if x == 0:
        break
    total += x
else:
    total = 999
print(total)
";
        assert_executes(source, "4\n");
    }

    #[test]
    fn executes_function_globals_and_docstrings() {
        let source = r#"
base = 40
def answer(offset):
    "answer docs"
    return base + offset

print(answer(2), answer.__doc__)
"#;
        assert_executes(source, "42 answer docs\n");
    }

    #[test]
    fn executes_extended_arguments_and_large_integers() {
        let mut source = String::from("flag = False\nif flag:\n");
        for index in 0..300 {
            writeln!(&mut source, "    value_{index} = {index}").unwrap();
        }
        source.push_str("else:\n    value_299 = 9223372036854775808\nprint(value_299)\n");

        assert_executes(&source, "9223372036854775808\n");
    }

    #[test]
    fn executes_for_loops_imports_and_globals() {
        let source = r"
import math as mathematics
from operator import add as plus

total = 0
for value in [1, 2, 3]:
    if value == 2:
        continue
    total = plus(total, value)
else:
    total += int(mathematics.sqrt(4))

def bump():
    global total
    total += 1

bump()
print(total)
";
        assert_executes(source, "7\n");
    }

    #[test]
    fn executes_unpacking_stores_and_deletes() {
        let source = r"
from types import SimpleNamespace

box = SimpleNamespace(value=1)
box.value += 4
values = [10, 20, 30]
values[1] += 2
head, *middle, tail = values
box.extra = head + tail
del values[0]
del box.extra
print(box.value, values, head, middle, tail, hasattr(box, 'extra'))
";
        assert_executes(source, "5 [22, 30] 10 [22] 30 False\n");
    }

    #[test]
    fn executes_starred_calls_collections_slices_and_comparisons() {
        let source = r"
def combine(a, b, c):
    return a + b + c

args = (1, 2)
keywords = {'c': 3}
values = [0, *args, 3]
mapping = {'a': 1, **{'b': 2}}
result = combine(*args, **keywords)
print(result, values[1:3], mapping, 0 < result < 10, (captured := result))
";
        assert_executes(source, "6 [1, 2] {'a': 1, 'b': 2} True 6\n");
    }

    #[test]
    fn executes_full_function_signatures_and_decorators() {
        let source = r"
def identity(function):
    return function

@identity
def combine(a, b=2, *args, c=3, **kwargs):
    return a + b + sum(args) + c + kwargs['d']

print(combine(1, 4, 5, 6, c=7, d=8))
";
        assert_executes(source, "31\n");
    }

    #[test]
    fn executes_lambdas_with_full_signatures() {
        let source = r"
combine = lambda a, b=2, *args, c=3, **kwargs: a + b + sum(args) + c + kwargs['d']
print(combine(1, 4, 5, 6, c=7, d=8))
";
        assert_executes(source, "31\n");
    }

    #[test]
    fn executes_nested_closures() {
        let source = r"
def outer(x):
    y = 2
    def middle(z):
        def inner():
            return x + y + z
        return inner
    return middle

print(outer(1)(3)())
";
        assert_executes(source, "6\n");
    }

    #[test]
    fn executes_nonlocal_assignments() {
        let source = r"
def counter():
    value = 0
    def bump():
        nonlocal value
        value += 1
        return value
    return bump

bump = counter()
print(bump(), bump())
";
        assert_executes(source, "1 2\n");
    }

    #[test]
    fn executes_fstrings_and_arbitrary_precision_integers() {
        let source = r#"
value = 1234567890123456789012345678901234567890
width = 6
print(f"value={value!r} suffix={value:0{width}d}")
print(f"{width=}")
"#;
        assert_executes(
            source,
            "value=1234567890123456789012345678901234567890 suffix=1234567890123456789012345678901234567890\nwidth=6\n",
        );
    }

    #[test]
    fn executes_classes_bases_decorators_and_methods() {
        let source = r"
def identity(value):
    return value

class Base:
    base = 40

@identity
class Answer(Base):
    offset = 2
    def value(self):
        return self.base + self.offset

print(Answer().value(), Answer.__firstlineno__, Answer.__static_attributes__)
";
        assert_executes(source, "42 8 ()\n");
    }

    #[test]
    fn executes_deferred_function_annotations() {
        let source = r"
def concrete(value: int) -> str:
    return str(value)

print(concrete.__annotations__)
";
        assert_executes(
            source,
            "{'value': <class 'int'>, 'return': <class 'str'>}\n",
        );

        let source = r"
from __future__ import annotations

def deferred(value: list[int]) -> str:
    return str(value)

print(deferred.__annotations__)
";
        assert_executes(source, "{'value': 'list[int]', 'return': 'str'}\n");
    }

    #[test]
    fn executes_future_module_annotations() {
        let source = r"
from __future__ import annotations

answer: int = 42
values: list[str]
print(__annotations__)
";
        assert_executes(source, "{'answer': 'int', 'values': 'list[str]'}\n");
    }

    #[test]
    fn executes_deferred_module_annotations() {
        let source = r"
answer: int = 42
values: list[str]
print(__annotate__(1))
";
        assert_executes(source, "{'answer': <class 'int'>, 'values': list[str]}\n");
    }

    #[test]
    fn executes_try_except_else_and_named_handlers() {
        let source = r#"
def parse(flag):
    try:
        if flag:
            raise ValueError("bad")
        value = 40
    except ValueError as error:
        value = len(str(error))
    else:
        value += 2
    return value

print(parse(False), parse(True))
"#;
        assert_executes(source, "42 3\n");
    }

    #[test]
    fn executes_try_finally_on_normal_and_exceptional_paths() {
        let source = include_str!("fixtures/execution/try_finally.py");
        assert_executes(
            source,
            "['normal', 'cleanup-1', 'handled', 'cleanup-2']\n2 ['normal', 'cleanup-1', 'handled', 'cleanup-2', 'value', 'override']\nbroken [('try', 1), ('break', 1)]\n[('try', 1), ('continue', 1), ('try', 2), ('continue', 2)]\n",
        );
    }

    #[test]
    fn executes_context_managers_and_exception_suppression() {
        let source = include_str!("fixtures/execution/context_managers.py");
        assert_executes(
            source,
            "['enter', 42, 'exit', 'enter', 'exit', 'enter', 'enter', 'exit', 'exit', 'enter', 0, 'exit', 'enter', 1, 'exit', 'handled-0', 'handled-1']\n",
        );
    }

    #[test]
    fn executes_inlined_comprehensions_without_leaking_targets() {
        let source = r"
x = 100
values = [x * 2 for x in [-1, 1, 2] if x > 0]
pairs = {(x, y) for x in [1, 2] for y in [3, 4] if y > 3}
mapping = {x: x * 2 for x in [1, 2]}
print(values, sorted(pairs), mapping, x)
";
        assert_executes(source, "[2, 4] [(1, 4), (2, 4)] {1: 2, 2: 4} 100\n");
    }

    #[test]
    fn executes_generators_and_generator_expressions() {
        let source = r"
def numbers():
    yield 1
    yield 2

def all_numbers():
    yield from numbers()

def capture():
    list((last := value) for value in numbers())
    return last

yielding_lambda = lambda: (yield 3)
delegating_lambda = lambda: (yield from numbers())

generated = (value * 2 for value in numbers() if value > 1)
print(list(all_numbers()), list(generated), capture(), list(yielding_lambda()), list(delegating_lambda()))
";
        assert_executes(source, "[1, 2] [4] 2 [3] [1, 2]\n");
    }

    #[test]
    fn executes_coroutines_and_async_generators() {
        let source = include_str!("fixtures/execution/coroutines.py");
        assert_executes(
            source,
            "(42, 1, 2) [1, 2, 'done'] [1] ['enter', 42, 'exit', 'enter', 'ValueError'] ([4], {1, 2}, {1: 2, 2: 4}, [1, 2]) [42, 42]\n43 ['enter', 'exit'] [42, 42] ['enter', 'exit', 'enter', 'exit']\n",
        );
    }

    #[test]
    fn executes_structural_pattern_matching() {
        let source = include_str!("fixtures/execution/pattern_matching.py");
        assert_executes(
            source,
            "none\n('number', 2)\n('mapping', 1, {'y': 2})\n('sequence', 1, [2, 3], 4)\n('point', 5)\nother\n",
        );
    }

    #[test]
    fn executes_template_strings() {
        let source = r"
name = 'world'
width = 8
template = t'hello {name!r:>{width}}!'
interpolation = template.interpolations[0]
print(template.strings)
print(interpolation.value, interpolation.expression, interpolation.conversion, interpolation.format_spec)
";
        assert_executes(source, "('hello ', '!')\nworld name r >8\n");
    }

    #[test]
    fn executes_starred_annotations_and_class_arguments() {
        let source = r"
bases = (object,)
options = {}

class Dynamic(*bases, **options):
    marker = 42

def annotated(*args: *tuple[int]):
    pass

print(Dynamic.marker, annotated.__annotate__(1))
";
        assert_executes(source, "42 {'args': *tuple[int]}\n");
    }

    #[test]
    fn executes_type_aliases_and_generic_functions() {
        let source = r"
type Plain = list[int]
type Pair[T] = tuple[T, T]

def identity[T](value: T) -> T:
    return value

def bounded[T: int = bool](value: T) -> T:
    return value

class Box[T]:
    def __init__(self, value):
        self.value = value

print(Plain.__value__, Plain.__type_params__)
print(Pair.__type_params__, Pair[int], Pair[int].__value__)
print(identity(42), identity.__type_params__, identity.__annotate__(1))
print(bounded.__type_params__[0].__bound__, bounded.__type_params__[0].__default__)
print(Box.__type_params__, Box[int], Box[int](42).value)
";
        assert_executes(
            source,
            "list[int] ()\n(T,) Pair[int] tuple[T, T]\n42 (T,) {'value': T, 'return': T}\n<class 'int'> <class 'bool'>\n(T,) __main__.Box[int] 42\n",
        );
    }
}
