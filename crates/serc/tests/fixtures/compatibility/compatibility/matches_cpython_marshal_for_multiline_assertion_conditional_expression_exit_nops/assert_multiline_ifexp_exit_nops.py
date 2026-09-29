def direct(condition, first, second):
    assert (
        first == 1 if condition else second == 2
    )


def split(condition, first, second):
    assert (
        first == 1
        if condition
        else second == 2
    )
    consume(first)


def message(condition, first, second):
    assert (
        first == 1
        if condition
        else second == 2
    ), "message"


async def asynchronous(condition, first, second):
    assert (
        first == 1 if condition else second == 2
    )


def try_finally(condition, first, second):
    try:
        assert (
            first == 1 if condition else second == 2
        )
    finally:
        cleanup()


def same_line_control(condition, first, second):
    assert first == 1 if condition else second == 2


def same_start_multiline_control(condition, first, second):
    assert (first == 1
            if condition
            else second == 2)


def ordinary_assert_control(first, second):
    assert (
        first == second
    )


def ordinary_if_control(condition, first, second):
    if (
        first == 1 if condition else second == 2
    ):
        consume(first)
