def f(run, values):
    try:
        run()
    except ValueError as error:
        raise RuntimeError(f'bad: {error}')
    except TimeoutError:
        raise RuntimeError('late')
    found = [value for value in values if value]
    assert found, 'empty'
    return values, found
