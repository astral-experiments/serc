use crate::common;

#[test]
fn terminal_generator_breaks_leave_the_enclosing_try_after_handler_cleanup() {
    for (handler, binding) in [("unnamed", ""), ("named", " as error")] {
        for (tail, continuation) in [("terminal", ""), ("nonterminal", "        print(items)\n")] {
            let source = format!(
                "def f(items):
    try:
        while True:
            try:
                item = next(items)
            except StopIteration{binding}:
                yield None
                break
            yield item
{continuation}    except StopIteration:
        pass
"
            );
            common::assert_matches_cpython_marshal(
                &source,
                &format!("generator_break_{handler}_{tail}.py"),
            );
        }
    }
}

#[test]
fn matches_resumed_yield_return_marker_slots() {
    for (value, handler) in [
        ("read(read(value))", ""),
        ("read(value)", "    except Error:\n        raise\n"),
        ("read(value)", ""),
    ] {
        let source = format!(
            "def entries(value):\n    try:\n        while True:\n            yield {value}\n            return\n{handler}    finally:\n        if value is not None:\n            value.close()\n"
        );
        common::assert_matches_cpython_marshal(&source, "resumed_yield_return.py");
    }
}

#[test]
fn matches_the_innermost_copied_finally_after_yield() {
    common::assert_matches_cpython_marshal(
        "def entries(value):\n    try:\n        try:\n            while True:\n                yield read(value)\n                return\n        except Error:\n            raise\n        finally:\n            if value is not None:\n                value.close()\n    finally:\n        done()\n",
        "nested_yield_return.py",
    );
}
