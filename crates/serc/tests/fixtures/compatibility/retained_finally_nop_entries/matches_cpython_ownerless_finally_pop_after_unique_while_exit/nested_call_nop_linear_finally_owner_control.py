def f(locales, jobs, errors, outputs, metrics, progress, executor):
    for locale in locales:
        pending = {}
        def drain(block=False):
            for future in list(pending):
                try:
                    value, metric = future.result()
                except Exception as exc:
                    errors.append(exc)
                else:
                    if value is None:
                        errors.append(value)
                    elif value:
                        outputs.append(value)
                        if metric is not None:
                            metrics.append(metric)
                finally:
                    progress.update(1)
                    del pending[future]
        try:
            for job in jobs:
                future = executor.submit(work, job)
                pending[future] = True
                drain(block=False)
            drain(block=True)
        except KeyboardInterrupt:
            for future in pending:
                future.cancel()
            try:
                executor.shutdown(wait=False, cancel_futures=True)
            except TypeError:
                executor.shutdown(wait=False)
            progress.close()
            return
        finally:
            try:
                executor.shutdown(wait=True)
            except Exception:
                pass
            try:
                progress.close()
            except Exception:
                pass
        outputs.append(locale)
