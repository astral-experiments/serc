def probe(action, first):
    try:
        action()
    except Exception as error:
        if first:
            raise
