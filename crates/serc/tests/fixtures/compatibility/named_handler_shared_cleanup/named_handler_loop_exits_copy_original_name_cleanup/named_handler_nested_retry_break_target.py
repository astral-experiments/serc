def run(items, retries, work, report):
    for item in items:
        try:
            work(item)
        except LookupError as caught:
            report(item)
            for retry in retries:
                try:
                    work(item)
                    break
                except LookupError as caught:
                    report(retry)
                except Exception as caught:
                    report(item)
                    break
            else:
                report(item)
        report(item)
