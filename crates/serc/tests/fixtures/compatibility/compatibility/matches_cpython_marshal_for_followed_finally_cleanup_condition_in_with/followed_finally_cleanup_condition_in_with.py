def f(cm, cond, max_workers):
    with cm() as manager:
        try:
            futures = []
            for arg in all_args:
                futures.append(manager.submit(*arg))
        except RuntimeError as e:
            import pytest
            pytest.skip(f"Spawning failed with error {e!r}")
        finally:
            if len(futures) < max_workers and cond:
                cleanup()
        for future in futures:
            future.result()
