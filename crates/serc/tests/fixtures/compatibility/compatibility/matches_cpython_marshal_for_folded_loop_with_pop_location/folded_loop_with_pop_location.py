async def async_generator_exception_break(manager, activate_span, read):
    with manager:
        try:
            while True:
                try:
                    with activate_span():
                        chunk = await read()
                except StreamClosedError:
                    break
                if chunk:
                    yield chunk
        except Exception as exc:
            record_exception(exc)
            raise


def synchronous_conditional_break(manager, stop, work):
    with manager:
        try:
            while True:
                if stop():
                    break
                work()
        except Exception:
            raise


def nested_manager_conditional_break(outer, inner, stop, work):
    with outer, inner:
        try:
            while True:
                if stop():
                    break
                work()
        except Exception:
            raise


def runtime_while_condition(manager, running, stop, work):
    with manager:
        try:
            while running():
                if stop():
                    break
                work()
        except Exception:
            raise


def work_after_loop(manager, stop, work, after_loop):
    with manager:
        try:
            while True:
                if stop():
                    break
                work()
            after_loop()
        except Exception:
            raise


def located_pass_after_loop(manager, stop, work):
    with manager:
        try:
            while True:
                if stop():
                    break
                work()
            pass
        except Exception:
            raise


def terminating_loop_else(manager, stop, work):
    with manager:
        try:
            while True:
                if stop():
                    break
                work()
            else:
                return
        except Exception:
            raise
