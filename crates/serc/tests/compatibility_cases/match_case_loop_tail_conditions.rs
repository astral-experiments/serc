use crate::common;

#[test]
fn matches_cpython_nested_match_case_loop_tail_conditions() {
    for kind in [
        "ordinary",
        "generator",
        "async_generator",
        "while_generator",
    ] {
        for condition in ["enabled", "enabled and not ready()", "enabled or ready()"] {
            for has_default in [false, true] {
                let function_prefix = if kind == "async_generator" {
                    "async "
                } else {
                    ""
                };
                let loop_header = match kind {
                    "async_generator" => "async for item in items:",
                    "while_generator" => "while enabled:",
                    _ => "for item in items:",
                };
                let leading = if kind == "ordinary" {
                    "use(item)"
                } else {
                    "yield item"
                };
                let default = if has_default {
                    "            case _:\n                use(item)\n"
                } else {
                    ""
                };
                let source = format!(
                    "{function_prefix}def f(items, item, enabled, ready, check):\n    {loop_header}\n        match item:\n            case First():\n                if check():\n                    {leading}\n                    if {condition}:\n                        action()\n{default}"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "match_case_loop_tail_conditions.py",
                );
            }
        }
    }
}
