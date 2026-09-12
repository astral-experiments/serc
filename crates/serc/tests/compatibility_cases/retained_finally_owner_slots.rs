use crate::common;

#[test]
fn matches_cpython_retained_break_pop_finally_marker() {
    common::assert_matches_cpython_marshal(
        r#"async def f(task, clock, ready, sleep, suppress, Error):
    try:
        deadline = clock() + 5.0
        while clock() < deadline:
            if ready():
                break
            await sleep(0.01)
        else:
            raise AssertionError("timeout")
    finally:
        if not task.done():
            task.cancel()
            with suppress(Error):
                await task
    assert ready()
"#,
        "retained_break_finally.py",
    );
}

#[test]
fn matches_cpython_retained_break_short_finally_marker() {
    common::assert_matches_cpython_marshal(
        r#"async def f(task, clock, ready, sleep, suppress, Error):
    try:
        deadline = clock() + 5.0
        while clock() < deadline:
            if ready():
                break
            await sleep(0.01)
        else:
            raise AssertionError("timeout")
    finally:
        if task:
            task.cancel()
            with suppress(Error):
                await task
    assert ready()
"#,
        "retained_break_short_finally.py",
    );
}

#[test]
fn matches_cpython_shared_break_finally_control() {
    common::assert_matches_cpython_marshal(
        r#"async def f(task, clock, ready, sleep, suppress, Error, other_ready):
    try:
        deadline = clock() + 5.0
        while clock() < deadline:
            if ready():
                break
            if other_ready():
                break
            await sleep(0.01)
        else:
            raise AssertionError("timeout")
    finally:
        if not task.done():
            task.cancel()
            with suppress(Error):
                await task
    assert ready()
"#,
        "shared_break_finally.py",
    );
}

#[test]
fn matches_cpython_compacted_global_finally_control() {
    common::assert_matches_cpython_marshal(
        r#"async def f(task, clock, ready, sleep, suppress, Error):
    try:
        deadline = clock() + 5.0
        while clock() < deadline:
            if ready():
                break
            await sleep(0.01)
        else:
            raise AssertionError("timeout")
    finally:
        if not done(task):
            task.cancel()
            with suppress(Error):
                await task
    assert ready()
"#,
        "compacted_global_finally.py",
    );
}

#[test]
fn matches_cpython_retained_pop_fused_prefix_owner() {
    for count in [1, 4, 9] {
        let prefix = "        sink(left, right)\n".repeat(count);
        let source = format!(
            "def f(sink, left, right, options):\n    try:\n{prefix}        return 0\n    finally:\n        if not options.keep:\n            sink(left, right)\n"
        );
        common::assert_matches_cpython_marshal(&source, "retained_pop_fused_prefix.py");
    }
}

#[test]
fn matches_cpython_retained_pop_join_and_unfused_controls() {
    for prefix in [
        "        sink()\n",
        "        if left:\n            sink(left, right)\n",
        "        sink(left, right)\n        if left:\n            return 1\n",
    ] {
        let source = format!(
            "def f(sink, left, right, options):\n    try:\n{prefix}        return 0\n    finally:\n        if not options.keep:\n            sink(left, right)\n"
        );
        common::assert_matches_cpython_marshal(&source, "retained_pop_prefix_controls.py");
    }
}

#[test]
fn matches_cpython_finally_condition_fused_suffix_owner() {
    common::assert_matches_cpython_marshal(
        r"def f(lock, filesystem, runner, temporary, destination):
    with lock:
        if not filesystem.exists(destination):
            if filesystem.exists(temporary):
                filesystem.remove(temporary)
            try:
                runner.run([temporary, destination], check=True, capture_output=True)
                filesystem.replace(temporary, destination)
            finally:
                if filesystem.exists(temporary):
                    filesystem.remove(temporary)
    return destination
",
        "finally_condition_fused_suffix.py",
    );
}
