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
