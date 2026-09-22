use crate::common;

#[test]
fn matches_cpython_match_call_arms_before_literal_return() {
    let source = r"def choose(value):
    try:
        match value:
            case 'first':
                first()
            case 'second':
                second()
    except Error:
        return 1
    return 0
";
    common::assert_matches_cpython_marshal(source, "match_call_literal_return.py");
}

#[test]
fn matches_cpython_match_call_arms_with_named_handler() {
    let source = r"def choose(options):
    try:
        value, extra = prepare(options)
        match options.mode:
            case 'first':
                first(value, extra)
            case 'second':
                second(value)
            case 'third':
                third(value, extra)
    except Exception as error:
        report(error)
        return 1
    return 0
";
    common::assert_matches_cpython_marshal(source, "match_call_named_handler.py");
}

#[test]
fn matches_cpython_match_call_following_call_control() {
    let source = r"def choose(value):
    try:
        match value:
            case 'first':
                first()
            case 'second':
                second()
    except Error:
        return 1
    finish()
    return 0
";
    common::assert_matches_cpython_marshal(source, "match_call_following_call_control.py");
}

#[test]
fn matches_cpython_match_terminating_arms_control() {
    let source = r"def choose(value):
    try:
        match value:
            case 'first':
                return first(value)
            case 'second':
                return second(value)
    except Error:
        return recover(value)
    raise Failure(value)
";
    common::assert_matches_cpython_marshal(source, "match_terminating_arms_control.py");
}

#[test]
fn matches_cpython_match_without_try_control() {
    let source = r"def choose(value):
    match value:
        case 'first':
            first()
        case 'second':
            second()
    return 0
";
    common::assert_matches_cpython_marshal(source, "match_without_try_control.py");
}

#[test]
fn matches_cpython_final_value_match_conditional_success() {
    let source = r"async def f(value, work, finish):
    await work(value)
    try:
        match value:
            case Kind.FIRST:
                if value.enabled:
                    left, right = work(value)
                else:
                    left, right = work(value)
            case Kind.LAST:
                left, right = work(value)
    except ValueError as error:
        raise RuntimeError(value) from error
    return finish(left, right, value)
";
    common::assert_matches_cpython_marshal(source, "final_value_match_conditional_success.py");
}

#[test]
fn matches_cpython_final_value_match_nested_conditional_success() {
    let source = r"async def f(value, work, finish):
    await work(value)
    try:
        match value:
            case Kind.FIRST:
                if value.enabled:
                    left, right = work(value)
                else:
                    if value.ready:
                        left, right = work(value)
            case Kind.LAST:
                left, right = work(value)
    except ValueError as error:
        raise RuntimeError(value) from error
    return finish(left, right, value)
";
    common::assert_matches_cpython_marshal(
        source,
        "final_value_match_nested_conditional_success.py",
    );
}

#[test]
fn matches_cpython_final_value_match_try_exit() {
    let source = r"async def f(value, work, finish):
    await work(value)
    try:
        match value:
            case Kind.FIRST:
                left, right = work(value)
            case Kind.LAST:
                left, right = work(value)
    except ValueError as error:
        raise RuntimeError(value) from error
    return finish(left, right, value)
";
    common::assert_matches_cpython_marshal(source, "final_value_match_try_exit.py");
}
#[test]
fn matches_cpython_wildcard_elif_await_try_exit_target() {
    let source = r#"async def f(value, mode, work, report, reject):
    result = 0
    try:
        match value:
            case First():
                result = 1
            case Second():
                details = await work(value)
                if mode == "present" and details is not None:
                    result = 1
                elif mode == "absent" and details is None:
                    result = 2
            case Last():
                result = await work(value)
            case _:
                reject(value)
    except Exception as error:
        report(error)
    bias = "up" if result > 1 else ("down" if result < 1 else None)
    return result, bias
"#;
    common::assert_matches_cpython_marshal(source, "wildcard_elif_await_try_exit_target.py");
}

#[test]
fn matches_cpython_wildcard_complete_else_await_try_exit_control() {
    let source = r#"async def f(value, mode, work, report, reject):
    result = 0
    try:
        match value:
            case First():
                result = 1
            case Second():
                details = await work(value)
                if mode == "present" and details is not None:
                    result = 1
                elif mode == "absent" and details is None:
                    result = 2
                else:
                    result = 0
            case Last():
                result = await work(value)
            case _:
                reject(value)
    except Exception as error:
        report(error)
    bias = "up" if result > 1 else ("down" if result < 1 else None)
    return result, bias
"#;
    common::assert_matches_cpython_marshal(
        source,
        "wildcard_complete_else_await_try_exit_control.py",
    );
}

#[test]
fn matches_cpython_async_for_match_success_try_finally_target() {
    let source = r"async def f(command, first, last, ready, close, record, kwargs):
    await ready()
    try:
        match command:
            case Kind.FIRST:
                async for item in first(**kwargs):
                    yield item
            case Kind.LAST:
                async for item in last(**kwargs):
                    yield item
            case _:
                raise RuntimeError(command)
    finally:
        await close()
";
    common::assert_matches_cpython_marshal(source, "async_for_match_success_try_finally_target.py");
}

#[test]
fn matches_cpython_async_for_match_success_continuation_control() {
    let source = r"async def f(command, first, last, ready, close, record, kwargs):
    await ready()
    try:
        match command:
            case Kind.FIRST:
                async for item in first(**kwargs):
                    yield item
            case Kind.LAST:
                async for item in last(**kwargs):
                    yield item
                record(command)
            case _:
                raise RuntimeError(command)
    finally:
        await close()
";
    common::assert_matches_cpython_marshal(
        source,
        "async_for_match_success_continuation_control.py",
    );
}
#[test]
fn matches_cpython_outer_if_match_return_join_target() {
    let source = r"def f(items, mode, active, ready, report):
    if active:
        match mode:
            case Mode.FIRST:
                report(items)
                if ready:
                    return [items[0]]
            case Mode.SEARCH:
                found = None
                for item in items:
                    if item.ready:
                        found = item
                        break
                if found is not None:
                    return [found]
                else:
                    report(items)
            case Mode.ALL:
                report(items)
                pass
            case _:
                raise ValueError(mode)
    return items
";
    common::assert_matches_cpython_marshal(source, "outer_if_match_return_join_target.py");
}

#[test]
fn matches_cpython_direct_match_return_join_control() {
    let source = r"def f(items, mode, active, ready, report):
    match mode:
        case Mode.FIRST:
            report(items)
            if ready:
                return [items[0]]
        case Mode.SEARCH:
            found = None
            for item in items:
                if item.ready:
                    found = item
                    break
            if found is not None:
                return [found]
            else:
                report(items)
        case Mode.ALL:
            report(items)
            pass
        case _:
            raise ValueError(mode)
    return items
";
    common::assert_matches_cpython_marshal(source, "direct_match_return_join_control.py");
}

#[test]
fn matches_cpython_outer_if_match_literal_return_control() {
    let source = r"def f(items, mode, active, ready, report):
    if active:
        match mode:
            case Mode.FIRST:
                report(items)
                if ready:
                    return [items[0]]
            case Mode.SEARCH:
                found = None
                for item in items:
                    if item.ready:
                        found = item
                        break
                if found is not None:
                    return [found]
                else:
                    report(items)
            case Mode.ALL:
                report(items)
                pass
            case _:
                raise ValueError(mode)
    return None
";
    common::assert_matches_cpython_marshal(source, "outer_if_match_literal_return_control.py");
}

#[test]
fn matches_cpython_outer_if_match_elif_name_return_control() {
    let source = r"def f(mode, active, first, second, work, items):
    if active:
        match mode:
            case Mode.FIRST:
                work(items)
            case Mode.LAST:
                if first:
                    work(items)
                elif second:
                    work(items)
    return items
";
    common::assert_matches_cpython_marshal(source, "outer_if_match_elif_name_return_control.py");
}

#[test]
fn matches_cpython_direct_match_elif_name_return_control() {
    let source = r"def f(mode, active, first, second, work, items):
    match mode:
        case Mode.FIRST:
            work(items)
        case Mode.LAST:
            if first:
                work(items)
            elif second:
                work(items)
    return items
";
    common::assert_matches_cpython_marshal(source, "direct_match_elif_name_return_control.py");
}

#[test]
fn matches_cpython_value_match_failure_before_finally() {
    for suspension in ["", "await "] {
        let prefix = if suspension.is_empty() { "" } else { "async " };
        let source = format!(
            "{prefix}def choose(value):\n    try:\n        match value:\n            case Choices.first:\n                {suspension}first()\n            case Choices.second:\n                {suspension}second()\n    except Error as error:\n        raise OtherError() from error\n    finally:\n        {suspension}cleanup()\n"
        );
        common::assert_matches_cpython_marshal(&source, "value_match_finally.py");
    }
}

#[test]
fn matches_cpython_value_match_finally_receiver_borrow() {
    let source = r"async def choose(self, mode):
    try:
        match self.kind:
            case Choices.first:
                await self.first(mode)
            case Choices.second:
                await self.second(mode)
    except Error as error:
        raise OtherError() from error
    finally:
        await self.coordinator.refresh()
";
    common::assert_matches_cpython_marshal(source, "value_match_finally_receiver_borrow.py");
}
