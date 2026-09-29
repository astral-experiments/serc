def return_from_manager(flag, manager, pause):
    while flag():
        try:
            with manager():
                return None
        except OSError:
            pause()
    raise RuntimeError


def effect_from_manager(flag, manager, pause, touch):
    while flag():
        try:
            with manager():
                touch()
        except OSError:
            pause()
    raise RuntimeError


def nested_managers(flag, first, second, pause, touch):
    while flag():
        try:
            with first():
                with second():
                    touch()
        except OSError:
            pause()
    return None


def conditional_try(flag, enabled, manager, pause, touch):
    while flag():
        if enabled:
            try:
                with manager():
                    touch()
            except OSError:
                pause()
    return None


def leading_effect(flag, manager, pause, touch):
    while flag():
        try:
            touch()
            with manager():
                touch()
        except OSError:
            pause()
    return None


def true_loop_with_finally(first, second, pause, touch):
    while True:
        try:
            with first():
                with second():
                    raise RuntimeError
        except OSError:
            pause()
        finally:
            touch()


def no_manager_control(flag, pause):
    while flag():
        try:
            return None
        except OSError:
            pause()
    return None


def outside_loop_control(manager, pause, touch):
    try:
        with manager():
            touch()
    except OSError:
        pause()


async def coroutine_control(flag, manager, pause, touch):
    while flag():
        try:
            with manager():
                touch()
        except OSError:
            pause()
        await pause()


def generator_control(flag, manager, pause, touch):
    while flag():
        try:
            with manager():
                touch()
        except OSError:
            pause()
        yield None
