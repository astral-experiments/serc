def discard_before_loop_exit():
    while running:
        try:
            break
        except:
            pass
    else:
        1 if 1 else 1


def discard_before_loop_backedge(x):
    while x:
        0 if 1 else 0


def nested_conditional_discard():
    a if (1 if b else c) else d


def negated_chained_loop(a, b, c):
    while not (a < b < c):
        pass


def copied_handler_backedges(items, a, b):
    for x in items:
        try:
            use(x)
        except OSError:
            try:
                use(x)
            except OSError:
                try:
                    if a:
                        if b:
                            use(x)
                except OSError:
                    pass
    return 42


async def erased_match_tail(x, v):
    match x:
        case True:
            pass
        case items if True:
            {k: v async for k in items}
        case []:
            pass
    [[x]]
