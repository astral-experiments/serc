def probe(action, first, second):
    try:
        action()
    except Exception as error:
        if first and second:
            raise
        pass
