use crate::common;

#[test]
fn conditional_else_finally_borrow_target() {
    let source = r"def run(context, item, commit):
    try:
        prepare(context, item)
        if item.kind == 'first':
            process_first(context, item)
        elif item.kind == 'second':
            process_second(context, item)
        else:
            raise ValueError('unsupported')
    except ValueError as error:
        context.rollback()
        item = recover(context)
        if item is None:
            raise
        item.error = str(error)
    except Exception as error:
        context.rollback()
        item = context.get()
        if item is None:
            raise
        item.error = str(error)
    else:
        failed = count_failed(context)
        item.state = 'partial' if failed else 'complete'
    finally:
        item.finished = timestamp()
        flush(context, commit)
    return item
";
    common::assert_matches_cpython_marshal(source, "conditional_else_finally_borrow_target.py");
}

#[test]
fn conditional_else_finally_borrow_body_tail_control() {
    let source = r"def run(context, item, commit):
    try:
        prepare(context, item)
        if item.kind == 'first':
            process_first(context, item)
        elif item.kind == 'second':
            process_second(context, item)
        else:
            raise ValueError('unsupported')
        failed = count_failed(context)
        item.state = 'partial' if failed else 'complete'
    except ValueError as error:
        context.rollback()
        item = recover(context)
        if item is None:
            raise
        item.error = str(error)
    except Exception as error:
        context.rollback()
        item = context.get()
        if item is None:
            raise
        item.error = str(error)
    finally:
        item.finished = timestamp()
        flush(context, commit)
    return item
";
    common::assert_matches_cpython_marshal(
        source,
        "conditional_else_finally_borrow_body_tail_control.py",
    );
}
