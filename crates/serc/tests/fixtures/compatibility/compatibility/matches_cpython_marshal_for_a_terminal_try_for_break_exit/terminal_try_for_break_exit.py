def target():
    try:
        response = 42
        for value in range(100):
            if value == response:
                break
    except ValueError as error:
        if not error.args:
            raise
