def direct(value):
    try:
        action()
    except ValueError:
        recover()
    else:
        assert value
    after()

def message(value):
    try:
        action()
    except ValueError:
        recover()
    else:
        prepare()
        assert value, 'bad'
    after()

def is_none(value):
    try:
        action()
    except ValueError:
        recover()
    else:
        assert value is None
    after()

def is_not_none(value):
    try:
        action()
    except ValueError:
        recover()
    else:
        prepare()
        assert value is not None
    after()

def multiple_handlers(value):
    try:
        action()
    except ValueError:
        recover()
    except TypeError:
        recover_type()
    else:
        assert value
    after()

def terminating_handler(value):
    try:
        action()
    except ValueError:
        return
    else:
        assert value
    after()

def protected(manager, value):
    with manager:
        try:
            action()
        except ValueError:
            recover()
        else:
            assert value
    after()

async def coroutine(value):
    try:
        await action()
    except ValueError:
        await recover()
    else:
        assert value is None
    after()

def terminal(value):
    try:
        action()
    except ValueError:
        recover()
    else:
        assert value

def nonfinal(value):
    try:
        action()
    except ValueError:
        recover()
    else:
        assert value
        consume(value)
    after()

def nested_suite(value, flag):
    try:
        action()
    except ValueError:
        recover()
    else:
        if flag:
            assert value
    after()
