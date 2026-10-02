use crate::common;

#[test]
fn matches_cpython_retained_setup_after_named_handler_value_return() {
    for asynchronous in [false, true] {
        for nested_calls in ["make(value(arg))", "make(value(extra(arg)))"] {
            let source = format!(
                "{}def f(flag, other, arg):
    try:
        return {}original()
    except Error as error:
        if not flag:
            raise
        effect(0 if flag else 1, first(arg))
        return [
            {nested_calls}
        ]
    finally:
        try:
            if flag:
                assert arg is not None
                {}cleanup(arg)
        finally:
            try:
                if other:
                    assert arg is not None
                    {}other_cleanup(arg)
            finally:
                {}finish()
",
                if asynchronous { "async " } else { "" },
                if asynchronous { "await " } else { "" },
                if asynchronous { "await " } else { "" },
                if asynchronous { "await " } else { "" },
                if asynchronous { "await " } else { "" },
            );
            common::assert_matches_cpython_marshal(
                &source,
                &format!("retained_setup_named_value_{asynchronous}_{nested_calls}.py"),
            );
        }
    }
}

#[test]
fn retained_setup_fusion_slots_account_for_closure_prologues() {
    for prologue in ["", "        super().f(value)\n"] {
        for count in 6..=10 {
            for condition in ["value", "check(value)", "isinstance(value, Kind)"] {
                let stores = "        self.item = value\n".repeat(count);
                let source = format!(
                    "class C(Base):\n    def f(self, value):\n{prologue}{stores}        try:\n            self.value = value if {condition} else convert(value)\n        except ValueError as error:\n            raise Error() from error\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "retained_setup_fusion_prologue.py",
                );
            }
        }
    }
}

#[test]
fn comprehension_cell_assignment_keeps_fresh_setup_marker_owner() {
    for capture in ["    capture = lambda: exceptions\n", ""] {
        for prefix in ["    await work()\n", ""] {
            for comprehension in [
                "[exc for exc in exceptions]",
                "[exc for sub in exceptions for exc in sub]",
                "[exc for outer in exceptions for sub in outer for exc in sub]",
            ] {
                let source = format!(
                    "async def f(host, sock, exceptions, all_errors):
{capture}{prefix}    if host:
        if sock is None:
            exceptions = {comprehension}
            try:
                if all_errors:
                    raise Group(exceptions)
                raise Error(exceptions)
            finally:
                exceptions = None
    else:
        consume(sock)
    await finish()
"
                );
                common::assert_matches_cpython_marshal(&source, "comprehension_cell_setup.py");
            }
        }
    }
}
