def f(sink, value, manager):
    try:
        while True:
            command = receive()
            if command == 'work':
                work(value)
            elif command == 'stop':
                break
            elif command == 'write':
                with manager:
                    write(value)
    except Interrupt:
        pass
    except Error as error:
        report(error)
    finally:
        sink.close()
