def branch_join(flag):
    if flag:
        if not predicate():
            return False
    else:
        recover()
    return True


def terminal_assert(flag):
    if flag:
        assert predicate()
    else:
        assert not predicate()


def handler_exit():
    try:
        action()
    except Error as error:
        if not predicate():
            recover()


def finally_exit():
    try:
        action()
    finally:
        if not predicate():
            recover()


def loop_tail():
    while outer():
        if not predicate():
            work()
