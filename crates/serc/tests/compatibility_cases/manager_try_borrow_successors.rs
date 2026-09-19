use crate::common;

fn check_manager_try(is_async: bool, body: &str) {
    let declaration = if is_async { "async def" } else { "def" };
    let manager = if is_async { "async with" } else { "with" };
    let mut source = format!(
        "{declaration} f(value, other, manager, inner, flag, items, consume):\n    {manager} manager:\n"
    );
    for line in body.lines() {
        source.push_str("        ");
        source.push_str(line);
        source.push('\n');
    }
    source.push_str("    consume(value)\n    return value\n");
    common::assert_matches_cpython_marshal(&source, "manager_try_borrow_successor.py");
}

#[test]
fn terminal_manager_try_keeps_empty_normal_exit_blocks() {
    for is_async in [false, true] {
        for body in [
            "with inner:\n    consume(value)\n",
            "try:\n    consume(value)\nexcept InnerError:\n    consume(other)\n",
        ] {
            for handler in [
                "except Error:\n    pass\n",
                "except Error:\n    consume(other)\n",
                "except Error as exc:\n    consume(exc)\n",
                "except Error:\n    try:\n        consume(other)\n    except InnerError:\n        raise\n",
            ] {
                let mut suite = String::from("try:\n");
                for line in body.lines() {
                    suite.push_str("    ");
                    suite.push_str(line);
                    suite.push('\n');
                }
                suite.push_str(handler);
                check_manager_try(is_async, &suite);
            }
        }
    }
}

#[test]
fn terminal_manager_try_keeps_nested_handler_empty_exits() {
    for is_async in [false, true] {
        for tail in ["consume(value)", "raise"] {
            let suite = format!(
                "try:\n    consume(value)\nexcept Error:\n    try:\n        consume(other)\n    except InnerError:\n        {tail}\n"
            );
            check_manager_try(is_async, &suite);
        }
    }
}

#[test]
fn manager_cleanup_does_not_replace_intervening_successors() {
    for is_async in [false, true] {
        for wrapper in [
            "branch_empty",
            "branch_populated",
            "loop",
            "following",
            "except",
        ] {
            let body = "try:\n    with inner:\n        consume(value)\nexcept Error:\n    pass\n";
            let mut suite = String::new();
            if wrapper == "following" {
                suite.push_str(body);
                suite.push_str("consume(other)\n");
            } else {
                suite.push_str(match wrapper {
                    "loop" => "for item in items:\n",
                    "except" => "try:\n",
                    _ => "if flag:\n",
                });
                for line in body.lines() {
                    suite.push_str("    ");
                    suite.push_str(line);
                    suite.push('\n');
                }
                if wrapper == "branch_populated" {
                    suite.push_str("else:\n    consume(other)\n");
                } else if wrapper == "except" {
                    suite.push_str("except OuterError:\n    consume(other)\n");
                }
            }
            check_manager_try(is_async, &suite);
        }
    }
}

#[test]
fn terminal_manager_try_preserves_retained_loop_exit_nops() {
    for is_async in [false, true] {
        for condition in ["flag", "poll()"] {
            for handler in [
                "try:\n    consume(other)\nexcept InnerError:\n    pass\n",
                "try:\n    consume(other)\nexcept InnerError:\n    consume(value)\ntry:\n    consume(other)\nexcept OtherError:\n    consume(value)\n",
            ] {
                let declaration = if is_async { "async def" } else { "def" };
                let manager = if is_async { "async with" } else { "with" };
                let mut source = format!(
                    "{declaration} f(value, other, manager, flag, items, consume, poll):\n    {manager} manager:\n        try:\n            while {condition}:\n                consume(value)\n                flag = poll()\n        except Error:\n"
                );
                for line in handler.lines() {
                    source.push_str("            ");
                    source.push_str(line);
                    source.push('\n');
                }
                source.push_str("    return None\n");
                common::assert_matches_cpython_marshal(&source, "manager_try_retained_loop_nop.py");
            }
        }
    }
}

#[test]
fn retained_nop_stops_borrowing_sync_while_flag_one_live_else0() {
    common::assert_matches_cpython_marshal(
        r"def f(value, other, manager, flag, items, consume, poll):
    with manager:
        try:
            while flag:
                consume(value)
                flag = poll()
        except Error:
            try:
                consume(other)
            except InnerError:
                consume(value)
    return value, other
",
        "002_sync_while_flag_one_live_else0.py",
    );
}

#[test]
fn retained_nop_stops_borrowing_sync_while_poll_two_live_else0() {
    common::assert_matches_cpython_marshal(
        r"def f(value, other, manager, flag, items, consume, poll):
    with manager:
        try:
            while poll() is None:
                consume(value)
                consume(other)
        except Error:
            try:
                consume(other)
            except InnerError:
                consume(value)
            try:
                consume(other)
            except InnerError:
                consume(other)
    return value, other
",
        "020_sync_while_poll_two_live_else0.py",
    );
}

#[test]
fn retained_nop_stops_borrowing_sync_while_flag_two_mixed_else0() {
    common::assert_matches_cpython_marshal(
        r"def f(value, other, manager, flag, items, consume, poll):
    with manager:
        try:
            while flag:
                consume(value)
                flag = poll()
        except Error:
            try:
                consume(other)
            except InnerError:
                pass
            try:
                consume(other)
            except InnerError:
                consume(other)
    return value, other
",
        "010_sync_while_flag_two_mixed_else0.py",
    );
}

#[test]
fn retained_nop_stops_borrowing_async_while_flag_one_live_else0() {
    common::assert_matches_cpython_marshal(
        r"async def f(value, other, manager, flag, items, consume, poll):
    async with manager:
        try:
            while flag:
                consume(value)
                flag = poll()
        except Error:
            try:
                consume(other)
            except InnerError:
                consume(value)
    return value, other
",
        "038_async_while_flag_one_live_else0.py",
    );
}

#[test]
fn retained_nop_stops_borrowing_async_while_poll_one_terminated_else0() {
    common::assert_matches_cpython_marshal(
        r"async def f(value, other, manager, flag, items, consume, poll):
    async with manager:
        try:
            while poll() is None:
                consume(value)
                consume(other)
        except Error:
            try:
                consume(other)
            except InnerError:
                raise
    return value, other
",
        "052_async_while_poll_one_terminated_else0.py",
    );
}

#[test]
fn retained_nop_stops_borrowing_async_while_poll_two_mixed_else0() {
    common::assert_matches_cpython_marshal(
        r"async def f(value, other, manager, flag, items, consume, poll):
    async with manager:
        try:
            while poll() is None:
                consume(value)
                consume(other)
        except Error:
            try:
                consume(other)
            except InnerError:
                pass
            try:
                consume(other)
            except InnerError:
                consume(other)
    return value, other
",
        "058_async_while_poll_two_mixed_else0.py",
    );
}

#[test]
fn keeps_manager_continuation_reachable_after_conditional_recovery() {
    for is_async in [false, true] {
        for loop_body in [false, true] {
            for named_handler in [false, true] {
                let declaration = if is_async { "async def" } else { "def" };
                let manager = if is_async { "async with" } else { "with" };
                let handler = if named_handler {
                    "except Error as error:"
                } else {
                    "except Error:"
                };
                let body = if loop_body {
                    "            while flag:\n                consume(value)\n                flag = poll()\n"
                } else {
                    "            consume(value)\n"
                };
                let source = format!(
                    "{declaration} f(value, other, manager, flag, consume, poll):\n    {manager} manager:\n        try:\n{body}        {handler}\n            if flag:\n                try:\n                    consume(other)\n                except InnerError:\n                    consume(value)\n            consume(other)\n    return value, other\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "conditional_recovery_manager_successor.py",
                );
            }
        }
    }
}

#[test]
fn terminal_recovery_keeps_following_handler_statements() {
    for is_async in [false, true] {
        for loop_body in [false, true] {
            for terminal in ["raise", "return other"] {
                let declaration = if is_async { "async def" } else { "def" };
                let manager = if is_async { "async with" } else { "with" };
                let body = if loop_body {
                    "            while flag:\n                consume(value)\n                flag = poll()\n"
                } else {
                    "            consume(value)\n"
                };
                let source = format!(
                    "{declaration} f(value, other, manager, flag, consume, poll):\n    {manager} manager:\n        try:\n{body}        except Error:\n            consume(value)\n            try:\n                consume(other)\n            except InnerError:\n                {terminal}\n            consume(other)\n    return value, other\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "terminal_recovery_with_handler_continuation.py",
                );
            }
        }
    }
}
