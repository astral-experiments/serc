def f(flag, owned, source, root, work):
    try:
        if flag:
            with work(source) as value:
                root = work(value)
        else:
            try:
                root = work(source)
            except TypeError:
                root = work()
    except TypeError as error:
        if owned is None:
            raise RuntimeError("failed") from error
        raise
    finally:
        namespace.value = source
    return root.value
