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
