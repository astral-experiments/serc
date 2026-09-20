def cleanup_under_manager(values, manager, cleanup, ready):
    with manager:
        for value in values:
            try:
                return value
            finally:
                if ready():
                    try:
                        cleanup()
                    except Exception:
                        pass


async def async_cleanup_under_manager(values, manager, cleanup, ready):
    async with manager:
        for value in values:
            try:
                return value
            finally:
                if ready():
                    try:
                        await cleanup()
                    except Exception:
                        pass


def cleanup_without_manager(values, cleanup, ready):
    for value in values:
        try:
            return value
        finally:
            if ready():
                try:
                    cleanup()
                except Exception:
                    pass


def literal_return_before_outer_finalizer(values, cleanup, ready, outer_cleanup):
    try:
        for value in values:
            try:
                return None
            finally:
                if ready():
                    try:
                        cleanup()
                    except Exception:
                        pass
    finally:
        outer_cleanup()


def literal_return_before_guarded_outer_finalizer(values, cleanup, ready, outer_cleanup):
    try:
        for value in values:
            try:
                return None
            finally:
                if ready():
                    try:
                        cleanup()
                    except Exception:
                        pass
    finally:
        try:
            outer_cleanup()
        except Exception:
            pass
