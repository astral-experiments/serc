def configure():
    global check
    check = predicate
result = [check(value) for value in values]
after = check()
