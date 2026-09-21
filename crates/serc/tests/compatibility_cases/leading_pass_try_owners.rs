use crate::common;

#[test]
fn matches_cpython_leading_pass_try_owners() {
    for source in [
        r"def f(x,g):
    a = True
    try:
        pass
        a = g() if x else g()
    except Exception:
        a = False
    return a
",
        r"def f(x,g):
    a=True
    try:
        pass
        a=g() if x else g()
    finally:
        g()
    return a
",
        r"def f(x,g):
    try:
        pass
        a=g() if x else g()
    except Exception:
        a=False
    else:
        g()
    return a
",
        r"def f(x,g):
    try:
        pass
        pass
        a=g() if x else g()
    except Exception:
        a=False
    return a
",
        r"def f(x,g):
    try:
        pass
        if x:
            g()
    except Exception:
        g()
",
        r"def f(x,g):
    try:
        g()
    except Exception:
        try:
            pass
            if x:
                return g()
        except RuntimeError:
            return g()
",
    ] {
        common::assert_matches_cpython_marshal(source, "leading_pass_try_owners.py");
    }
}

#[test]
fn matches_cpython_leading_pass_copy_and_compaction_controls() {
    for source in [
        r"def f(x,g):
    with x:
        try:
            pass
            a=g() if x else g()
        except Exception:
            a=False
    return a
",
        r"def f(x,g):
    with x:
        try:
            pass
            a=g() if x else g()
        finally:
            g()
    return a
",
        r"async def f(x,g):
    async with x:
        try:
            pass
            a=g() if x else g()
        except Exception:
            a=False
    return a
",
        r"def f(x,g):
    while x:
        try:
            pass
            if g():
                g()
        except Exception:
            g()
",
        r"def f(x,g):
    try:
        pass; a=g() if x else g()
    except Exception:
        a=False
    return a
",
        r"def f(x,g):
    a=2+3
    try:
        pass
        a=g() if x else g()
    except Exception:
        a=False
    return a
",
        r"def f(x,g):
    try:
        1; pass
        a=g() if x else g()
    except Exception:
        a=False
    return a
",
        r"def f(x,g):
    try:
        pass
        yield g() if x else g()
    finally:
        g()
",
    ] {
        common::assert_matches_cpython_marshal(source, "leading_pass_try_owners.py");
    }
}
