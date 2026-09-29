use crate::common;

#[test]
fn matches_cpython_nested_match_failure_branch_locations() {
    for function in ["def", "async def"] {
        for prefix in ["", "    yield value\n"] {
            for body in ["value = 3", "return"] {
                for pattern in ["1", "Kind.VALUE", "None"] {
                    let source = format!(
                        "{function} process(subject, value, outer):\n{prefix}    if outer:\n        match subject:\n            case 0:\n                {body}\n            case {pattern}:\n                {body}\n    else:\n        fallback(value)\n    after(value)\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "nested_match_exit_locations.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_nested_match_failure_before_loop_continue() {
    for outer_pattern in ["1", "1 | 2"] {
        for body in ["return", "raise Error"] {
            let source = format!(
                "def process(subject, value):\n    while True:\n        match subject:\n            case 0:\n                return\n            case {outer_pattern}:\n                match value:\n                    case 0:\n                        return\n                    case 1:\n                        {body}\n        continue\n"
            );
            common::assert_matches_cpython_marshal(&source, "nested_match_loop_continue.py");
        }
    }
}

#[test]
fn matches_cpython_nested_match_failure_before_scope_exit() {
    for function in ["def", "async def"] {
        for prefix in ["", "    yield value\n"] {
            for pattern in ["1", "Kind.VALUE", "None"] {
                for default in ["", "        case _:\n            raise Error\n"] {
                    let source = format!(
                        "{function} process(subject, value, outer):\n{prefix}    match outer:\n        case 0:\n            match subject:\n                case 0:\n                    return\n                case {pattern}:\n                    return\n{default}"
                    );
                    common::assert_matches_cpython_marshal(&source, "nested_match_scope_exit.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_final_nested_match_failure_before_return() {
    for function in ["def", "async def"] {
        for body in [
            "value = inner",
            "if value:\n                        return inner",
        ] {
            for return_value in ["value", "(value, outer)"] {
                let source = format!(
                    "{function} f(outer, inner, value):\n    match outer:\n        case 0:\n            return 0\n        case 1:\n            match inner:\n                case 0:\n                    return 1\n                case 1:\n                    {body}\n    return {return_value}\n"
                );
                common::assert_matches_cpython_marshal(&source, "final_nested_match_failure.py");
            }
        }
    }
}
