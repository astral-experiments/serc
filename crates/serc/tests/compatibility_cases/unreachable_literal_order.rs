use crate::common;

#[test]
fn matches_cpython_marshal_for_reused_unreachable_literals() {
    for value in ["'reused'", "300", "1.25"] {
        for dead_statement in [
            format!("return {value}"),
            format!("consume({value})"),
            value.to_string(),
            format!("def inner():\n            return {value}"),
            format!("value = (lambda default={value}: default)"),
            format!("value = ({value} for element in values)"),
            format!("value = (element for element in ({value},))"),
        ] {
            for terminal in ["raise Error()", "return marker"] {
                let source = format!(
                    "def f(flag, marker, values):\n    consume('prior')\n    if flag:\n        {terminal}\n        {dead_statement}\n    consume('between')\n    return {value}\n"
                );
                common::assert_matches_cpython_marshal(&source, "unreachable_literal_order.py");
            }
        }
    }
}

#[test]
fn matches_cpython_constants_first_visited_in_discarded_suites() {
    for prefix in ["", "    consume('before')\n"] {
        for header in [
            "    if False:\n",
            "    if True:\n        work()\n    else:\n",
            "    while False:\n",
            "    if flag:\n        return\n",
        ] {
            for (definition, discarded, surviving) in [
                ("def", "consume('reused')", "consume('reused')"),
                ("def", "consume(key=value)", "consume(key=value)"),
                (
                    "def",
                    "consume(*args, key=value)",
                    "consume(*args, key=value)",
                ),
                ("async def", "await work()", "consume(None)"),
                (
                    "async def",
                    "await work('reused')",
                    "consume('reused', None)",
                ),
                ("def", "with manager():\n    work()", "consume(None)"),
                ("def", "class Cached:\n    pass", "consume('Cached')"),
                (
                    "def",
                    "class Cached(factory('base')):\n    pass",
                    "consume('Cached', 'base')",
                ),
                ("def", "class Cached[T]:\n    pass", "consume('Cached')"),
            ] {
                let mut source = format!("{definition} f(flag, value, args):\n{prefix}{header}");
                for line in discarded.lines() {
                    source.push_str("        ");
                    source.push_str(line);
                    source.push('\n');
                }
                source.push_str("    consume('middle')\n    ");
                source.push_str(surviving);
                source.push('\n');
                common::assert_matches_cpython_marshal(
                    &source,
                    "discarded_suite_constant_order.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_names_in_unreachable_try_suites() {
    for handler in ["except", "except*"] {
        for terminal in ["raise NotImplementedError", "return"] {
            let source = format!(
                "def f(obj):\n    {terminal}\n    try:\n        obj.body_name('body-value')\n    {handler} HandlerError:\n        obj.handler_name('handler-value')\n    else:\n        obj.else_name('else-value')\n    finally:\n        obj.final_name('final-value')\n"
            );
            common::assert_matches_cpython_marshal(&source, "unreachable_try_names.py");
        }
    }
}

#[test]
fn preserves_unreachable_try_admissions_with_finally_unwinds() {
    for exit in [
        "return body_name('reused')",
        "body_name('reused')\n    break",
    ] {
        for outer in [false, true] {
            let inner = format!(
                "try:\n    {exit}\nexcept HandlerError:\n    handler_name('reused')\nelse:\n    else_name('reused')\n"
            );
            let mut suite = if outer {
                let mut suite = String::from("try:\n");
                for line in inner.lines() {
                    suite.push_str("    ");
                    suite.push_str(line);
                    suite.push('\n');
                }
                suite
            } else {
                inner
            };
            suite.push_str("finally:\n    HandlerError('reused')\n    handler_name('reused')\n");
            if exit.contains("break") {
                let mut loop_suite = String::from("while loop_test:\n");
                for line in suite.lines() {
                    loop_suite.push_str("    ");
                    loop_suite.push_str(line);
                    loop_suite.push('\n');
                }
                suite = loop_suite;
            }
            let mut source =
                String::from("def f(flag):\n    if flag:\n        raise NotImplementedError\n");
            for line in suite.lines() {
                source.push_str("        ");
                source.push_str(line);
                source.push('\n');
            }
            source.push_str("    return ('reused',)\n");
            common::assert_matches_cpython_marshal(&source, "unreachable_try_finally_unwind.py");
        }
    }
}
