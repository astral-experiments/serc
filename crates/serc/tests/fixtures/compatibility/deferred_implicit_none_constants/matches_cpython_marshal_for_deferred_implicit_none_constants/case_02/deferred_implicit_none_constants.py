def f(flag):
    call(first=1)
    try:
        while True:
            if flag:
                break
            call(last=3)
            if flag is None:
                call()
    except Error:
        call()
