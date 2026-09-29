def f(run, values):
    try:
        run()
    except ValueError as error:
        assert False, f'bad: {error}'
    except TimeoutError:
        assert False, 'late'
    return values
