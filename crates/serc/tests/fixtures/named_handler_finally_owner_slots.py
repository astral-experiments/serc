def conditional_call_return(value, flag, guard, values, manager, state):
    try:
        try:
            action(value)
        except Error as exc:
            if flag:
                return make_result(value)
            raise
    finally:
        if prior is not None:
            cleanup(prior)
    return value


def nested_cleanup_return(value, flag, guard, values, manager, state):
    try:
        try:
            action(value)
        except Error as exc:
            if flag:
                return make_result(value)
            raise
    finally:
        if value is not None:
            try:
                cleanup(value)
            except CleanupError:
                pass
    return value


async def async_conditional_call_return(value, flag, guard, values, manager, state):
    try:
        try:
            await action(value)
        except Error as exc:
            if flag:
                return make_result(value)
            raise
    finally:
        if prior is not None:
            cleanup(prior)
    return value


async def async_nested_cleanup_return(value, flag, guard, values, manager, state):
    try:
        try:
            await action(value)
        except Error as exc:
            if flag:
                return make_result(value)
            raise
    finally:
        if value is not None:
            try:
                cleanup(value)
            except CleanupError:
                pass
    return value


def unconditional_call_return(value, flag, guard, values, manager, state):
    try:
        try:
            action(value)
        except Error as exc:
            return make_result(value)
    finally:
        if prior is not None:
            cleanup(prior)
    return value


async def async_unconditional_call_return(value, flag, guard, values, manager, state):
    try:
        try:
            await action(value)
        except Error as exc:
            return make_result(value)
    finally:
        if prior is not None:
            cleanup(prior)
    return value
