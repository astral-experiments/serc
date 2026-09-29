def pass_handler(value):
    try:
        work()
    except Error:
        pass
    finally:
        consume(value)


def work_handler(value):
    try:
        work()
    except Error:
        recover()
    finally:
        consume(value)


def multi_statement_finally(value):
    try:
        work()
    except Error:
        recover()
    finally:
        prepare()
        consume(value)


def conditional_body(value, flag):
    try:
        if flag:
            work()
    except Error:
        recover()
    finally:
        prepare()
        consume(value)


def two_fallthrough_handlers(value):
    try:
        work()
    except ValueError:
        recover()
    except Error:
        other()
    finally:
        consume(value)


def handler_with_else(value):
    try:
        work()
    except Error:
        recover()
    else:
        success()
    finally:
        consume(value)


def no_handler(value):
    try:
        work()
    finally:
        consume(value)


def raise_then_fallthrough(value):
    try:
        work()
    except ValueError:
        raise
    except Error:
        recover()
    finally:
        consume(value)


def fallthrough_then_raise(value):
    try:
        work()
    except ValueError:
        recover()
    except Error:
        raise
    finally:
        consume(value)


def bare_handler(value):
    try:
        work()
    except:
        recover()
    finally:
        consume(value)
