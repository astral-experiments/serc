def f(work, flag, after):
    try:
        work()
    except Exception as error:
        if flag:
            assert work(error), "failed"
    finally:
        after()
