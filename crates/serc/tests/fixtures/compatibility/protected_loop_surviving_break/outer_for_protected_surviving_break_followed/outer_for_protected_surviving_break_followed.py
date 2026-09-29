def run(readers, value):
    for reader in readers:
        try:
            while True:
                item = reader.read()
                if item is None:
                    break
                if skip(item):
                    continue
                consume(item)
                if finished(item):
                    break
            consume(reader, value)
        finally:
            reader.close()
