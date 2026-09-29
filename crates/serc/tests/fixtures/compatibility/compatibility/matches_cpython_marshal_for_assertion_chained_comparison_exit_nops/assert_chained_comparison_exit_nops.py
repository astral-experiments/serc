def direct_chain(first, middle, last):
    assert first.value < middle.value < last.value


def long_chain(first, second, third, fourth):
    assert first.value == second.value == third.value == fourth.value


def chain_with_message(first, middle, last):
    assert first.value <= middle.value <= last.value, "out of order"


def simple_comparison(first, last):
    assert first.value < last.value


def non_assertion_chain(first, middle, last):
    if first.value < middle.value < last.value:
        consume()
    after()
