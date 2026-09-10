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

#[test]
fn retains_multiline_boolean_assertion_exit_locations() {
    for join in ["and", "or"] {
        for comparison in [
            "0 < value < 10",
            "0 < value is not None",
            "0 < value < 10 < 20",
        ] {
            for message in ["", ", 'invalid'"] {
                for multiline in [false, true] {
                    let condition = if multiline {
                        format!("ready {join} (\n        {comparison}\n    )")
                    } else {
                        format!("ready {join} {comparison}")
                    };
                    let source = format!("def f(ready, value):\n    assert {condition}{message}\n");
                    common::assert_matches_cpython_marshal(
                        &source,
                        "boolean_assertion_exit_nops.py",
                    );
                }
            }
        }
    }
}

#[test]
fn retains_chained_try_exit_locations() {
    for returned in ["value", "convert(value)"] {
        for continuation in [
            "return default",
            "default = convert(default)\n    return normalize(default)",
        ] {
            let source = format!(
                "def f(value, default):\n    try:\n        if 0 < value < 10:\n            return {returned}\n    except ValueError:\n        pass\n    {continuation}\n"
            );
            common::assert_matches_cpython_marshal(&source, "chained_try_exit_nops.py");
        }
    }
}

#[test]
fn retains_chained_while_and_try_else_exit_locations() {
    for body in [
        "while 0 < value < 10:\n            value += 1",
        "while 0 < value < 10:\n            for item in items:\n                if item:\n                    break\n            value += 1",
    ] {
        let source = format!(
            "def f(value, default, items):\n    try:\n        {body}\n    except ValueError:\n        pass\n    return default\n"
        );
        common::assert_matches_cpython_marshal(&source, "chained_while_try_exit_nops.py");
    }
    for comparison in ["0 < value < 10", "0 < value is not None"] {
        for returned in ["default", "convert(default)"] {
            let source = format!(
                "def f(value, default):\n    try:\n        value = convert(value)\n    except ValueError:\n        pass\n    else:\n        assert {comparison}\n    return {returned}\n"
            );
            common::assert_matches_cpython_marshal(&source, "chained_try_else_exit_nops.py");
        }
    }
}

#[test]
fn preserves_negated_chained_try_exit_provenance() {
    for condition in ["0 < value < 10", "not 0 < value < 10"] {
        let source = format!(
            "def recovered(value):\n    try:\n        if {condition}:\n            raise ValueError\n    except ValueError:\n        report(value)\n    return normalize(convert(value))\n\ndef finalized(value):\n    try:\n        if {condition}:\n            raise ValueError\n    except ValueError:\n        raise Error from None\n    finally:\n        value.close()\n"
        );
        common::assert_matches_cpython_marshal(&source, "negated_chained_try_exit_provenance.py");
    }
}
