def f(exits, turn, enters):
    exits, turn, enters = [e for _, e in exits], turn[1], [e for _, e in enters]
    return exits, turn, enters
