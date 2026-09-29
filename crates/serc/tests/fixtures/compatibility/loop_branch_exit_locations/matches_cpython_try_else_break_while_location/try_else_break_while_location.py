def monitor(read, active, accept):
    while active():
        try:
            value = read()
        except Error as error:
            report(error)
        else:
            if not accept(value):
                break
        report("tick")
