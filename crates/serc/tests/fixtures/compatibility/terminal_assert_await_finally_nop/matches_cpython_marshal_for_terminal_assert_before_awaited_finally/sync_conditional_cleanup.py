def f(running, stop, recover, parent, child):
    try:
        assert not running(child)
    except ValueError:
        recover()
    finally:
        if running(parent):
            stop(parent)
        if running(child):
            stop(child)
    return None
