def crasher(timeout: float | None, name: str | None = None):
    if timeout is None:
        yield
        return
    create_time = psutil.Process(os.getpid()).create_time()
    log_file = os.path.join(os.environ["HOME"], ".crasher_logs", f"{os.getpid()}.log")
    kill_eventually = True

    os.makedirs(os.path.dirname(log_file), exist_ok=True)
    with open(log_file, "w") as f:
        process = subprocess.Popen(
            [
                "python",
                "-m",
                __loader__.name,
                str(time.time() + timeout),
                str(os.getpid()),
                str(create_time),
            ],
            stdin=subprocess.DEVNULL,
            stdout=f,
            stderr=subprocess.STDOUT,
        )
        try:
            yield
        except KeyboardInterrupt:
            # don't kill in case we hang still
            label = f": {name}" if name is not None else ""
            logger.info(f"Likely interrupted by crasher{label} (timeout: {timeout} seconds)")
            kill_eventually = False
            raise
        finally:
            if kill_eventually:
                process.kill()
                process.wait()
