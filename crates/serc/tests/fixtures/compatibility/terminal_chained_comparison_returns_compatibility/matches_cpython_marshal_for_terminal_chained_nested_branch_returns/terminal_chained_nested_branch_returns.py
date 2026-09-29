def report(a, b, left, right):
    if len(a) == len(b) == 1:
        if left == right:
            emit(
                'same',
            )
        else:
            if equivalent(left, right):
                emit(
                    'equivalent',
                )
            else:
                emit('different')
                rows = list(diff(left, right))
                if rows:
                    limit = 40
                    for row in rows[:limit]:
                        emit(row)
                    if len(rows) > limit:
                        emit('more')
