use crate::common;

#[test]
fn original_pop_keeps_sole_fallthrough_location() {
    for (filename, source) in [
        (
            "pop_three.py",
            r"def f(flag, left, right):
    try:
        if flag:
            value = left
        else:
            value = right
    except ValueError:
        return 0
    else:
        return value
",
        ),
        (
            "pop_four.py",
            r"def f(flag, left, right):
    try:
        if flag:
            value = left
        else:
            value = right
    except ValueError:
        return 0
    else:
        return -value
",
        ),
        (
            "pop_three_arms.py",
            r"def f(flag, left, right):
    try:
        if flag:
            value = left
        elif left:
            value = 2
        else:
            value = right
    except ValueError:
        return 0
    else:
        return value
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn original_pop_respects_native_copy_budget() {
    for (filename, source) in [
        (
            "pop_five.py",
            r"def f(flag, left, right):
    try:
        if flag:
            value = left
        else:
            value = right
    except ValueError:
        return 0
    else:
        return value + 1
",
        ),
        (
            "pop_global_call.py",
            r"def f(flag, left, right):
    try:
        if flag:
            value = left
        else:
            value = right
    except ValueError:
        return 0
    else:
        return global_call()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn original_jump_keeps_return_guard_location() {
    common::assert_matches_cpython_marshal(
        r#"class Doc:
    def document(self, object, name=None, *args):
        """Generate documentation for an object."""
        args = (object, name) + args
        # 'try' clause is to attempt to handle the possibility that inspect
        # identifies something in a way that pydoc itself has issues handling;
        # think 'super' and how it is a descriptor (which raises the exception
        # by lacking a __name__ attribute) and an instance.
        try:
            if inspect.ismodule(object): return self.docmodule(*args)
            if inspect.isclass(object): return self.docclass(*args)
            if inspect.isroutine(object): return self.docroutine(*args)
        except AttributeError:
            pass
        if inspect.isdatadescriptor(object): return self.docdata(*args)
        return self.docother(*args)
"#,
        "jump_document_shape.py",
    );
}

#[test]
fn original_exit_keeps_shared_and_outer_owner_controls() {
    for (filename, source) in [
        (
            "jump_shared_bool.py",
            r"def f(flag, call, args):
    try:
        if flag and args:
            return call(*args)
    except AttributeError:
        pass
    if call:
        return call(args)
    return None
",
        ),
        (
            "jump_outer_handler.py",
            r"def f(flag, call, args):
    try:
        try:
            if flag:
                return call(*args)
        except AttributeError:
            pass
        if call:
            return call(args)
        return None
    except Exception:
        report()
",
        ),
        (
            "pop_shared.py",
            r"def f(flag, left, right):
    value = right
    try:
        if flag:
            value = left
    except ValueError:
        return 0
    else:
        return value
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
