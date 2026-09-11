use crate::common;

#[test]
fn matches_cpython_borrows_after_omitted_for_backedges() {
    for source in [
        r"def probe(x, flag, work, use):
    try:
        value = work(x)
    except Exception:
        value = None
        for item in x:
            if flag:
                value = item
                break
        if value is None:
            raise ValueError()
    use(value)
",
        r"def probe(x, flag, work, use):
    try:
        value = work(x)
    except Exception:
        value = None
        for item in x:
            if flag:
                value = work(item)
        if value is None:
            raise ValueError()
    use(value)
",
        r"def probe(x, flag, work, use):
    try:
        value = work(x)
    except Exception:
        value = None
        for item in x:
            if flag:
                value = item
                break
            work(item)
        if value is None:
            raise ValueError()
    use(value)
",
        r"def probe(x, flag, work, use):
    try:
        value = work(x)
    except Exception:
        value = None
        while x:
            if flag:
                value = x
                break
        if value is None:
            raise ValueError()
    use(value)
",
        r"def probe(x, flag, work, use):
    try:
        value = None
        for item in x:
            if flag:
                value = item
                break
    except Exception:
        value = None
        if value is None:
            raise ValueError()
    use(value)
",
        r"def probe(x, flag, work, use):
    try:
        value = work(x)
    except Exception:
        value = None
        for item in x:
            pass
        if value is None:
            raise ValueError()
    use(value)
",
        r"def probe(x, flag, work, use):
    try:
        value = work(x)
    except Exception:
        value = None
        for item in x:
            value = item
            break
        if value is None:
            raise ValueError()
    use(value)
",
        r"def probe(x, flag, work, use):
    try:
        value = work(x)
    except Exception:
        value = None
        for item in x:
            if flag:
                value = work(item)
            else:
                value = item
    use(value)
",
        r"def probe(x, flag, work, use):
    try:
        value = work(x)
    except Exception:
        value = None
        for item in x:
            if flag is not None:
                value = work(item)
            else:
                value = item
    use(value)
",
        r"def probe(x, flag, work, use, cm):
    try:
        value = work(x)
    except Exception:
        value = None
        for item in x:
            if flag:
                with cm:
                    return item
            else:
                with cm:
                    return item
    use(value)
",
        r"def probe(x, flag, work, use, cm):
    try:
        value = work(x)
    except Exception:
        value = None
        for item in x:
            if flag:
                with cm:
                    return item
            else:
                value = work(item)
    use(value)
",
        r"def probe(x, flag, work, use, cm):
    try:
        value = work(x)
    except Exception:
        value = None
        for item in x:
            if flag:
                value = work(item)
            else:
                with cm:
                    return item
    use(value)
",
        r"def probe(x, flag, work, use, cm):
    try:
        value = work(x)
    except Exception:
        value = None
        for item in x:
            if flag:
                try:
                    return work(item)
                except Exception:
                    return item
            else:
                try:
                    return work(item)
                except Exception:
                    return item
    use(value)
",
        r"def probe(x, flag, work, use, cm):
    try:
        value = work(x)
    except Exception:
        value = None
        for item in x:
            if flag:
                try:
                    return work(item)
                except Exception:
                    return item
            else:
                value = work(item)
    use(value)
",
        r"def probe(x, flag, work, use, cm):
    try:
        value = work(x)
    except Exception:
        value = None
        for item in x:
            if flag:
                value = work(item)
            else:
                try:
                    return work(item)
                except Exception:
                    return item
    use(value)
",
        r"def probe(x, flag, work, use):
    try:
        value = work(x)
    except Exception:
        value = None
        for item in x:
            if flag:
                pass
            else:
                pass
    use(value)
",
        r"def probe(x, flag, work, use):
    try:
        value = work(x)
    except Exception:
        value = None
        for item in x:
            if flag:
                pass
            else:
                value = item
    use(value)
",
        r"def probe(x, flag, work, use):
    try:
        value = work(x)
    except Exception:
        value = None
        for item in x:
            if flag:
                value = work(item)
            else:
                pass
    use(value)
",
    ] {
        common::assert_matches_cpython_marshal(source, "omitted_for_backedge.py");
    }
}
