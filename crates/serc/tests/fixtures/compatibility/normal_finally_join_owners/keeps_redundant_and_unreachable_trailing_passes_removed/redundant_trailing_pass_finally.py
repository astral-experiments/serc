def same_line(work, close):
    try:
        work(); pass
    finally:
        close()

def unreachable(close):
    try:
        return 1
        pass
    finally:
        close()
