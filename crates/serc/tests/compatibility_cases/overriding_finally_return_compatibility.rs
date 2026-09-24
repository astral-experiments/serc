use crate::common;

#[test]
fn matches_cpython_marshal_for_overriding_finally_returns() {
    let source = r"def typed_raise_from(distance, time):
    try:
        return distance / time
    except ZeroDivisionError as error:
        raise ValueError() from error
    finally:
        return 299792458

def typed_raise(distance, time):
    try:
        return distance / time
    except ZeroDivisionError:
        raise ValueError()
    finally:
        return 299792458

def typed_reraise(distance, time):
    try:
        return distance / time
    except ZeroDivisionError:
        raise
    finally:
        return 299792458

def bare_raise(distance, time):
    try:
        return distance / time
    except:
        raise
    finally:
        return 299792458

def two_handlers(distance, time):
    try:
        return distance / time
    except ZeroDivisionError as error:
        raise ValueError() from error
    except TypeError:
        raise RuntimeError()
    finally:
        return 299792458

def call_result(factory):
    try:
        return factory()
    except ValueError as error:
        raise RuntimeError() from error
    finally:
        return 299792458

def attribute_result(value):
    try:
        return value.result
    except AttributeError as error:
        raise RuntimeError() from error
    finally:
        return 299792458

def literal_result():
    try:
        return 1
    except ValueError as error:
        raise RuntimeError() from error
    finally:
        return 299792458

def statement_then_result(distance, time):
    try:
        record(distance)
        return distance / time
    except ZeroDivisionError as error:
        raise ValueError() from error
    finally:
        return 299792458

def finally_none(distance, time):
    try:
        return distance / time
    except ZeroDivisionError as error:
        raise ValueError() from error
    finally:
        return None

def finally_boolean(distance, time):
    try:
        return distance / time
    except ZeroDivisionError as error:
        raise ValueError() from error
    finally:
        return True

def no_handler(distance, time):
    try:
        return distance / time
    finally:
        return 299792458

async def coroutine(distance, time):
    await prepare(distance)
    try:
        return distance / time
    except ZeroDivisionError as error:
        raise ValueError() from error
    finally:
        return 299792458

def generator(distance, time):
    yield distance
    try:
        return distance / time
    except ZeroDivisionError as error:
        raise ValueError() from error
    finally:
        return 299792458
";
    common::assert_matches_cpython_marshal(source, "overriding_finally_returns.py");
}

#[test]
fn matches_cpython_handler_exits_into_local_finally_returns() {
    for (source, filename) in [
        (
            r"def f(work, value):
    try:
        work()
        value = False
    except Exception as error:
        report(error)
    finally:
        return value
",
            "finally_local_return_named_handler_target.py",
        ),
        (
            r"async def f(work, value):
    try:
        value = await work()
        record(value)
    except Exception:
        report()
    finally:
        return value
",
            "coroutine_finally_local_return_handler_target.py",
        ),
        (
            r"def f(work, value):
    try:
        work()
        value = False
    except Exception as error:
        report(error)
    return value
",
            "local_return_after_named_handler_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_pending_values_before_direct_finally_returns() {
    for (filename, source) in [
        (
            "pending_subscript_return.py",
            "def f(values):\n    try:\n        return values[1]\n    finally:\n        return values[0]\n",
        ),
        (
            "pending_call_return.py",
            "def f(factory, other):\n    try:\n        return factory()\n    finally:\n        return other()\n",
        ),
        (
            "pending_iterator_bare_return.py",
            "def f(values):\n    for value in values:\n        try:\n            return value\n        finally:\n            return\n",
        ),
        (
            "pending_iterator_subscript_return.py",
            "def f(values):\n    for value in values:\n        try:\n            return value\n        finally:\n            return values[0]\n",
        ),
        (
            "pending_two_iterators_return.py",
            "def f(values):\n    for value in values:\n        for other in values:\n            try:\n                return value\n            finally:\n                return other\n",
        ),
        (
            "pending_while_return.py",
            "def f(values, flag):\n    while flag:\n        try:\n            return values[1]\n        finally:\n            return values[0]\n",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_pending_return_through_conditional_finalizer_break() {
    for (filename, source) in [
        (
            "pending_return_conditional_finalizer_break.py",
            r"def f(items, value, stop):
    for item in items:
        try:
            value += item
            if item < 0:
                break
            return value
        finally:
            if stop:
                break
            return value
    return None
",
        ),
        (
            "pending_call_conditional_finalizer_break.py",
            r"def f(items, factory, value, stop):
    for item in items:
        try:
            return factory()
        finally:
            if stop:
                break
            return value
    return None
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_named_handler_return_overridden_inside_iterator() {
    for (filename, source) in [
        (
            "iterator_named_handler_local_return_overridden.py",
            "def f(work, items):\n    for item in items:\n        try:\n            work()\n        except Exception as error:\n            return work\n        finally:\n            return items\n",
        ),
        (
            "iterator_named_handler_constant_return_overridden.py",
            "def f(work, items):\n    for item in items:\n        try:\n            work()\n        except Exception as error:\n            return 1\n        finally:\n            return items\n",
        ),
        (
            "iterator_named_handler_bare_return_overridden.py",
            "def f(work, items):\n    for item in items:\n        try:\n            work()\n        except Exception as error:\n            return\n        finally:\n            return items\n",
        ),
        (
            "iterator_named_handler_call_return_overridden.py",
            "def f(work, items):\n    for item in items:\n        try:\n            work()\n        except Exception as error:\n            return work(error)\n        finally:\n            return items\n",
        ),
        (
            "iterator_named_handler_exception_return_overridden.py",
            "def f(work, items):\n    for item in items:\n        try:\n            work()\n        except Exception as error:\n            return error\n        finally:\n            return items\n",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
