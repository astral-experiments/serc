use crate::common::assert_matches_cpython_marshal;

#[test]
fn boolean_fusion_tail_preserves_protected_marker_owners() {
    for (filename, source) in [
        (
            "attr_and_prep_3.py",
            r"def f(a, b, obj, auxiliary):
    sink(a, b)
    sink(a, b)
    sink(a, b)
    try:
        value = obj.left and obj.right
        sink(value)
    except ValueError:
        recover()
    return obj
",
        ),
        (
            "attr_and_prep_4.py",
            r"def f(a, b, obj, auxiliary):
    sink(a, b)
    sink(a, b)
    sink(a, b)
    sink(a, b)
    try:
        value = obj.left and obj.right
        sink(value)
    except ValueError:
        recover()
    return obj
",
        ),
        (
            "attr_or_cell_prefix.py",
            r"def f(a, b, obj, auxiliary):
    def capture():
        return auxiliary
    sink(a, b)
    sink(a, b)
    sink(a, b)
    sink(a, b)
    try:
        value = obj.left or obj.right
        sink(value)
    except ValueError:
        recover()
    return obj
",
        ),
        (
            "attr_or_loop_array.py",
            r"def f(a, b, obj, auxiliary):
    while obj.active:
        sink(a, b)
        sink(a, b)
        sink(a, b)
        sink(a, b)
        try:
            value = obj.left or obj.right
            sink(value)
        except ValueError:
            recover()
    return obj
",
        ),
        (
            "attr_or_outer_owner.py",
            r"def f(a, b, obj, auxiliary):
    try:
        sink(a, b)
        sink(a, b)
        sink(a, b)
        sink(a, b)
        try:
            value = obj.left or obj.right
            sink(value)
        except ValueError:
            recover()
    except TypeError:
        outer_recover()
    return obj
",
        ),
        (
            "attr_or_prep_3.py",
            r"def f(a, b, obj, auxiliary):
    sink(a, b)
    sink(a, b)
    sink(a, b)
    try:
        value = obj.left or obj.right
        sink(value)
    except ValueError:
        recover()
    return obj
",
        ),
        (
            "attr_or_prep_4.py",
            r"def f(a, b, obj, auxiliary):
    sink(a, b)
    sink(a, b)
    sink(a, b)
    sink(a, b)
    try:
        value = obj.left or obj.right
        sink(value)
    except ValueError:
        recover()
    return obj
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn boolean_insertions_consume_available_fusion_slots() {
    for (filename, source) in [
        (
            "attr_or_prep_1.py",
            r"def f(a, b, obj, auxiliary):
    sink(a, b)
    try:
        value = obj.left or obj.right
        sink(value)
    except ValueError:
        recover()
    return obj
",
        ),
        (
            "attr_or_prep_2.py",
            r"def f(a, b, obj, auxiliary):
    sink(a, b)
    sink(a, b)
    try:
        value = obj.left or obj.right
        sink(value)
    except ValueError:
        recover()
    return obj
",
        ),
        (
            "attr_or_coroutine.py",
            r"async def f(a, b, obj, auxiliary):
    sink(a, b)
    sink(a, b)
    sink(a, b)
    sink(a, b)
    try:
        value = obj.left or obj.right
        sink(value)
    except ValueError:
        recover()
    return obj
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_named_continue_finally_tails() {
    for (filename, source) in [
        (
            "named_continue_method_join_finally_tail.py",
            r"class Worker:
    async def consume(self):
        while True:
            item = await self.queue.get()
            try:
                if item is None:
                    return
                try:
                    value = await item.run()
                except Exception as error:
                    if not item.future.done():
                        item.future.fail(error)
                    continue
                if not item.future.done():
                    item.future.finish(value)
            finally:
                if item is not None:
                    self.active -= 1
                self.queue.task_done()
",
        ),
        (
            "named_continue_nonterminal_finally_tail.py",
            r"async def consume(queue, work, close):
    while True:
        item = await queue.get()
        try:
            if item is None:
                return
            try:
                value = await work(item)
            except Exception as error:
                if item.ready:
                    item.reject(error)
                continue
            item.accept(value)
        finally:
            if item is not None:
                close(item)
            queue.task_done()
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}
