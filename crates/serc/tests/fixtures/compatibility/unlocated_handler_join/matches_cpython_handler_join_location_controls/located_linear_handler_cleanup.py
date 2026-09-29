def run(work, recover, finish):
    try:
        work()
    except Exception:
        recover()
    finish()
