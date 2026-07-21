def single_with_is_none(manager, action, value):
    try:
        with manager():
            action()
        if value is None:
            action()
        action()
    except RuntimeError:
        pass


def single_with_is_not_none(manager, action, value):
    try:
        with manager():
            action()
        if value is not None:
            action()
        action()
    except RuntimeError:
        pass


def pass_with_attribute_is_none(manager, action, source):
    try:
        with manager():
            pass
        if source.value is None:
            action()
        action()
    except RuntimeError:
        pass


def assignment_with_attribute_is_not_none(manager, action, source):
    try:
        with manager():
            current = source.value
        if source.value is not None:
            action(current)
        action()
    except RuntimeError:
        pass


def multi_item_with_call_is_none(manager, action):
    try:
        with manager(), manager():
            action()
        if action() is None:
            action()
        action()
    except RuntimeError:
        pass


def nested_with_named_is_none(manager, action, source):
    try:
        with manager():
            with manager():
                action()
        if (current := source.value) is None:
            action(current)
        action()
    except RuntimeError:
        pass


def assignment_between_with_and_condition(manager, action, source, value):
    try:
        with manager():
            action()
        current = source.value
        if value is None:
            action(current)
        action()
    except RuntimeError:
        pass


def named_handler_with_return(manager, action, value):
    try:
        with manager():
            action()
        if value is None:
            action()
        return
    except RuntimeError as error:
        action(error)


def generic_name_control(manager, action, value):
    try:
        with manager():
            action()
        if value:
            action()
        action()
    except RuntimeError:
        pass


def global_call_control(manager, action, value):
    try:
        with manager():
            action()
        if bool(value):
            action()
        action()
    except RuntimeError:
        pass


def ordinary_comparison_control(manager, action, value):
    try:
        with manager():
            action()
        if value == 0:
            action()
        action()
    except RuntimeError:
        pass


def generator_control(manager, action, value):
    try:
        with manager():
            action()
        if value is None:
            action()
        action()
    except RuntimeError:
        pass
    yield value


async def coroutine_control(manager, action, value):
    try:
        with manager():
            action()
        if value is None:
            action()
        await action()
    except RuntimeError:
        pass


async def async_with_control(manager, action, value):
    try:
        async with manager():
            action()
        if value is None:
            action()
        action()
    except RuntimeError:
        pass


async def async_generator_control(manager, action, value):
    try:
        with manager():
            action()
        if value is None:
            action()
        await action()
    except RuntimeError:
        pass
    yield value


def plain_try_control(action, value):
    try:
        action()
        if value is None:
            action()
        action()
    except RuntimeError:
        pass
