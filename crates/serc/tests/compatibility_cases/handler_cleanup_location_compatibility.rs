use crate::common;

#[test]
fn matches_cpython_cleanup_positions_for_folded_and_handlers() {
    let source = r"def true_and_value_control(value):
    try:
        action()
    except Error:
        if True and value:
            return value
    after()


def value_and_true_control(value):
    try:
        action()
    except Error:
        if value and True:
            return value
    after()


def false_and_true_control(value):
    try:
        action()
    except Error:
        if False and True:
            return value
    after()


def true_and_false_control(value):
    try:
        action()
    except Error:
        if True and False:
            return value
    after()


def true_and_true_control(value):
    try:
        action()
    except Error:
        if True and True:
            return value
    after()


def true_and_not_true_control(value):
    try:
        action()
    except Error:
        if True and not True:
            return value
    after()
";
    common::assert_matches_cpython_pop_except_positions(
        source,
        "folded_single_edge_and_handler_cleanup.py",
    );
}

#[test]
fn sole_handler_name_store_locates_manager_exit() {
    for (function, prefix, recovery) in [
        ("def", "", "recover(value)"),
        ("async def", "", "await recover(value)"),
        ("def", "    yield value\n", "recover(value)"),
        ("async def", "    yield value\n", "await recover(value)"),
    ] {
        for binding in ["", " as error"] {
            for terminal in ["return", "raise Error(value)"] {
                let source = format!(
                    "{function} f(manager, value):\n{prefix}    with manager:\n        try:\n            work(value)\n            {terminal}\n        except Error{binding}:\n            if invalid(value):\n                raise\n            result = {recovery}\n    consume(result)\n"
                );
                common::assert_matches_cpython_marshal(&source, "handler_name_manager_exit.py");
            }
        }
    }
}

#[test]
fn handler_store_location_preserves_continuation_barriers() {
    for assignment in [
        "first = result = recover(value)",
        "first, result = recover(value)",
        "result: object = recover(value)",
        "value.result = recover(value)",
        "value[0] = recover(value)",
    ] {
        let source = format!(
            "def f(manager, value):\n    with manager:\n        try:\n            return\n        except Error:\n            {assignment}\n        consume(value)\n    after()\n"
        );
        common::assert_matches_cpython_marshal(&source, "handler_store_continuation.py");
    }

    let source = r"def shared_handlers(manager, value):
    with manager:
        try:
            return
        except FirstError:
            result = recover(value)
        except SecondError:
            result = fallback(value)
    consume(result)


def shared_normal_and_handler(manager, value):
    with manager:
        try:
            result = work(value)
        except Error:
            result = recover(value)
    consume(result)


def compound_handler(manager, value, flag):
    with manager:
        try:
            return
        except Error:
            if flag:
                result = recover(value)
            else:
                result = fallback(value)
    consume(result)


def finalizer_barrier(manager, value):
    with manager:
        try:
            return
        except Error:
            result = recover(value)
        finally:
            cleanup(value)
    consume(result)
";
    common::assert_matches_cpython_marshal(source, "handler_store_source_barriers.py");
}
