def f(manager, path, marker, pairs, source, catalog, snapshot_id,
      extra0, extra1, extra2, extra3, extra4, extra5, extra6, extra7, extra8,
      extra9, extra10, extra11, extra12, extra13, extra14, extra15, extra16):
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
