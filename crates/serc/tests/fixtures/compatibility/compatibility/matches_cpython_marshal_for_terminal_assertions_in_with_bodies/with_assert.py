def simple(manager, condition):
    with manager:
        assert condition

def message(manager, condition):
    with manager:
        use()
        assert condition, make_message()

def boolean(manager, first, second):
    with manager:
        assert first or second

def conjunction(manager, first, second):
    with manager:
        assert first and second

def conditional(manager, first, second, condition):
    with manager:
        assert first if condition else second

def constant_true():
    with manager:
        assert True

def constant_false():
    with manager:
        assert False

def followed(manager, condition):
    with manager:
        assert condition
    use()

def same_line(manager, condition):
    with manager: assert condition

def nested(outer, inner, condition):
    try:
        with outer:
            with inner:
                assert condition
    finally:
        cleanup()

async def asynchronous(manager, condition):
    async with manager:
        assert condition
