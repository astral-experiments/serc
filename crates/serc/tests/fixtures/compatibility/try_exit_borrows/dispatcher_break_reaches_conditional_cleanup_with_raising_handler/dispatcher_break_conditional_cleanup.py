def f(sink, value):
    try:
        setup(value)
        while True:
            command = receive()
            if command == 'work':
                work(value)
            elif command == 'stop':
                break
            elif command == 'read' and ready(value):
                reply(value)
    except Error as error:
        report(error)
        raise error
    finally:
        if sink is not None:
            sink.close()
        reply(value)
