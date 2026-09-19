use crate::common;

#[test]
fn matches_cpython_protected_while_normal_exit_joins() {
    for body in [
        "with cm:\n    while pending():\n        work()\n    if enabled:\n        raise RuntimeError\n",
        "with cm:\n    if enabled:\n        while pending():\n            work()\n",
        "with cm:\n    if enabled:\n        while pending():\n            work()\n    state[0] = value\n",
        "with cm:\n    while pending():\n        work()\n",
        "with cm:\n    while pending():\n        work()\n    state[0] = value\n",
        "if enabled:\n    while pending():\n        work()\nstate[0] = value\n",
        "with cm:\n    try:\n        work()\n    except RuntimeError:\n        return value\n    else:\n        while pending():\n            work()\n",
        "with cm:\n    try:\n        work()\n    finally:\n        while pending():\n            work()\n",
    ] {
        let mut source = String::from("def f(cm, enabled, pending, work, state, value):\n");
        for line in body.lines() {
            source.push_str("    ");
            source.push_str(line);
            source.push('\n');
        }
        common::assert_matches_cpython_marshal(&source, "protected_while_normal_exit_joins.py");
    }
}

#[test]
fn matches_cpython_while_condition_exit_locations() {
    for condition in [
        "a or b",
        "not a or not b",
        "a() or b()",
        "not a() or not b()",
        "not (a and b)",
        "(a or b) and (c or d)",
    ] {
        let source = format!(
            "def f(a, b, c, d):\n    while {condition}:\n        try:\n            consume()\n        except Error:\n            continue\n        consume()\n"
        );
        common::assert_matches_cpython_marshal(&source, "while_condition_exit_locations.py");
    }
}

#[test]
fn matches_cpython_while_exit_nop_locations_in_with() {
    for condition in [
        "pending or not exhausted",
        "pending or exhausted",
        "not pending or not exhausted",
        "pending and exhausted",
        "not (pending and exhausted)",
        "pending is not None",
        "pending() or not exhausted()",
    ] {
        let source = format!(
            "def f(manager, pending, exhausted):\n    with manager:\n        while {condition}:\n            consume()\n"
        );
        common::assert_matches_cpython_marshal(&source, "while_exit_nop_locations_in_with.py");
    }
}

#[test]
fn matches_cpython_while_before_separately_compiled_loop_tail() {
    for scope in ["def", "async def"] {
        for outer_loop in ["for item in items:", "while running:"] {
            for tail in [
                "if other:\n    after()\n",
                "while other:\n    after()\n",
                "assert other\n",
                "try:\n    after()\nexcept Error:\n    recover()\n",
            ] {
                let mut indented_tail = String::new();
                for line in tail.lines() {
                    indented_tail.push_str("            ");
                    indented_tail.push_str(line);
                    indented_tail.push('\n');
                }
                let source = format!(
                    "{scope} f(manager, items, running, flag, other):\n    with manager:\n        {outer_loop}\n            while flag:\n                work()\n{indented_tail}"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "while_before_separately_compiled_loop_tail.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_runtime_while_normal_try_exit_retention() {
    for (filename, source) in [
        (
            "manager_runtime_while_exit.py",
            r"def transfer(manager, stream, sink):
    with manager:
        try:
            while chunk := stream.read():
                sink.write(chunk)
        except OSError:
            return
        except Exception:
            try:
                sink.fail()
            except OSError:
                return
            return
        sink.flush()
        sink.finish()
",
        ),
        (
            "nested_try_runtime_while_exit.py",
            r"def collect(stream):
    values = []
    try:
        try:
            while stream.ready():
                values.append(stream.take())
        except Exception:
            values = []
    except Exception:
        values = []
    return values
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
