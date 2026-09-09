use crate::common;

#[test]
fn preserves_interrupt_checks_after_normal_manager_cleanup() {
    let source = r"def normal_sync(flag, value):
    if flag:
        with manager():
            try:
                return work()
            except Error:
                pass
    return work(value)

async def normal_async_manager(flag, value):
    if flag:
        async with manager():
            try:
                return work()
            except Error:
                pass
    return await work()

async def normal_sync_manager_in_coroutine(flag, value):
    if flag:
        with manager():
            try:
                return work()
            except Error:
                pass
    return await work()

def small_return_control(flag, value):
    if flag:
        with manager():
            try:
                return work()
            except Error:
                pass
    return value

def terminating_handler_control(flag, value):
    if flag:
        with manager():
            try:
                return work()
            except Error:
                return value
    return work(value)
";
    common::assert_matches_cpython_marshal(source, "normal_manager_cleanup_interrupts.py");
}

#[test]
fn preserves_interrupt_checks_through_match_cleanup_continuations() {
    let source = r"def match_sync(value):
    match value:
        case 1:
            with manager():
                work()
    work()

async def match_async(value):
    match value:
        case 1:
            async with manager():
                work()
    await work()

def conditional_match(value, ready):
    match value:
        case 1:
            work()
            if ready:
                try:
                    work()
                except Error:
                    recover()
    work()

def conditional_match_loop(values, ready):
    for value in values:
        match value:
            case 1:
                if ready:
                    try:
                        work()
                    except Error:
                        recover()
            case _:
                work()
        work()

def noninterruptible_if_control(value):
    if value == 1:
        with manager():
            work()
    work()
";
    common::assert_matches_cpython_marshal(source, "match_cleanup_continuation_interrupts.py");
}

#[test]
fn preserves_authored_break_interrupts_through_unlocated_handler_cleanup() {
    let source = r"def f(manager, values, flag):
    try:
        work()
    except Error:
        try:
            while True:
                before()
                if flag:
                    break
                after()
        except Other:
            ordinary()
    following()
    return flag
";
    common::assert_matches_cpython_marshal(source, "cleanup_break_16.py");
    let source = r"def f(manager, values, flag):
    try:
        work()
    except Error as error:
        try:
            while True:
                before()
                if flag:
                    break
                after()
        except Other:
            ordinary()
    following()
    return flag
";
    common::assert_matches_cpython_marshal(source, "cleanup_break_18.py");
}

#[test]
fn keeps_terminal_cleanup_controls_byte_exact() {
    let source = r"def f(manager, values, flag):
    try:
        work()
    except Error:
        try:
            while True:
                before()
                if flag:
                    break
                after()
        except Other:
            ordinary()
        return flag
    following()
    return flag
";
    common::assert_matches_cpython_marshal(source, "cleanup_break_24.py");
    let source = r"def f(manager, values, flag):
    try:
        work()
    except Error:
        try:
            for item in values:
                before()
                if flag:
                    break
                after()
        except Other:
            ordinary()
        raise
    following()
    return flag
";
    common::assert_matches_cpython_marshal(source, "cleanup_break_172.py");
}
