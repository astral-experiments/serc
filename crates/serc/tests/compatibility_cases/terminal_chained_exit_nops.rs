use crate::common;

#[test]
fn retains_terminal_chained_return_jump_locations() {
    for prefix in ["", "async "] {
        for condition in [
            "not low < value < high",
            "not (ready and low < value < high)",
            "ready or low < value < high",
        ] {
            for returned in [
                "return",
                "return value",
                "return value + 1",
                "return 1 + 2",
                "return value.attr",
                "return value.method()",
                "return (value, ready)",
                "return work(value)",
            ] {
                let source = format!(
                    "{prefix}def f(low, value, high, ready):\n    if {condition}:\n        {returned}\n"
                );
                common::assert_matches_cpython_marshal(&source, "terminal_chained_return_nops.py");
            }
        }
    }
}

#[test]
fn retains_terminal_generator_and_elif_exit_locations() {
    let source = r"def generator(low, value, high, ready):
    yield value
    if not (ready and low < value < high):
        return value

def final_elif(low, value, high, ready):
    if ready:
        work()
    elif not low < value < high:
        raise Error

def same_line_control(low, value, high):
    if not low < value < high: return value

def preceding_call_control(low, value, high):
    if not low < value < high:
        work()
        return value

def explicit_following_return_control(low, value, high):
    if not low < value < high:
        return value
    return low
";
    common::assert_matches_cpython_marshal(source, "terminal_chained_generator_elif_nops.py");
}

#[test]
fn retains_chained_boolean_loop_return_locations() {
    for loop_header in ["for value in values:", "while ready(values):"] {
        for condition in [
            "ready(value) and 0 <= value <= bound",
            "ready(value) and not flag(value) and 0 <= value <= bound",
            "ready(value) or 0 <= value <= bound",
            "ready(value) and 0 < value in (1, 2)",
        ] {
            for returned in ["return", "return value"] {
                let source = format!(
                    "def f(values, ready, flag, bound):\n    {loop_header}\n        if {condition}:\n            {returned}\n    return None\n"
                );
                common::assert_matches_cpython_marshal(&source, "chained_boolean_loop_return.py");
            }
        }
    }
}
