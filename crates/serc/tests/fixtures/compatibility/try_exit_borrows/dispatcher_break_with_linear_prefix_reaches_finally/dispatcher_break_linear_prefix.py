def f(sink, value):
    try:
        while True:
            command, data = receive()
            if command == 'work':
                work(data)
            elif command == 'stop':
                reply(value)
                break
            else:
                raise Error(command)
    except (Interrupt, Error):
        report(value)
    finally:
        sink.close()
