def transfer(manager, stream, sink):
    with manager:
        try:
            while chunk := stream.read():
                sink.write(chunk)
        except OSError:
            return
        except Exception:
            try:
                sink.fail()
            except OSError:
                return
            return
        sink.flush()
        sink.finish()
