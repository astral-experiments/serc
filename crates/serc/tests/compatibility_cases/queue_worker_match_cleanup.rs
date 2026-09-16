use crate::common;

#[test]
fn queue_worker_match_cleanup_target() {
    let source = r"def make_worker(queue):
    failed = None

    def worker() -> None:
        nonlocal failed
        while True:
            signal = queue.get()
            try:
                match signal:
                    case Work(value=value):
                        if failed is not None:
                            continue
                        consume(value)
                    case Stop():
                        break
            except Exception as error:
                failed = error
            finally:
                queue.task_done()

    return worker
";
    common::assert_matches_cpython_marshal(source, "queue_worker_match_cleanup_target.py");
}

#[test]
fn queue_worker_match_cleanup_wildcard_control() {
    let source = r"def make_worker(queue):
    failed = None

    def worker() -> None:
        nonlocal failed
        while True:
            signal = queue.get()
            try:
                match signal:
                    case Work(value=value):
                        if failed is not None:
                            continue
                        consume(value)
                    case _:
                        break
            except Exception as error:
                failed = error
            finally:
                queue.task_done()

    return worker
";
    common::assert_matches_cpython_marshal(
        source,
        "queue_worker_match_cleanup_wildcard_control.py",
    );
}
