def copy_retry(run, report, remove, atomic):
    while True:
        try:
            run()
        except TimeoutError as error:
            report(error)
            continue
        else:
            break
        finally:
            if atomic:
                remove()
