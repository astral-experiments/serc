def f(flag, owned, source, root, work, cleanup):
    try:
        try:
            if flag:
                work(source, directory=root)
            else:
                work(root, directory=source)
        except OSError as error:
            raise RuntimeError("failed") from error
    except (RuntimeError, OSError):
        if owned:
            try:
                work(source)
            except OSError:
                pass
        raise
    finally:
        if owned:
            cleanup(source)
        cleanup(root)
