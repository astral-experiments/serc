def target(holder, enabled, choose, left, right):
    if enabled:
        fallback()
    else:
        holder.value = left if choose else right
