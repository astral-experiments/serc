def publish(path, data, *, key):
    parent = storage.dirname(path)
    if not storage.exists(parent):
        try:
            storage.makedirs(parent)
        except Exception:
            if not storage.isdir(parent):
                raise
    temporary_path = f"{path}.tmp-{new_id()}"
    try:
        with storage.open(temporary_path, "wb", partial_writes_on_exc=False) as stream:
            stream.write(data)
        try:
            storage.copy(temporary_path, path, overwrite=False)
        except Exception as exc:
            if not storage.exists(path):
                raise
            existing = storage.read_bytes(path)
            if existing != data and not records_match(
                decode(existing, key=key).state,
                decode(data, key=key).state,
            ):
                raise ValueError(
                    f"conflicting records were produced for {path}"
                ) from exc
    finally:
        if storage.exists(temporary_path):
            storage.remove(temporary_path)
