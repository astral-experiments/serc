use crate::common;

#[test]
fn matches_cpython_marshal_for_named_handler_else_bare_raise_layout() {
    let source = r"def assignment_follower(value):
    try:
        value = action(value)
    except Error as error:
        if matches(error):
            value = fallback(error)
        else:
            raise
    return value

def conjunction_follower(value):
    try:
        value = action(value)
    except Error as error:
        if matches(error) and enabled(value):
            value = fallback(error)
        else:
            raise
    return value

def two_assignments(value):
    try:
        value = action(value)
    except Error as error:
        if matches(error):
            value = fallback(error)
            recovered = True
        else:
            raise
    return value, recovered

def expression_try_body(value):
    try:
        action(value)
    except Error as error:
        if matches(error):
            value = fallback(error)
        else:
            raise
    return value

async def coroutine_handler(value):
    try:
        await action(value)
    except Error as error:
        if matches(error):
            value = fallback(error)
        else:
            raise
    return value

class ClassControl:
    try:
        action(value)
    except Error as error:
        if matches(error):
            value = fallback(error)
        else:
            raise
";
    common::assert_matches_cpython_marshal(source, "named_handler_else_bare_raise_layout.py");
}

#[test]
fn matches_cpython_assignment_cleanup_after_raising_alternative() {
    for handler in ["Error", "Error as error"] {
        for raising in ["raise", "raise Error(b)", "raise Error(b) from cause"] {
            let source = format!(
                "def f(a, b, items):\n    try:\n        value = work()\n    except {handler}:\n        if a:\n            report()\n            value = None\n        else:\n            report()\n            {raising}\n    return value\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "assignment_cleanup_after_raising_alternative.py",
            );
        }
    }
}

#[test]
fn matches_cpython_raising_alternative_in_enclosing_contexts() {
    for source in [
        r"def f(a, b, items):
    try:
        value = work()
    except Error:
        if a:
            report()
            value = None
        else:
            report()
            raise Error(b)
",
        r"async def f(a, b, items):
    try:
        value = work()
    except Error:
        if a:
            report()
            value = None
        else:
            report()
            raise Error(b)
    return value
",
        r"def f(a, b, items):
    with manager():
        try:
            value = work()
        except Error:
            if a:
                value = None
            else:
                report()
                raise Error(b) from cause
    return value
",
        r"def f(a, b, items):
    for item in items:
        try:
            value = work()
        except Error:
            if a:
                value = None
            else:
                report()
                raise Error(b) from cause
    return value
",
    ] {
        for handler in ["Error", "Error as error"] {
            let source = source.replace("except Error:", &format!("except {handler}:"));
            common::assert_matches_cpython_marshal(
                &source,
                "raising_alternative_in_enclosing_contexts.py",
            );
        }
    }
}

#[test]
fn matches_cpython_finally_entry_after_normal_handler_cleanup_copy() {
    for source in [
        r"def f(a, b, flag, other, value, items, manager, cause):
    try:
        work(value)
        return value
    except Error:
        if b:
            report(value)
            value = None
        else:
            report(other)
            raise Error(value)
    finally:
        if flag:
            consume(value, other)
    consume(value, other)
",
        r"async def f(a, b, flag, other, value, items, manager, cause):
    try:
        await work(value)
        return value
    except Error:
        if b:
            report(value)
            value = None
        else:
            report(other)
            raise Error(value)
    finally:
        if flag:
            consume(value, other)
    consume(value, other)
",
        r"def f(a, b, flag, other, value, items, manager, cause):
    with manager:
        try:
            work(value)
            return value
        except Error:
            if b:
                report(value)
                value = None
            else:
                report(other)
                raise Error(value)
        finally:
            if flag:
                consume(value, other)
    consume(value, other)
",
        r"def f(a, b, flag, other, value, items, manager, cause):
    for item in items:
        try:
            work(value)
            return value
        except Error:
            if b:
                report(value)
                value = None
            else:
                report(other)
                raise Error(value)
        finally:
            if flag:
                consume(value, other)
    consume(value, other)
",
    ] {
        common::assert_matches_cpython_marshal(
            source,
            "finally_entry_after_normal_handler_cleanup_copy.py",
        );
    }
}
