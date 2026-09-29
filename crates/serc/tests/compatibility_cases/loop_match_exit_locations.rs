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
fn terminal_match_iterator_breaks_keep_their_own_return_locations() {
    for (function, prefix, iterator) in [
        ("def", "", "for"),
        ("def", "    yield subject\n", "for"),
        ("async def", "", "async for"),
    ] {
        for pattern in ["Entry()", "_"] {
            for otherwise in ["", "            else:\n                exhausted()\n"] {
                let source = format!(
                    "{function} f(subject, items, result):\n{prefix}    match subject:\n        case 1:\n            prepare()\n        case {pattern}:\n            {iterator} item in items:\n                if stop(item):\n                    break\n                visit(item)\n{otherwise}"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "terminal_match_iterator_break_locations.py",
                );
            }
        }
    }
}

#[test]
fn unresolved_match_pop_does_not_erase_break_line() {
    for pattern in ["None", "0", "Failed()", "None if active"] {
        for loop_header in ["while True:", "while active:", "for item in items:"] {
            for body in ["break", "consume(value); break"] {
                let source = format!(
                    "def f(value, active, items):\n    {loop_header}\n        match next_value(value):\n            case {pattern}:\n                {body}\n            case State() as new_value:\n                value = new_value\n            case _ as invalid:\n                fail(invalid)\n    return value\n"
                );
                common::assert_matches_cpython_marshal(&source, "match_break_next_location.py");
            }
        }
    }
}
