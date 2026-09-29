def with_owner(manager, condition):
    with manager:
        try:
            body()
            assert condition
        finally:
            cleanup()


def finally_owner(condition):
    try:
        try:
            body()
            assert condition, message()
        finally:
            inner_cleanup()
    finally:
        outer_cleanup()


def expression_tail(manager):
    with manager:
        try:
            body()
        finally:
            cleanup()
