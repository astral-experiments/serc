def replace_selected(records, edit):
    for record in records:
        try:
            rows = record.rows
            if isinstance(rows, list):
                selected = None
                for index, row in enumerate(rows):
                    if isinstance(row, dict) and row.get("kind") == "entry" and row.get("key") == "primary":
                        selected = index
                        break
                if selected is None:
                    for index, row in enumerate(rows):
                        if isinstance(row, dict) and row.get("kind") == "entry":
                            selected = index
                            break
                if selected is not None:
                    old = rows[selected].get("value")
                    if isinstance(old, str) and old:
                        rows[selected]["value"] = edit(old)
                    else:
                        rows[selected]["value"] = "default"
                    save(record, rows)
        except Exception:
            recover(record)
    complete()
