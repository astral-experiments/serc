def choose(worker, files, position):
    if 0 <= position < len(files):
        return (worker, files[position:])
    else:
        None
