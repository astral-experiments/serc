def run(recorder, worker, sample, index, record_raw):
    with recorder.open():
        if record_raw:
            try:
                recorder.record(sample)
            except Exception as error:
                if worker.ignore:
                    recorder.report(error)
                else:
                    raise
        seed = worker.seed(index)
        try:
            result = worker.run(sample, seed)
        except Exception as error:
            partial = getattr(error, "partial", None)
            if partial is not None:
                try:
                    recorder.record(partial)
                except Exception:
                    recorder.report(sample)
            recorder.report(error)
            if not worker.accepts(error):
                recorder.report(sample)
                raise
            result = None
            recorder.report(error)
        recorder.completed(index)
        return index, result
