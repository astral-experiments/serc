def outer(futures, errors, outputs, metrics, progress):
    def drain():
        for future in futures:
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
    return drain
