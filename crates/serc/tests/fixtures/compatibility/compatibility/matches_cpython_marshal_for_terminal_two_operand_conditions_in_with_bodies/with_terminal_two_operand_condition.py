def terminal(manager, first, second):
    with manager:
        value = make()
        if not first or not second:
            raise Error(value)


def negated_conjunction(z, msgform):
    with np.errstate(invalid='ignore'):
        if not (np.isnan(z.real) and np.isinf(z.imag)):
            raise AssertionError(msgform % (z.real, z.imag))


def nonterminal(manager, first, second):
    with manager:
        if not first or not second:
            raise Error
        consume()


def fallthrough_body(manager, first, second):
    with manager:
        if not first or not second:
            consume()


def outside_with(first, second):
    if not first or not second:
        raise Error


async def asynchronous(manager, first, second):
    async with manager:
        if not first or not second:
            raise Error
