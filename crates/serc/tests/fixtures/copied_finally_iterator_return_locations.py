def return_after_cleanup(items, value, cleanup):
    try:
        for item in items:
            try:
                return value
            finally:
                cleanup()
    except Exception:
        return None


def return_from_nested_loop(items, other_items, value, cleanup):
    try:
        for item in items:
            try:
                for other in other_items:
                    if other:
                        return value
            finally:
                cleanup()
    except Exception:
        return None


async def async_return_after_cleanup(items, value, cleanup):
    try:
        for item in items:
            try:
                return value
            except Exception:
                raise
            finally:
                cleanup.close()
    except Exception:
        return None


def no_outer_handler_control(items, value, cleanup):
    for item in items:
        try:
            return value
        finally:
            cleanup()
