with manager() as sink:
    for key, value in items:
        if key.ready:
            first(value)
            sink.write(key, value)
        else:
            second(value)
            sink.write(key, value)
