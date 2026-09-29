def run_job(config, session):
    session.start()
    try:
        wait_ready(session, retry_count=config.retry_count)
        runner = lambda harness, args: run(harness, args, timeout=config.timeout)
        if config.first:
            first(config.harness, first_args(), runner=runner)
        elif config.second:
            second(config.harness, second_args(), runner=runner)
        elif config.third:
            third(config.harness, third_args(), runner=runner)
        else:
            run(config.harness, config.args, runner=runner)
    except Exception as error:
        if config.wait_on_failure:
            report(str(error))
            report("Waiting for interruption")
            try:
                while True:
                    idle()
            except KeyboardInterrupt:
                report("Stopping")
        else:
            raise
    finally:
        if session.started and not config.wait_on_failure:
            session.stop()
    return 0
