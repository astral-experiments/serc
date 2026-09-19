use crate::common;

#[test]
fn named_handler_await_join_jump_target() {
    let source = r"async def run(flag):
    try:
        submit()
    except Busy:
        note()
        await observe()
        try:
            await send()
        except Error as error:
            if expected(error):
                record()
            else:
                report()
                await acknowledge()
        return
    await finish()
";
    common::assert_matches_cpython_marshal(source, "named_handler_await_join_jump_target.py");
}

#[test]
fn named_handler_await_join_jump_outer_free_control() {
    let source = r"async def run(flag):
    try:
        await send()
    except Error as error:
        if expected(error):
            record()
        else:
            report()
            await acknowledge()
    return
";
    common::assert_matches_cpython_marshal(
        source,
        "named_handler_await_join_jump_outer_free_control.py",
    );
}

#[test]
fn named_handler_loop_join_jump_target() {
    let source = r"def run(retry, marked):
    while True:
        try:
            prepare()
            try:
                action()
            except Fatal as error:
                report(error)
                if not marked:
                    record()
                    marked = True
                stop()
            except Error as error:
                delay = trim()
                if retry:
                    retry += 1
                    report(error)
                else:
                    report(error)
                    if not marked:
                        record()
                        marked = True
                    stop()
            else:
                report()
                retry = 0
                marked = False
        except Error as error:
            report(error)
";
    common::assert_matches_cpython_marshal(source, "named_handler_loop_join_jump_target.py");
}

#[test]
fn named_handler_loop_join_jump_linear_control() {
    let source = r"def run(retry, marked):
    while True:
        try:
            prepare()
            try:
                action()
            except Fatal as error:
                report(error)
                if not marked:
                    record()
                    marked = True
                stop()
            except Error as error:
                delay = trim()
                report(error)
                stop()
            else:
                report()
                retry = 0
                marked = False
        except Error as error:
            report(error)
";
    common::assert_matches_cpython_marshal(
        source,
        "named_handler_loop_join_jump_linear_control.py",
    );
}
