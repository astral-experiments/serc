def copy_one(row, base_dest, *, overwrite=False, verbose=False):
    src_uri = normalize_uri((row.get("path") or "").strip())
    if not src_uri:
        return ("", "Missing path")
    dest_uri = make_dest_path(base_dest, row)
    try:
        local_src = asset(src_uri)
        if not overwrite:
            try:
                pass
            except Exception:
                pass
        with open(local_src, "rb") as fp:
            data = fp.read()
        with asset_sink(dest_uri) as local_dest:
            Path(parent_directory(local_dest)).mkdir(parents=True, exist_ok=True)
            with open(local_dest, "wb") as out:
                out.write(data)
        if verbose:
            print(f"Copied -> {dest_uri}")
        return (dest_uri, None)
    except Exception as error:
        return (dest_uri, str(error))
