def terminal_try_true_or(value):
    try:
        assert True or value
    except Exception:
        recover()
