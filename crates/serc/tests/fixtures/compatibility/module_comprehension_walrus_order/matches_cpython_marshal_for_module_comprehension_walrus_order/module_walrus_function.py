def f():
    return {(bound := outer): [bound for inner in values] for outer in values}
