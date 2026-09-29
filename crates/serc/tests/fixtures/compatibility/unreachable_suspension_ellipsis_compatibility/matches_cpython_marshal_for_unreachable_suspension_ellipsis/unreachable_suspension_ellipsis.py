import abc


class BaseCommand(abc.ABC):
    @abc.abstractmethod
    async def execute(self, value: object | None = None) -> None:
        raise NotImplementedError
        ...


async def coroutine_ellipsis():
    raise NotImplementedError
    ...


def generator_ellipsis(value):
    yield value
    raise NotImplementedError
    ...


async def async_generator_ellipsis(value):
    yield value
    raise NotImplementedError
    ...


async def nested_coroutine_ellipsis(value):
    if value:
        raise NotImplementedError
        ...


def nested_generator_ellipsis(value):
    yield value
    if value:
        raise NotImplementedError
        ...


async def nested_async_generator_ellipsis(value):
    yield value
    if value:
        raise NotImplementedError
        ...


async def comparison_ellipsis_control(value):
    raise NotImplementedError
    value is ...


async def tuple_ellipsis_control():
    raise NotImplementedError
    (...,)


async def assignment_ellipsis_control():
    raise NotImplementedError
    value = ...


async def call_ellipsis_control(call):
    raise NotImplementedError
    call(...)


async def bare_then_used_ellipsis_control(value):
    raise NotImplementedError
    ...
    value is ...


async def bare_then_tuple_ellipsis_control():
    raise NotImplementedError
    ...
    (...,)


def regular_number_control():
    raise NotImplementedError
    value = 42


async def coroutine_string_control():
    raise NotImplementedError
    value = "unreachable"


def generator_number_control(value):
    yield value
    raise NotImplementedError
    value = 42


async def async_generator_string_control(value):
    yield value
    raise NotImplementedError
    value = "unreachable"
