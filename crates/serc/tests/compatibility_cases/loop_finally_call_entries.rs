use crate::common;

#[test]
fn matches_cpython_finally_calls_reached_by_retained_breaks() {
    for (prefix, cleanup) in [("", "x.close()"), ("async ", "await x.close()")] {
        for (name, body) in [
            (
                "else_arm",
                "try:\n    while True:\n        if pred:\n            x.work()\n        else:\n            if x.ready():\n                break\n            x.wait()\nfinally:\n    x.close()",
            ),
            (
                "after_manager",
                "try:\n    while True:\n        with cm:\n            x.work()\n        if pred:\n            break\n        x.wait()\nfinally:\n    x.close()",
            ),
            (
                "outer_manager",
                "with cm:\n    try:\n        while True:\n            if pred:\n                break\n            x.work()\n    finally:\n        x.close()",
            ),
        ] {
            let source = format!(
                "{prefix}def f(x, pred, cm):\n    {}\n",
                body.replace("x.close()", cleanup).replace('\n', "\n    ")
            );
            common::assert_matches_cpython_marshal(
                &source,
                &format!("retained_break_finally_{name}.py"),
            );
        }
    }
}

#[test]
fn preserves_cpython_removed_break_and_return_call_entries() {
    for (prefix, cleanup) in [("", "x.close()"), ("async ", "await x.close()")] {
        for (name, body) in [
            (
                "tail_break",
                "try:\n    while True:\n        x.work()\n        if pred:\n            break\nfinally:\n    x.close()",
            ),
            (
                "tail_break_with_manager",
                "with cm:\n    try:\n        while True:\n            x.work()\n            if pred:\n                break\n    finally:\n        x.close()",
            ),
            (
                "ordinary_return_call",
                "while True:\n    if pred:\n        break\n    x.work()\nreturn cleanup(x)",
            ),
        ] {
            let source = format!(
                "{prefix}def f(x, pred, cm):\n    {}\n",
                body.replace("x.close()", cleanup).replace('\n', "\n    ")
            );
            common::assert_matches_cpython_marshal(
                &source,
                &format!("removed_break_finally_{name}.py"),
            );
        }
    }
}

#[test]
fn matches_cpython_borrows_after_copied_async_manager_exits() {
    for (mode, work) in [("coroutine", "x.work()"), ("async_generator", "yield x")] {
        for (handler_name, handler) in [("ordinary", "except"), ("group", "except*")] {
            let source = format!(
                r"async def f(x, pred, cm):
    timed_out = False
    try:
        async with cm:
            try:
                while True:
                    if pred:
                        timed_out = True
                        break
                    {work}
            finally:
                x.close()
    {handler} E:
        pass
    if timed_out:
        x.done()
"
            );
            common::assert_matches_cpython_marshal(
                &source,
                &format!("copied_manager_exit_{mode}_{handler_name}.py"),
            );
        }
    }
}

#[test]
fn matches_cpython_finally_borrows_after_prefixed_breaks() {
    for (name, condition, prefix) in [
        ("call", "True", "record()"),
        ("assignment", "True", "value = 1"),
        ("runtime_condition", "running()", "record()"),
        ("standalone", "True", ""),
    ] {
        let source = format!(
            r"async def f(manager, env):
    async with manager:
        try:
            while {condition}:
                if stop():
                    {prefix}
                    break
                await work()
        except Error:
            report()
        finally:
            await env.close()
"
        );
        common::assert_matches_cpython_marshal(
            &source,
            &format!("prefixed_break_finally_{name}.py"),
        );
    }
}

#[test]
fn matches_cpython_nested_setup_finally_break_entries() {
    let source = r"def f(queue, close, done):
    try:
        while True:
            item = queue.get()
            if item is None:
                break
            work(item)
    except Error as exc:
        report(exc)
    finally:
        try:
            close()
        finally:
            done()
";
    for (name, source) in [
        ("typed_handler", source.to_owned()),
        (
            "following_condition",
            source.replace(
                "            work(item)",
                "            if ready():\n                prepare()\n            work(item)",
            ),
        ),
        (
            "same_line_setup",
            source.replace("        try:\n            close()", "        try: close()"),
        ),
        (
            "prefixed_break",
            source.replace(
                "                break",
                "                report()\n                break",
            ),
        ),
    ] {
        common::assert_matches_cpython_marshal(&source, &format!("nested_setup_finally_{name}.py"));
    }
}

#[test]
fn matches_cpython_normal_finally_authored_break_await_keyword() {
    let source = r"async def target(queue, finished, pending, gather, task):
    try:
        while True:
            if not pending:
                if finished and not pending:
                    break
                item = await queue.get()
                pending.append(item)
                continue
            item = pending.pop()
            yield item
    except Exception:
        task.cancel()
        raise
    finally:
        await gather(task, return_exceptions=True)
        if pending:
            await gather(*pending, return_exceptions=True)
";
    common::assert_matches_cpython_marshal(
        source,
        "normal_finally_authored_break_await_keyword.py",
    );
}

#[test]
fn matches_cpython_normal_finally_authored_break_await_name_with() {
    let source = r"async def target(manager, queue, cleanup, record):
    with manager as token:
        total = 0
        try:
            while True:
                if total == 0:
                    item = await queue.get()
                else:
                    item = await queue.get()
                if isinstance(item, Exception):
                    raise item
                if item is None:
                    break
                total += 1
                yield item
        except Exception:
            record(token)
            raise
        finally:
            await cleanup
        fresh = total + 1
        record(token, total, fresh)
";
    common::assert_matches_cpython_marshal(
        source,
        "normal_finally_authored_break_await_name_with.py",
    );
}
