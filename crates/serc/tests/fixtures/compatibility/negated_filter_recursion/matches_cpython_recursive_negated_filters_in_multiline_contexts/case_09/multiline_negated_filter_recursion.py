result = {
    x
    for x in items
    if (
        not not (first(x) and second(x))
    )
}
