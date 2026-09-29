def f(manager, flag):
    try:
        with manager:
            first()
            second()
            third()
            flag = True
            return
    except Exception:
        pass
    finally:
        if not flag:
            finish()
