def dispatch(read):
    while True:
        command = read()
        if command == "stop":
            break
        if command == "skip":
            visit()
        elif command == "write":
            result = prepare()
            if result:
                first, second = result
                publish(first, second)
        elif command == "later":
            finish()
        else:
            unknown()
