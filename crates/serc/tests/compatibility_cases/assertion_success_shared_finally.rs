use crate::common;

#[test]
fn conditional_finally() {
    common::assert_matches_cpython_marshal(
        r"def f(item):
    try:
        work(item)
        assert not ready(item)
    except Error:
        recover(item)
    finally:
        if cleanup_ready(item):
            cleanup(item)
            wait(item)
        if item is not None and ready(item):
            finish(item)
",
        "conditional_finally.py",
    );
}

#[test]
fn one_conditional_finally() {
    common::assert_matches_cpython_marshal(
        r"def f(item):
    try:
        work(item)
        assert not ready(item)
    except Error:
        recover(item)
    finally:
        if cleanup_ready(item):
            cleanup(item)
            wait(item)
",
        "one_conditional_finally.py",
    );
}

#[test]
fn terminating_handler() {
    common::assert_matches_cpython_marshal(
        r"def f(item):
    try:
        work(item)
        assert not ready(item)
    except Error:
        raise
    finally:
        if cleanup_ready(item):
            cleanup(item)
            wait(item)
",
        "terminating_handler.py",
    );
}

#[test]
fn shared_assert_success() {
    common::assert_matches_cpython_marshal(
        r"def f(item):
    try:
        work(item)
        assert left(item) or right(item)
    except Error:
        recover(item)
    finally:
        if cleanup_ready(item):
            cleanup(item)
            wait(item)
",
        "shared_assert_success.py",
    );
}

#[test]
fn folded_assert_success() {
    common::assert_matches_cpython_marshal(
        r"def f(item):
    try:
        work(item)
        assert True or ready(item)
    except Error:
        recover(item)
    finally:
        if cleanup_ready(item):
            cleanup(item)
            wait(item)
",
        "folded_assert_success.py",
    );
}

#[test]
fn named_fallthrough_handler() {
    common::assert_matches_cpython_marshal(
        r"def f(item):
    try:
        work(item)
        assert not ready(item)
    except Error as error:
        recover(error)
    finally:
        if cleanup_ready(item):
            cleanup(item)
            wait(item)
",
        "named_fallthrough_handler.py",
    );
}

#[test]
fn conditional_pass_cleanup() {
    common::assert_matches_cpython_marshal(
        r"def f(item):
    try:
        work(item)
        assert not ready(item)
    except Error:
        recover(item)
    finally:
        if cleanup_ready(item):
            pass
",
        "conditional_pass_cleanup.py",
    );
}

#[test]
fn conditional_return_cleanup() {
    common::assert_matches_cpython_marshal(
        r"def f(item):
    try:
        work(item)
        assert not ready(item)
    except Error:
        recover(item)
    finally:
        if cleanup_ready(item):
            return item
        else:
            return None
",
        "conditional_return_cleanup.py",
    );
}
