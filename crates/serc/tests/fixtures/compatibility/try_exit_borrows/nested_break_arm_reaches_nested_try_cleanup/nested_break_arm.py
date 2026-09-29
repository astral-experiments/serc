def f(sink):
    try:
        while True:
            item = receive()
            if item is not None:
                result = int(item)
                if result != 0:
                    report(result)
                break
            advance(item)
    except Error:
        report_error()
    finally:
        try:
            sink.close()
        except Timeout:
            pass
