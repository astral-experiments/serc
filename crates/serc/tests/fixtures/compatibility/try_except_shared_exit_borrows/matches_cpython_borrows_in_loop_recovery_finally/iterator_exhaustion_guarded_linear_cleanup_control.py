def collect(groups, job, process, report, update):
    records = []
    for group in groups:
        try:
            selected = process(group)
            for key, value in selected:
                if key in records:
                    report(value)
                else:
                    records.append(key)
                records.append(value)
        except ValueError as error:
            if error.args[0] == "fatal":
                raise
            report(error)
        finally:
            if job is not None:
                update(job, count=len(records), values=list(records))
    return records
