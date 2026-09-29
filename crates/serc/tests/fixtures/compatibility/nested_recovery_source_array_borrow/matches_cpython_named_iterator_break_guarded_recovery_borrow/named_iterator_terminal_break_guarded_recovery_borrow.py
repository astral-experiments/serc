def run(items, options, work, log, cleanup):
    failures = []
    for item in items:
        launched = False
        started = False
        finished = False
        try:
            work(item)
            launched = True
            started = True
            work(item)
            finished = True
        except (ValueError, RuntimeError) as error:
            if started and not finished:
                log("incomplete", item)
            elif not launched:
                log("unstarted", item)
            else:
                log("failed", item)
            failures.append((item, str(error)))
            log(error)
            if not options.keep_going:
                break
        finally:
            if launched and not options.keep_item:
                try:
                    cleanup(["delete", item.name])
                except (ValueError, RuntimeError) as error:
                    failures.append((item, str(error)))
                    log(error)
    return failures
