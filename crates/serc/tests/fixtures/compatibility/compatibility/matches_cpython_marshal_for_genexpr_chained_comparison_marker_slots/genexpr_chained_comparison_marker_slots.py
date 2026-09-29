def slot_zero_small_int(items):
    return (0 < item < 10 for item in items)

def slot_zero_float(items):
    return (0.0 < item < 10 for item in items)

def slot_one_null_global(items):
    return (lower() < item < upper for item in items)

def slot_one_fused_store(items):
    return (0 < left < right for left, right in items)
