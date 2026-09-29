def direct_nonterminal(values):
    try:
        result = tuple(int(value) for value in values)
        if result:
            consume(result)
    except ValueError:
        recover()
