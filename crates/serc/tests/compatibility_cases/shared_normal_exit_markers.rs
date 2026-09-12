use crate::common;

#[test]
fn matches_cpython_while_break_exit_markers() {
    for (filename, source) in [
        (
            "async_try_while_tail_break.py",
            r"async def f(ready, step, result):
    try:
        while True:
            await step()
            if not ready():
                break
    except RuntimeError as error:
        recover(error)
        raise error
    return result
",
        ),
        (
            "try_while_else_break.py",
            r"def f(ready, step, result):
    try:
        while True:
            step()
            if ready():
                result = step()
            else:
                break
    except RuntimeError:
        recover()
    return result
",
        ),
        (
            "try_while_tail_break.py",
            r"def f(ready, step, result):
    try:
        while True:
            step()
            if ready():
                break
    except RuntimeError:
        return None
    return result
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_match_nested_branch_markers() {
    for (filename, source) in [
        (
            "match_final_if_assignment.py",
            r"def f(subject, flag, first, second, consume):
    match subject:
        case 0:
            value = first()
        case 1:
            if flag:
                value = first()
            else:
                value = second()
    consume(value)
    return value
",
        ),
        (
            "match_final_if_await.py",
            r"async def f(subject, flag, first, second):
    match subject:
        case 0:
            await first()
        case 1:
            if flag:
                await first()
            else:
                await second()
    return True
",
        ),
        (
            "match_linear_control.py",
            r"def f(subject, first, second, consume):
    match subject:
        case 0:
            value = first()
        case 1:
            value = second()
    consume(value)
    return value
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_while_break_borrow_routes() {
    for (filename, source) in [
        (
            "while_break_large_borrow_route.py",
            r"def f(step, done, value, consume):
    try:
        while True:
            step()
            if done():
                break
    except OSError:
        return None
    consume(value)
    return value
",
        ),
        (
            "while_break_manager_borrow_route.py",
            r"async def f(manager, poll):
    async with manager:
        try:
            while True:
                value = await poll()
                if value is not None:
                    break
        except OSError:
            return None
    return value
",
        ),
        (
            "while_break_loop_suffix_borrow_route.py",
            r"def f(step, done, value):
    try:
        while True:
            step()
            if done():
                break
    except OSError:
        return None
    result: list = []
    for item in value:
        result.append(item)
    return result
",
        ),
        (
            "while_break_named_retry_route_control.py",
            r"def f(ready, other, step, finish, result):
    try:
        while True:
            step()
            if ready() or other():
                break
    except RuntimeError as error:
        try:
            while True:
                step()
                if ready() or other():
                    break
        except ValueError as retry_error:
            step()
    finish(result)
    return result
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
