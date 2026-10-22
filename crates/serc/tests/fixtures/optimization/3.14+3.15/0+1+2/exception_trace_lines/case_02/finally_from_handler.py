def run():
    try:
        work()
    except:
        recover()
    else:
        raise ValueError
    finally:
        7
