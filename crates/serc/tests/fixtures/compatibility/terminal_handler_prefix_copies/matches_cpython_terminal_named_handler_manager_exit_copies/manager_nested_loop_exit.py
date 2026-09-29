def probe(manager, read, write, recover):
    try:
        with manager():
            with manager():
                with manager():
                    while True:
                        chunk = read()
                        if not len(chunk):
                            break
                        write(chunk)
    except Exception as error:
        if recover(error):
            with manager():
                with manager():
                    with manager():
                        while True:
                            chunk = read()
                            if not len(chunk):
                                break
                            write(chunk)
                write(None)
