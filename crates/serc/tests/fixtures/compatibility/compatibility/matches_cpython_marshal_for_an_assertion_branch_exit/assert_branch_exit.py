def select(result, states, failed):
    if failed:
        assert states == {"disabled"}
    else:
        assert states == {"normal"}

    return result
