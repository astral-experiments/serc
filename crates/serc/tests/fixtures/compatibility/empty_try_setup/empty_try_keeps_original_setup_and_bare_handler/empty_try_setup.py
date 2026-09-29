def f(items):
    for item in items:
        try: operation()
        except NameError: pass
        except ZeroDivisionError: pass
        except TypeError: pass
        try: pass
        except: pass
        try: pass
        finally: pass
        consume(item)
