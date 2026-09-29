def run(flag):
    try:
        work()
    finally:
        try:
            if flag:
                close()
        except Exception:
            pass
