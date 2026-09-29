def run(stop, check, prepare, work, notify, recover, finish):
    try:
        while True:
            try:
                try:
                    prepare()
                except ValueError:
                    recover()
                work()
                if stop and check():
                    notify()
                    break
            except Exception:
                recover()
    finally:
        finish()
