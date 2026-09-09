use crate::common;

#[test]
fn matches_cpython_marshal_for_unreachable_plain_call_kw_constants() {
    let source = r#"import c_analyzer.datafiles as _datafiles


def imported_module(variables, ignored, root):
    raise NotImplementedError
    _datafiles.write_ignored(variables, ignored, relroot=root)


def plain_name(call, value):
    raise NotImplementedError
    call(value=value)


def two_keywords(call, left, right):
    raise NotImplementedError
    call(left=left.value, right=right.value)


def positional_and_keyword(call, value, other):
    raise NotImplementedError
    call(value.attribute, other=other.attribute)


def generator(call, value):
    yield value
    raise NotImplementedError
    call(value=value)


async def coroutine(call, value):
    await value
    raise NotImplementedError
    call(value=value)


def debug_leaf_control(call):
    raise NotImplementedError
    call(value=__debug__)


def debug_attribute_leaf_control(call):
    raise NotImplementedError
    call(value=__debug__.attribute)


def debug_receiver(value):
    raise NotImplementedError
    __debug__.call(value=value)


def earlier_literal(call, value):
    raise RuntimeError("earlier")
    call(value=value)


def nested_call_control(call, inner, value):
    raise RuntimeError("earlier")
    call(value=inner(flag=value))


def nested_attribute_control(call, factory, value):
    raise RuntimeError("earlier")
    call(value=factory().attribute)


def starred_control(call, items, value):
    raise RuntimeError("earlier")
    call(*items, value=value)


def unpack_control(call, values, value):
    raise RuntimeError("earlier")
    call(value=value, **values)


def plain_threshold_control(call, value):
    raise RuntimeError("earlier")
    call(value, value, value, value, value, value, value, value,
         value, value, value, value, value, value, value, value,
         value, value, value, value, value, value, value, value,
         value, value, value, value, value=value)


def method_control(target, value):
    raise RuntimeError("earlier")
    target.call(value=value)


def trailing_statement_control(call, value):
    raise RuntimeError("earlier")
    call(value=value)
    call(value=value)
"#;

    common::assert_matches_cpython_marshal(source, "unreachable_plain_call_kw_constants.py");
}

#[test]
fn matches_debug_constants_in_discarded_suites() {
    for definition in ["def", "async def"] {
        for prefix in ["", "    'documentation'\n", "    consume('prefix')\n"] {
            for shape in [
                "raise",
                "return",
                "branch",
                "false_if",
                "false_while",
                "branch_literal",
                "true_if",
            ] {
                for expression in [
                    "consume(__debug__)",
                    "consume(__debug__.attribute)",
                    "__debug__.consume(value=value)",
                    "consume((__debug__, value))",
                    "consume(__debug__ and value)",
                    "consume(__debug__ == value)",
                    "consume(value, flag=__debug__, **options)",
                ] {
                    let body = match shape {
                        "raise" => format!("    raise Error\n    {expression}\n"),
                        "return" => format!("    return value\n    {expression}\n"),
                        "branch" => format!(
                            "    if flag:\n        return value\n        {expression}\n    return __debug__\n"
                        ),
                        "false_if" => format!(
                            "    if False:\n        {expression}\n    return (None, __debug__)\n"
                        ),
                        "false_while" => format!(
                            "    while False:\n        {expression}\n    return (None, __debug__)\n"
                        ),
                        "true_if" => format!(
                            "    if True:\n        return value\n    else:\n        {expression}\n    return (None, __debug__)\n"
                        ),
                        _ => format!(
                            "    if flag:\n        return value\n        {expression}\n    consume('later')\n    return __debug__\n"
                        ),
                    };
                    let source = format!("{definition} f(flag, value, options):\n{prefix}{body}");
                    common::assert_matches_cpython_marshal(&source, "discarded_debug_constants.py");
                }
            }
        }
    }
}

#[test]
fn matches_debug_names_in_skipped_expressions() {
    for definition in ["def", "async def"] {
        for prefix in ["", "    'documentation'\n", "    consume('prefix')\n"] {
            for expression in [
                "False and __debug__",
                "True or __debug__",
                "False and __debug__.attribute",
                "True or (__debug__, value)",
                "True or consume(__debug__)",
                "__debug__ if False else value",
            ] {
                for assertion in [false, true] {
                    let body = if assertion {
                        format!("    assert True, {expression}\n    return __debug__\n")
                    } else {
                        format!("    return {expression}\n")
                    };
                    let source = format!("{definition} f(value):\n{prefix}{body}");
                    common::assert_matches_cpython_marshal(&source, "skipped_debug_names.py");
                }
            }
        }
    }
}
