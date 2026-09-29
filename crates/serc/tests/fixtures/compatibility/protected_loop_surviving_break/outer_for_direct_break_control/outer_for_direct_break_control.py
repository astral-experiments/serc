def run(readers, value):
    for reader in readers:
        try:
            while True:
                item = reader.read()
                break
            consume(reader, value)
        finally:
            reader.close()
