def read_items(stream, process):
    if stream:
        try:
            yield stream
        finally:
            stream.close()
            _, error = process.communicate()
            assert process.returncode == 0, error.decode()
        return
    yield None
