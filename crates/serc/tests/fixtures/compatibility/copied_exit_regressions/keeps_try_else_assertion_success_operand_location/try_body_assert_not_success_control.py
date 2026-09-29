def target(work, flag, record):
    try:
        work()
        assert not flag
    except ValueError:
        assert flag
    record()
