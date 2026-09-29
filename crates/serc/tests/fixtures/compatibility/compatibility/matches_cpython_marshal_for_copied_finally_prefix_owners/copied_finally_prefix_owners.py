def conditional_cleanup(value, enabled):
    try:
        return value
    finally:
        if enabled:
            try:
                observe(value)
            except ValueError:
                pass


def conditional_recovery(value, enabled):
    try:
        return value
    finally:
        if enabled:
            try:
                observe(value)
            except (ValueError, TypeError):
                recover(value)


def prefixed_cleanup(value):
    try:
        return value
    finally:
        observe(value)
        try:
            observe(value)
        except ValueError:
            pass


def prefixed_recovery(value):
    try:
        return value
    finally:
        prepared = prepare(value)
        try:
            observe(prepared)
        except ValueError as error:
            recover(error)


async def coroutine_conditional_cleanup(value, enabled):
    try:
        return value
    finally:
        if enabled:
            try:
                observe(value)
            except ValueError:
                recover(value)


def generator_conditional_cleanup(value, enabled):
    if False:
        yield value
    try:
        return value
    finally:
        if enabled:
            try:
                observe(value)
            except ValueError:
                recover(value)


def direct_try_control(value):
    try:
        return value
    finally:
        try:
            observe(value)
        except ValueError:
            pass


def no_return_control(value, enabled):
    try:
        observe(value)
    finally:
        if enabled:
            try:
                observe(value)
            except ValueError:
                pass
