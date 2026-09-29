def read_info(path, index, *, config):
    for attempt in range(1, 4):
        try:
            resource = open_resource(path, config=config)
            try:
                return Info(
                    path=path,
                    index=index,
                    count=int(resource.metadata.count),
                )
            finally:
                resource.reader.close()
        except Exception:
            if attempt == 3:
                raise
            wait(float(2 ** (attempt - 1)))
    raise AssertionError("unreachable")
