def target(options, path, value):
    try:
        if options.skip:
            return 0
        result = run(value, mode=options.mode, enabled=options.enabled)
        print(f"output={result.output}")
        print(f"details={result.details}")
        print(f"report={result.report}")
        print(f"url={result.url}")
        print(f"manifest={result.manifest}")
        print(f"count={result.count}")
        print(f"average={result.average}")
        return 0
    finally:
        if not options.keep:
            try:
                path.unlink(missing_ok=True)
                path.parent.rmdir()
            except OSError:
                pass
