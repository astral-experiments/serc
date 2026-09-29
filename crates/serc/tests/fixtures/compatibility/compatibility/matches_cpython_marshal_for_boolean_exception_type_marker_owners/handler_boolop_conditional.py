def f():
    try:
        action()
    except (A if C else B) or D:
        recover()
