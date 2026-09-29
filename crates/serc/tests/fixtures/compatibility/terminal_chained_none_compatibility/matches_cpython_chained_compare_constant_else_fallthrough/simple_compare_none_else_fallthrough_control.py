def choose(worker, files, position):
    if position < len(files):
        return (worker, files[position:])
    else:
        None
