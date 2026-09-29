def target(work, flag, record):
    try:
        work()
    except ValueError:
        assert flag
    else:
        assert not flag
    record()
