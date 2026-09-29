async def watch(process, retry):
    while True:
        try:
            if retry:
                continue
            yield process
        except OSError:
            await pause()
        finally:
            if process is not None and process.returncode is None:
                process.terminate()
                try:
                    await process.wait()
                except RuntimeError:
                    process.kill()
                    await process.wait()
