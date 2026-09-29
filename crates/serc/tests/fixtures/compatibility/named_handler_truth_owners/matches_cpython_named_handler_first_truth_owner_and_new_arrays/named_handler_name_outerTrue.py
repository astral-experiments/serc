def f(warn, flag):
    try:
        if exists():
            try:
                if check():
                    remove()
                elif other():
                    unlink()
            except Error as e:
                if warn:
                    from module import Display
                    display = Display()
                    display(e)
    except Error:
        pass
