def protected_by_target_store_fusion(items):
    return [
        (left, right)
        for left, right in items
        if any(value for value in right)
    ]
