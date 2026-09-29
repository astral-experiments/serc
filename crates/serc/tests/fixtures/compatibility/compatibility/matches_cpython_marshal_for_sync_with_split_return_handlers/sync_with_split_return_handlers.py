"""CPython marshal controls for small returns after synchronous context managers."""


def direct_literal(manager, action, Error):
    try:
        with manager:
            action()
        return True
    except Error:
        return False


def direct_none(manager, action, Error):
    try:
        with manager:
            action()
        return None
    except Error:
        return False


def direct_integer(manager, action, Error):
    try:
        with manager:
            action()
        return 1000
    except Error:
        return 0


def direct_string(manager, action, Error):
    try:
        with manager:
            action()
        return "ready"
    except Error:
        return "failed"


def direct_name(manager, action, Error, result):
    try:
        with manager:
            action()
        return result
    except Error:
        return None


def bound_manager(manager, action, Error):
    try:
        with manager as value:
            action(value)
        return True
    except Error:
        return False


def prefixed_try(manager, prepare, action, Error):
    try:
        prepare()
        with manager:
            action()
        return True
    except Error:
        return False


def nested_managers(first, second, action, Error):
    try:
        with first:
            with second:
                action()
        return True
    except Error:
        return False


def multi_item(first, second, action, Error):
    try:
        with first, second:
            action()
        return True
    except Error:
        return False


def named_handler(manager, action, report, Error):
    try:
        with manager:
            action()
        return True
    except Error as error:
        report(error)
        return False


def multiple_handlers(manager, action, First, Second):
    try:
        with manager:
            action()
        return True
    except First:
        return False
    except Second as error:
        action(error)
        return None


def multiline_literal(manager, action, Error):
    try:
        with manager:
            action()
        return (
            True
        )
    except Error:
        return False


def multiline_literal_nested(first, second, action, Error):
    try:
        with first:
            with second:
                action()
        return (
            "ready"
        )
    except Error:
        return "failed"


def multiline_literal_multi_item(first, second, action, First, Second):
    try:
        with first, second:
            action()
        return (
            1000
        )
    except First:
        return 0
    except Second as error:
        action(error)
        return -1


def tuple_return(manager, action, Error):
    try:
        with manager:
            action()
        return (1, 2)
    except Error:
        return ()


def call_return(manager, action, Error, result):
    try:
        with manager:
            action()
        return result()
    except Error:
        return None


def finally_context(manager, action, cleanup, Error):
    try:
        with manager:
            action()
        return True
    except Error:
        return False
    finally:
        cleanup()


def loop_context(managers, action, Error):
    for manager in managers:
        try:
            with manager:
                action()
            return True
        except Error:
            continue
    return False


async def coroutine_context(manager, action, Error):
    try:
        with manager:
            action()
        return (
            True
        )
    except Error:
        return False


def generator_context(manager, action, Error):
    yield None
    try:
        with manager:
            action()
        return (
            True
        )
    except Error:
        return False
