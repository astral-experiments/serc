def contexts(first, second):
    with first() as left:
        consume(left)

    with first() as left, second() as right:
        consume(left, right)

    with first():
        action()
