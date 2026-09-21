use crate::common;
#[test]
fn matches_cpython_distinct_and_shared_nan_constants() {
    for source in [
        r"def f(g):
    g(1e309 - 1e309)
    g(1e309 - 1e309)
",
        r"def f():
    return ((1e309 - 1e309), (1e309 - 1e309))
",
        r"def f():
    return ((1e309 - 1e309),) * 2
",
        r"def f():
    return (((1e309 - 1e309),), ((1e309 - 1e309),))
",
        r"def f():
    return (((1e309 - 1e309),),) * 2
",
        r"def f():
    return (1e309 - 1e309)
def g():
    return (1e309 - 1e309)
",
    ] {
        common::assert_matches_cpython_marshal(source, "distinct_and_shared_nan_constants.py");
    }
}

#[test]
fn matches_cpython_nan_container_and_numeric_origins() {
    for source in [
        r"def f():
    return ((1e309j-1e309j), (1e309j-1e309j), ((1e309j-1e309j),)*2)
",
        r"def f():
    return (((1e309-1e309),)*3)[1:]
",
        r"def f():
    return ((1e309-1e309),)*2 + ((1e309-1e309),)*2
",
        r"def f():
    return ((1e309-1e309),)[0]
",
        r"def f(x):
    return x in {(1e309-1e309), (1e309-1e309)}
",
        r"def f(x):
    return x in {((1e309-1e309),), ((1e309-1e309),)}
",
        r"def f(g):
    g(0.25)
    g(1.25)
    g(2.25)
    g(3.25)
    g(4.25)
    g(5.25)
    g(6.25)
    g(7.25)
    g(8.25)
    g(9.25)
    g(10.25)
    g(11.25)
    g(12.25)
    g(13.25)
    g(14.25)
    g(15.25)
    g(1e309-1e309)
    g(1e309-1e309)
",
        r"def f(x=1e309-1e309, y=1e309-1e309):
    return (x,y)
",
        r"def f():
    return ((+(1e309-1e309),)*2, (-(1e309-1e309),)*2)
",
        r"def f():
    return ((+(1e309j-1e309j),)*2, (-(1e309j-1e309j),)*2)
",
        r"def f():
    return ((1e309-1e309)+1, (1e309-1e309)*2, (1e309-1e309)%2, (1e309-1e309)//2)
",
    ] {
        common::assert_matches_cpython_marshal(source, "nan_container_and_numeric_origins.py");
    }
}

#[test]
fn matches_cpython_nan_copy_allocation_events() {
    for source in [
        r"def f(flag, g):
    if flag:
        g()
    else:
        g()
    return 1e309-1e309
",
        r"def f(flag, g):
    if flag:
        g()
    else:
        g()
    return 1e309j-1e309j
",
        r"def f(flag, g):
    if flag:
        g()
    else:
        g()
    return +(1e309-1e309)
",
        r"def f(flag, g):
    if flag:
        g()
    else:
        g()
    return -(1e309-1e309)
",
        r"def f(flag, g):
    if flag:
        g()
    else:
        g()
    return ((1e309-1e309),)
",
        r"def f(flag, g):
    if flag:
        g()
    else:
        g()
    return ((1e309-1e309),)*2
",
        r"def f(flag):
    return 0 if flag else (1e309-1e309)
",
        r"def f(flag):
    return (1e309-1e309) if flag else 0
",
        r"def f(flag):
    return flag or (1e309-1e309)
",
        r"def f(flag, g):
    try:
        return 1e309-1e309
    finally:
        if flag:
            g()
",
        r"def f(cm):
    with cm:
        return 1e309-1e309
",
        r"def f(flag, g):
    try:
        if flag:
            return 1
        return 2
    finally:
        g(1e309-1e309)
",
        r"def f(flag, g):
    try:
        if flag:
            return 1
        return 2
    finally:
        g(((1e309-1e309),)*2)
",
        r"def f(flag, g):
    try:
        if flag:
            return 1
        return 2
    finally:
        g(1e309j-1e309j)
",
        r"def f(flag):
    try:
        if flag:
            return 1
        return 2
    finally:
        def g():
            return 1e309-1e309
",
        r"def f(flag):
    try:
        if flag:
            return 1
        return 2
    finally:
        def g(x=1e309-1e309):
            return x
",
    ] {
        common::assert_matches_cpython_marshal(source, "nan_copy_allocation_events.py");
    }
}
