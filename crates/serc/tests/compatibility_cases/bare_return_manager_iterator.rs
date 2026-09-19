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

#[test]
fn matches_cpython_bare_return_interleaved_manager_iterators() {
    for (filename, source) in [
        (
            "bare_return_interleaved_sync_manager_iterators.py",
            r"def target(groups, manager, stop):
    try:
        for group in groups:
            with manager(group) as values:
                for value in values:
                    consume(value)
                    if stop(value):
                        return
    finally:
        finish()
",
        ),
        (
            "generator_return_interleaved_manager_outer_try.py",
            r"def target(groups, retries, manager, stop):
    try:
        for group in groups:
            for retry in retries:
                try:
                    with manager(group) as values:
                        for value in values:
                            yield value
                            if stop(value):
                                return
                    break
                except Exception:
                    recover(retry)
    finally:
        finish()
",
        ),
        (
            "bare_return_manager_inside_all_iterators_control.py",
            r"def target(groups, manager, stop):
    try:
        for group in groups:
            for value in group:
                with manager(value):
                    if stop(value):
                        return
    finally:
        finish()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_async_iterator_none_finally_return_boundaries() {
    for (filename, source) in [
        (
            "bare_async_iterator_return_local_none_pop_pair_target.py",
            r"async def run(stream, stop, enqueue):
    pending = None
    try:
        async for message in stream:
            if stop(message):
                return
            if pending is not None:
                await enqueue(pending)
            pending = message
    except ValueError:
        pass
    finally:
        if pending is not None:
            await enqueue(pending)
        await enqueue(None)
",
        ),
        (
            "value_async_iterator_return_local_none_pop_pair_control.py",
            r"async def run(stream, stop, enqueue):
    pending = None
    try:
        async for message in stream:
            if stop(message):
                return pending
            if pending is not None:
                await enqueue(pending)
            pending = message
    except ValueError:
        pass
    finally:
        if pending is not None:
            await enqueue(pending)
        await enqueue(None)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
