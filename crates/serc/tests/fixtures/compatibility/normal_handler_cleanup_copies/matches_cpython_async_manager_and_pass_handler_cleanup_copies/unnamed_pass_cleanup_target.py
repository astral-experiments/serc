def handle(flag, owner, work, send):
    try:
        work()
    except ValueError as error:
        if flag:
            try:
                send(str(error))
                owner.closed = True
            except Exception:
                pass
        else:
            send(str(error))
    except Exception as error:
        if flag:
            try:
                send(str(error))
                owner.closed = True
            except Exception:
                pass
        else:
            send(str(error))
