def finally_then_expression_name_return(manager, condition, value):
    with manager:
        try:
            prepare(value)
        finally:
            cleanup(value)
        observe(value)
        if condition:
            return value
    return None


def finally_then_assignment_not_attribute_raise(manager, condition, value):
    with manager:
        try:
            prepare(value)
        finally:
            cleanup(value)
        saved = observe(value)
        if not value.ready:
            raise RuntimeError(saved)
    return None


def finally_global_call_return(manager, condition, value):
    with manager:
        try:
            prepare(value)
        finally:
            cleanup(value)
        if gcheck(condition):
            return value
    return None


def finally_nested_global_call_return(manager, condition, value, check):
    with manager:
        try:
            prepare(value)
        finally:
            cleanup(value)
        if check(gcheck(condition)):
            return value
    return None


def finally_global_attribute_call_return(manager, condition, value):
    with manager:
        try:
            prepare(value)
        finally:
            cleanup(value)
        if gvalue.check(condition):
            return value
    return None


def finally_named_double_not_return(manager, condition, value, check):
    with manager:
        try:
            prepare(value)
        finally:
            cleanup(value)
        if result := not (not check(condition)):
            return value
    return None


def finally_named_split_not_return(manager, condition, value, check):
    with manager:
        try:
            prepare(value)
        finally:
            cleanup(value)
        if result := not (inner := not check(condition)):
            return value
    return None


def finally_named_split_none_return(manager, condition, value, check):
    with manager:
        try:
            prepare(value)
        finally:
            cleanup(value)
        if result := not (inner := (check(condition) is None)):
            return value
    return None


def finally_deferred_global_call_return(manager, condition, value, check):
    with manager:
        try:
            prepare(value)
        finally:
            cleanup(value)
        if check(lambda: gcheck(condition)):
            return value
    return None


def finally_eager_global_default_return(manager, condition, value, check):
    with manager:
        try:
            prepare(value)
        finally:
            cleanup(value)
        if check(lambda result=gcheck(condition): result):
            return value
    return None
