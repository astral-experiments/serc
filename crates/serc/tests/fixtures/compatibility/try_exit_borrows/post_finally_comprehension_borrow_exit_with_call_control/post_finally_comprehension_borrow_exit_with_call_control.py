def collect(items: list[object], workers: int = 8, kind='primary'):
    rows = []
    pending = [index for index, _ in enumerate(items)]
    try:
        with Pool(workers=workers) as manager:
            jobs = [
                manager.submit(
                    run,
                    index,
                    items[index],
                    kind,
                )
                for index in pending
            ]
            for job in progress.track(completed(jobs), total=len(jobs)):
                try:
                    result = job.result(timeout=10.0)
                except TimeoutError:
                    notify('timeout')
                    result = None
                except Exception as error:
                    notify(f'failed: {error}')
                    result = None
                rows.append(result)
    finally:
        rows = sorted(rows, key=lambda value: value[0])
        rows = [value[1] for value in rows]
        consume(rows)
    selected = rows
    missing = [index for index, value in enumerate(selected) if value is None]
    retained = [value for index, value in enumerate(items) if index not in missing]
    selected = [value for index, value in enumerate(selected) if index not in missing]
    assert len(retained) == len(selected)
    return retained, selected
