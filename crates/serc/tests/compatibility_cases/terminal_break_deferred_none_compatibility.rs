use crate::common;

#[test]
fn matches_cpython_marshal_for_cleanup_free_terminal_breaks() {
    let source = r#"def plain(flag):
    while True:
        if flag():
            break
        use("later")


def prior_constant(flag):
    use("before")
    while True:
        if flag():
            break
        use("later")


def prior_none(flag):
    use(None)
    while True:
        if flag():
            break
        use("later")


def multiple_breaks(value):
    while True:
        if value != 0:
            break
        if value == 0:
            break
        value -= 10


def nested(flag):
    while True:
        while True:
            if flag():
                break
            use("inner")
        if flag():
            break
        use("outer")


async def coroutine(flag):
    while True:
        if await flag():
            break
        use("later")


def bounded(flag):
    while flag():
        if flag():
            break
        use("later")


def following(flag):
    while True:
        if flag():
            break
        use("later")
    use("after")
"#;

    common::assert_matches_cpython_marshal(source, "terminal_break_deferred_none.py");
}

#[test]
fn matches_cpython_handler_break_implicit_none_copy() {
    for (filename, source) in [
        (
            "copied_finally_handler_break_implicit_none_target.py",
            r"async def run(work, cleanup, stop, report):
    while True:
        try:
            try:
                await work()
            except RuntimeError:
                if stop:
                    break
                report()
            finally:
                try:
                    await cleanup()
                except ValueError:
                    pass
        except Exception:
            report()
",
        ),
        (
            "copied_finally_linear_break_implicit_none_control.py",
            r"async def run(work, cleanup, stop, report):
    while True:
        try:
            try:
                await work()
            except RuntimeError:
                if stop:
                    break
                report()
            finally:
                await cleanup()
        except Exception:
            report()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
