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
                    try:
                        notify()
                        break
                    finally:
                        recover()
            except Exception:
                recover()
    finally:
        finish()
