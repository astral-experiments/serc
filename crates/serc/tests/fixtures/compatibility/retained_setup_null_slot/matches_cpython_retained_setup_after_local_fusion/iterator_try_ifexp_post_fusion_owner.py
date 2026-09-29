def target(rows, options):
    for index, row in rows:
        publish(path(options), index, row)
        report(stamp())
        started = clock.time()
        try:
            result = prepare(row, options) if options.prepare else run(row, options)
            consume(result, started)
        except Exception as error:
            recover(error)
