use crate::common;

fn indent(source: &str) -> String {
    let mut indented = String::new();
    for line in source.lines() {
        indented.push_str("    ");
        indented.push_str(line);
        indented.push('\n');
    }
    indented
}

#[test]
fn matches_cpython_normal_finally_source_join_owners() {
    for (scope, suspension, close) in [
        ("def", "work()", "close(value)"),
        ("async def", "await work()", "await close(value)"),
        ("def", "yield value", "close(value)"),
        ("async def", "yield value", "await close(value)"),
    ] {
        for manager in ["with", "async with"] {
            if manager == "async with" && scope != "async def" {
                continue;
            }
            for condition in [
                "value is not None",
                "value is None",
                "value.attr",
                "value == enabled",
                "value in choices",
                "value and enabled",
            ] {
                let body = format!(
                    "try:\n    if enabled:\n        {suspension}\n    else:\n        other()\nfinally:\n    if {condition}:\n        {close}\n"
                );
                let body = format!("{manager} manager:\n{}", indent(&body));
                let source = format!(
                    "{scope} f(value, enabled, manager, choices, limit):\n{}",
                    indent(&body),
                );
                common::assert_matches_cpython_marshal(&source, "normal_finally_source_join.py");
            }
        }
    }
}

#[test]
fn matches_cpython_normal_finally_structural_marker_owners() {
    for suspension in ["await work()", "yield value"] {
        for shape in ["expression", "iterator", "linear"] {
            let body = match shape {
                "expression" => "item = value if enabled else limit".to_string(),
                "iterator" => format!("for item in choices:\n    {suspension}"),
                _ if suspension == "yield value" => "yield value\nwork()\nwork()".to_string(),
                _ => suspension.to_string(),
            };
            for condition in ["value is not None", "value is None"] {
                let body = format!(
                    "try:\n{}finally:\n    if {condition}:\n        await close(value)\n",
                    indent(&body),
                );
                let body = format!("async with manager:\n{}", indent(&body));
                let source = format!(
                    "async def f(value, enabled, manager, choices, limit):\n{}",
                    indent(&body),
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "normal_finally_structural_owner.py",
                );
            }
        }
    }
}

#[test]
fn preserves_normal_finally_original_prefix_owners() {
    for (scope, suspension, close) in [
        ("def", "work()", "close(value)"),
        ("async def", "await work()", "await close(value)"),
        ("def", "yield value", "close(value)"),
        ("async def", "yield value", "await close(value)"),
    ] {
        for prefix in ["work()\nwork()\n", "work()\nwork()\nwork()\n"] {
            for condition in ["value is not None", "value is None"] {
                let body = format!("if enabled:\n    {suspension}\nelse:\n    other()\n{prefix}");
                let body = format!(
                    "try:\n{}finally:\n    if {condition}:\n        {close}\n",
                    indent(&body),
                );
                let source = format!(
                    "{scope} f(value, enabled, manager, choices, limit):\n{}",
                    indent(&body),
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "normal_finally_original_prefix.py",
                );
            }
        }
    }
}
