def probe(options):
    try:
        if options.mode == "other":
            consume(options)
        elif options.mode == "run":
            EngineError = RuntimeError
            try:
                if options.token and options.fresh:
                    raise ValueError("conflicting options")
                consume(options.token)
            except EngineError as error:
                raise ValueError(str(error)) from error
    except RuntimeError as error:
        raise ValueError(str(error)) from error
