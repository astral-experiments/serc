use crate::common;

#[test]
fn matches_cpython_nested_match_failure_exits_inside_loops() {
    for function in ["def", "async def"] {
        for prefix in ["", "    yield value\n"] {
            for loop_header in ["while active:", "for item in items:"] {
                for pattern in ["1", "None"] {
                    for body in ["result = 7", "process(value)"] {
                        for continuation in ["continue", "break", "after(result)"] {
                            let source = format!(
                                "{function} f(subject, value, choice, active, items, result):\n{prefix}    {loop_header}\n        if choice:\n            match subject:\n                case 0:\n                    {body}\n                case {pattern}:\n                    {body}\n        else:\n            fallback(value)\n        {continuation}\n    done(result)\n"
                            );
                            common::assert_matches_cpython_marshal(
                                &source,
                                "loop_match_exit_locations.py",
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn preserves_nested_value_match_failure_before_outer_case_exit() {
    for (filename, source) in [
        (
            "nested_match_loop_polarity.py",
            r#"def f(tokens, low=3, high=7):
    count = 0
    for token in tokens:
        match token:
            case int():
                count += 1
            case dict():
                match token.get("type"):
                    case "low":
                        count += low
                    case "high":
                        count += high
    return count
"#,
        ),
        (
            "direct_match_loop_polarity.py",
            r#"def f(tokens, low=3, high=7):
    count = 0
    for token in tokens:
        match token.get("type"):
            case "low":
                count += low
            case "high":
                count += high
    return count
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn threads_refutable_pass_success_to_loop_backedge() {
    for (filename, source) in [
        (
            "refutable_class_pass_before_default_loop.py",
            r"def collect(values):
    output = []
    for value in values:
        match value:
            case Included():
                output.append(value)
            case Skipped():
                pass
            case _:
                assert_never(value)
    return output
",
        ),
        (
            "refutable_class_pass_final_async_loop.py",
            r"async def collect(values, output):
    async for value in values:
        match value:
            case Included():
                await output.put(value)
            case Skipped():
                pass
",
        ),
        (
            "refutable_class_call_before_default_loop_control.py",
            r"def collect(values):
    output = []
    for value in values:
        match value:
            case Included():
                output.append(value)
            case Skipped():
                consume(value)
            case _:
                assert_never(value)
    return output
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
