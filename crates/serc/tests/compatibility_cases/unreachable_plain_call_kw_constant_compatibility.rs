use crate::common;

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
