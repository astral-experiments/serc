def f(items, path, batch, fail):
    for item in items:
        try:
            work(item)
        except Error as error:
            report(error)
            if fail:
                raise
        finally:
            write_json(path, {"item": item, "batch": batch.get("created") if batch else None, "failed": fail})
