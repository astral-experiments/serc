def f(load, make_frame):
    try:
        return load()
    except NameError:
        frame = make_frame()
        try:
            return load(frame)
        except Exception:
            return {}
        finally:
            del frame
