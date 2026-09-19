use crate::common;

#[test]
fn async_try_else_break_borrow() {
    common::assert_matches_cpython_marshal(
        r"async def run(client, delays):
    for delay in delays:
        try:
            response = await client.send()
        except Error as error:
            report(error)
            if delay is None:
                break
        else:
            if response.done:
                return
            if response.retry:
                response.check()
            if delay is None:
                break
        assert delay is not None
        await pause(delay)
",
        "async_try_else_break_borrow.py",
    );
}

#[test]
fn sync_try_else_break_borrow() {
    common::assert_matches_cpython_marshal(
        r"def run(client, delays):
    for delay in delays:
        try:
            response = client.send()
        except Error as error:
            report(error)
            if delay is None:
                break
        else:
            if response.done:
                return
            if response.retry:
                response.check()
            if delay is None:
                break
        assert delay is not None
        pause(delay)
",
        "sync_try_else_break_borrow.py",
    );
}

#[test]
fn async_try_else_continue_borrow() {
    common::assert_matches_cpython_marshal(
        r"async def run(client, delays):
    for delay in delays:
        try:
            response = await client.send()
        except Error as error:
            report(error)
            if delay is None:
                continue
        else:
            if response.done:
                return
            if response.retry:
                response.check()
            if delay is None:
                continue
        assert delay is not None
        await pause(delay)
",
        "async_try_else_continue_borrow.py",
    );
}

#[test]
fn async_try_else_return_control() {
    common::assert_matches_cpython_marshal(
        r"async def run(client, delays):
    for delay in delays:
        try:
            response = await client.send()
        except Error as error:
            report(error)
            if delay is None:
                return
        else:
            if response.done:
                return
            if response.retry:
                response.check()
            if delay is None:
                return
        assert delay is not None
        await pause(delay)
",
        "async_try_else_return_control.py",
    );
}

#[test]
fn folded_break_condition_keeps_original_loop_exit() {
    for (condition, result) in [
        ("flag is None or True", "left, right, edges"),
        ("flag is None or other", "left, right, edges"),
        ("flag is None or False", "left, right, edges"),
        ("flag is None and True", "left, right, edges"),
        ("True", "left, right, edges"),
        ("flag is None or True", "left"),
    ] {
        let source = format!(
            "def run(flag, other, source):\n    left = []\n    right = []\n    edges = []\n    while True:\n        flag = choose()\n        if {condition}:\n            for item in source:\n                left.append(item)\n            for item in source:\n                right.append(item)\n            break\n        else:\n            for item in source:\n                left.append(item)\n            for item in source:\n                right.append(item)\n            edges.append(flag)\n    return {result}\n"
        );
        common::assert_matches_cpython_marshal(&source, "folded_break_condition.py");
    }
}

#[test]
fn pass_only_try_else_break_borrow() {
    common::assert_matches_cpython_marshal(
        r"def run(client, delays, stop):
    for delay in delays:
        try:
            client.send()
        except Error:
            pass
        else:
            if stop:
                break
        consume(delay)
        consume(client)
",
        "pass_only_try_else_break_borrow.py",
    );
}

#[test]
fn pass_only_try_else_continue_borrow() {
    common::assert_matches_cpython_marshal(
        r"def run(client, delays, stop):
    for delay in delays:
        try:
            client.send()
        except Error:
            pass
        else:
            if stop:
                continue
        consume(delay)
        consume(client)
",
        "pass_only_try_else_continue_borrow.py",
    );
}
