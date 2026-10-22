def process(items, done):
    try:
        for item in items:
            try:
                if done:
                    break
                work(item)
            finally:
                value = 1
    except:
        recover()
    finish()
