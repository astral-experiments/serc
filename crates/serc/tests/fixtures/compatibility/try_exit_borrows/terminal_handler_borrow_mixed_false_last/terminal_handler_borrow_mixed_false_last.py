def f(run, values):
    try:
        run()
    except ValueError as error:
        values = [error]
    except TimeoutError:
        assert False, 'late'
    found = [value for value in values if value]
    assert found, 'empty'
    return values, found
