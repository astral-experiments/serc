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
