if path:
    try:
        with ui.progress("loading"):
            data = read_rows(path)
        ui.success(f"Loaded {len(data)} rows from {path}")
        frame = table.make(data)
        ui.show(frame, wide=True, hidden=True)
        if not data:
            ui.warning("empty")
        else:
            size = "?"
            try:
                if path.startswith("store://") and storage is not None:
                    size = storage.stat(path).size
                else:
                    size = filesystem.size(path)
            except Exception:
                pass
            ui.caption(f"Size {size}; rows {len(data)}")
    except Exception as error:
        ui.error(f"Failed: {error}")
