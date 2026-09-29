def f(flag, owned, source, root, work, cleanup, values):
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
        write_json(source, {"items": [item.value for item in values]})
        cleanup(root)
