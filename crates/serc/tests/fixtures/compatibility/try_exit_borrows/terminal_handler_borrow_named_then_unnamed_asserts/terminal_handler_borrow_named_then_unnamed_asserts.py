def f(run, values):
    try:
        run()
    except ValueError as error:
        assert False, f'bad: {error}'
    except TimeoutError:
        assert False, 'late'
    found = [value for value in values if value]
    assert found, 'empty'
    return values, found
