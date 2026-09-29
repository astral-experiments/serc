def f(manager, path, marker, pairs, source, catalog, snapshot_id):
    with manager:
        complete = False
        try:
            for key, value in pairs:
                save(key, value)
            save(path / "source.json", source)
            save(path / "catalog.json", catalog)
            temporary = path / f".CURRENT.{identity.new().hex}.tmp"
            write(temporary, f"{snapshot_id}\n".encode("ascii"))
            temporary.replace(path / "CURRENT")
            sync(path)
            complete = True
        finally:
            if complete:
                marker.unlink(missing_ok=True)
                sync(path)
        unlock(manager)
