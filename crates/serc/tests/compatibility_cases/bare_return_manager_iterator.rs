use crate::common;

fn source(value: &str) -> String {
    format!(
        r"def collect(queue):
    found = False
    while not found:
        try:
            path = queue.get_nowait()
            started(path)
        except Empty:
            exhausted()
            break
        try:
            with open_stream(path) as stream:
                for line in stream:
                    try:
                        item = parse(line)
                        if matches(item):
                            publish(item)
                            found = True
                            found_item(item)
                            return{value}
                    except Exception as error:
                        rejected(error)
        except Exception as error:
            failed(error)
        finally:
            queue.task_done()
"
    )
}

#[test]
fn bare_return_retires_inner_try_before_manager_iterator() {
    common::assert_matches_cpython_marshal(
        &source(""),
        "bare_return_retires_inner_try_before_manager_iterator.py",
    );
}

#[test]
fn explicit_none_retires_inner_try_before_manager_iterator_control() {
    common::assert_matches_cpython_marshal(
        &source(" None"),
        "explicit_none_retires_inner_try_before_manager_iterator_control.py",
    );
}
