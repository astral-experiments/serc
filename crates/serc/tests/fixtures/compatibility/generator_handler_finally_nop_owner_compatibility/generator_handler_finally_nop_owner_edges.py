def explicit_raise_cleanup():
    try:
        yield source
    except Exception as error:
        raise error
    finally:
        cleanup()


def raise_from_cleanup():
    try:
        yield source
    except Exception as error:
        raise error from error.__cause__
    finally:
        cleanup()


def assigned_yield_cleanup():
    try:
        received = yield source
        yield received
    except Exception:
        raise
    finally:
        cleanup()


def handler_yield_cleanup():
    try:
        yield source
    except Exception:
        yield fallback
        raise
    finally:
        cleanup()


def finally_yield_cleanup():
    try:
        yield source
    except Exception:
        raise
    finally:
        yield cleanup
