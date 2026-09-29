def f(work, fail, after):
    try:
        work()
    except Exception as error:
        if not fail:
            assert False, f"unexpected: {error}"
    else:
        work()
    after()
