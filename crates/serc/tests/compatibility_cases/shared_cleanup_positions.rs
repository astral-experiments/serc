use crate::common;

#[test]
fn preserves_shared_handler_cleanup_positions() {
    for (filename, source) in [
        (
            "cleanup_join_async_nested_handler_then_assign.py",
            r"async def f(flag, values, manager, other):
    try:
        work()
    except Error as error:
        report()
        if flag and other:
            try:
                await primary()
            except Other:
                await fallback()
            result = None
    finish()
",
        ),
        (
            "cleanup_join_bare_nested_handler_then_call.py",
            r"def f(flag, values, manager, other):
    try:
        work()
    except Error:
        report()
        if flag:
            try:
                value = primary()
            except Other:
                value = fallback()
            consume(value)
    finish()
",
        ),
        (
            "cleanup_join_conditional_for.py",
            r"def f(flag, values, manager, other):
    try:
        work()
    except Error as error:
        report()
        if flag:
            for value in values:
                consume(value)
    finish()
",
        ),
        (
            "cleanup_join_conditional_with_then_call.py",
            r"def f(flag, values, manager, other):
    try:
        work()
    except Error as error:
        report()
        if flag:
            with manager:
                prepare()
            consume()
    finish()
",
        ),
        (
            "cleanup_join_conditional_with_then_for.py",
            r"def f(flag, values, manager, other):
    try:
        work()
    except Error as error:
        report()
        if flag:
            with manager:
                values = prepare()
            for value in values:
                consume(value)
    finish()
",
        ),
        (
            "cleanup_join_nested_handler_then_call.py",
            r"def f(flag, values, manager, other):
    try:
        work()
    except Error as error:
        report()
        if flag:
            try:
                value = primary()
            except Other:
                value = fallback()
            consume(value)
    finish()
",
        ),
        (
            "cleanup_join_control_suppressed_return.py",
            r"def f(flag, values, manager, other):
    try:
        work()
    except Error as error:
        report()
        if flag:
            with manager:
                return primary()
    return None
",
        ),
        (
            "cleanup_reverse_control_async_terminal.py",
            r"async def f(flag, other):
    try:
        await work()
    except Error as error:
        await report(error)
        if not flag or other:
            await recover(error)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_finally_exit_branch_positions() {
    for (filename, source) in [
        (
            "finally_async_for_handler_break.py",
            r"async def f(work, values, get):
    try:
        await work()
    finally:
        for value in values:
            try:
                await get(value)
            except LookupError:
                break
",
        ),
        (
            "finally_for_break_else.py",
            r"def f(work, values, get):
    try:
        work()
    finally:
        for value in values:
            try:
                get(value)
                break
            except LookupError:
                break
            except OSError:
                continue
        else:
            get(None)
",
        ),
        (
            "finally_for_conditional_break.py",
            r"def f(work, values, get, condition):
    try:
        work()
    finally:
        for value in values:
            if condition(value):
                break
            get(value)
",
        ),
        (
            "finally_for_handler_break.py",
            r"def f(work, values, get):
    try:
        work()
    finally:
        for value in values:
            try:
                get(value)
            except LookupError:
                break
",
        ),
        (
            "finally_for_two_breaks.py",
            r"def f(work, values, get):
    try:
        work()
    finally:
        for value in values:
            try:
                get(value)
                break
            except LookupError:
                break
            except OSError:
                continue
",
        ),
        (
            "finally_match_shared_reraise.py",
            r"def f(work, value, done):
    try:
        work()
    finally:
        match value:
            case 1:
                done(1)
            case 2:
                done(2)
            case _:
                done(0)
",
        ),
        (
            "finally_while_break.py",
            r"def f(work, get):
    try:
        work()
    finally:
        while True:
            try:
                get()
            except LookupError:
                break
",
        ),
        (
            "control_finally_for_no_break.py",
            r"def f(work, values, get):
    try:
        work()
    finally:
        for value in values:
            get(value)
",
        ),
        (
            "control_finally_while_no_break.py",
            r"def f(work, values, get):
    try:
        work()
    finally:
        while values:
            get(values.pop())
",
        ),
        (
            "control_finally_for_break_following.py",
            r"def f(work, values, get):
    try:
        work()
    finally:
        for value in values:
            try:
                get(value)
            except LookupError:
                break
        get(None)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
