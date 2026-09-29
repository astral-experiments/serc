def target(work, flag, other, record):
    try:
        work()
    except ValueError:
        assert flag
    else:
        assert flag or other
    record()
