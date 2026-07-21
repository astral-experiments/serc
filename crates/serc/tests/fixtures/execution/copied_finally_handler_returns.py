import asyncio
import sys


def sync_override(events):
    try:
        raise ValueError("outer")
    except (ValueError, LookupError) as outer:
        events.append(("outer-handler", type(sys.exception()).__name__, str(outer)))
        return "outer-value"
    finally:
        events.append(("finally-entry", type(sys.exception()).__name__))
        try:
            raise TypeError("inner")
        except (TypeError, RuntimeError) as inner:
            events.append(("inner-handler", type(sys.exception()).__name__, str(inner)))
            return "inner-value"


def sync_fallthrough(events):
    try:
        raise ValueError("outer")
    except ValueError:
        events.append(("outer-handler", type(sys.exception()).__name__))
        return object()
    finally:
        try:
            events.append(("finally-body", type(sys.exception()).__name__))
        except TypeError:
            return "unexpected"


def sync_propagates(events):
    try:
        raise ValueError("outer")
    except ValueError:
        events.append(("outer-handler", type(sys.exception()).__name__))
        return "outer-value"
    finally:
        try:
            raise RuntimeError("propagated")
        except TypeError:
            return "unexpected"


def sync_name_override(events):
    saved = ("outer-name", len(events))
    try:
        raise ValueError("outer")
    except ValueError:
        events.append(("name-outer", type(sys.exception()).__name__))
        return saved
    finally:
        try:
            raise TypeError("inner")
        except TypeError:
            events.append(("name-inner", type(sys.exception()).__name__))
            return ("inner-name", len(events))


def make_value(events, label):
    events.append(("make", label, type(sys.exception()).__name__))
    return label


def sync_call_bare_override(events):
    try:
        raise ValueError("outer")
    except:
        return make_value(events, "outer-call")
    finally:
        try:
            raise TypeError("inner")
        except:
            return make_value(events, "inner-call")


def generator_override(events):
    yield "tick"
    try:
        raise ValueError("outer")
    except ValueError as outer:
        events.append(("generator-outer", type(sys.exception()).__name__, str(outer)))
        return "outer-value"
    finally:
        try:
            raise TypeError("inner")
        except TypeError as inner:
            events.append(("generator-inner", type(sys.exception()).__name__, str(inner)))
            return "inner-value"


async def coroutine_override(events):
    await asyncio.sleep(0)
    try:
        raise ValueError("outer")
    except ValueError as outer:
        events.append(("coroutine-outer", type(sys.exception()).__name__, str(outer)))
        return "outer-value"
    finally:
        try:
            raise TypeError("inner")
        except TypeError as inner:
            events.append(("coroutine-inner", type(sys.exception()).__name__, str(inner)))
            return "inner-value"


async def coroutine_name_override(events):
    saved = "outer-coroutine-name"
    await asyncio.sleep(0)
    try:
        raise ValueError("outer")
    except ValueError as outer:
        events.append(("coroutine-name-outer", type(sys.exception()).__name__, str(outer)))
        return saved
    finally:
        try:
            raise TypeError("inner")
        except TypeError as inner:
            events.append(("coroutine-name-inner", type(sys.exception()).__name__, str(inner)))
            return make_value(events, "inner-coroutine-call")


def generator_call_override(events):
    yield "call-tick"
    try:
        raise ValueError("outer")
    except ValueError:
        return make_value(events, "outer-generator-call")
    finally:
        try:
            raise TypeError("inner")
        except TypeError:
            return make_value(events, "inner-generator-call")


events = []
values = [sync_override(events)]
values.append(type(sync_fallthrough(events)).__name__)
try:
    sync_propagates(events)
except RuntimeError as error:
    values.append((type(error).__name__, str(error)))
values.append(sync_name_override(events))
values.append(sync_call_bare_override(events))
iterator = generator_override(events)
values.append(next(iterator))
try:
    next(iterator)
except StopIteration as error:
    values.append(error.value)
call_iterator = generator_call_override(events)
values.append(next(call_iterator))
try:
    next(call_iterator)
except StopIteration as error:
    values.append(error.value)
values.append(asyncio.run(coroutine_override(events)))
values.append(asyncio.run(coroutine_name_override(events)))
values.append(type(sys.exception()).__name__)

assert tuple(values) == (
    "inner-value",
    "object",
    ("RuntimeError", "propagated"),
    ("inner-name", 8),
    "inner-call",
    "tick",
    "inner-value",
    "call-tick",
    "inner-generator-call",
    "inner-value",
    "inner-coroutine-call",
    "NoneType",
)
assert tuple(events) == (
    ("outer-handler", "ValueError", "outer"),
    ("finally-entry", "NoneType"),
    ("inner-handler", "TypeError", "inner"),
    ("outer-handler", "ValueError"),
    ("finally-body", "NoneType"),
    ("outer-handler", "ValueError"),
    ("name-outer", "ValueError"),
    ("name-inner", "TypeError"),
    ("make", "outer-call", "ValueError"),
    ("make", "inner-call", "TypeError"),
    ("generator-outer", "ValueError", "outer"),
    ("generator-inner", "TypeError", "inner"),
    ("make", "outer-generator-call", "ValueError"),
    ("make", "inner-generator-call", "TypeError"),
    ("coroutine-outer", "ValueError", "outer"),
    ("coroutine-inner", "TypeError", "inner"),
    ("coroutine-name-outer", "ValueError", "outer"),
    ("coroutine-name-inner", "TypeError", "inner"),
    ("make", "inner-coroutine-call", "TypeError"),
)
print("ok")
